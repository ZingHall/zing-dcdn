# WAL Read-Fee Economic Layer — MVP Implementation Plan

## Goal
Peers pay WAL tokens as read fees to serving peers via on-chain Sui transactions. Payment is mandatory — a peer refuses to serve blobs without a valid payment proof. Peers without a Sui wallet skip L1 and fall back to L3 (Walrus direct).

## Design Decisions
- **Payment**: On-chain Sui transactions (real WAL token transfers)
- **Fee**: Fixed fee per fetch (configurable, default 0.001 WAL = 1_000_000 NANOS)
- **Address discovery**: Kad DHT `put_record` / `get_record` (key = `b"zing-sui-addr" + peer_id_bytes`)
- **Wallet**: Sui CLI keystore at `~/.sui/sui_config/`
- **No-wallet behavior**: Skip L1, use L3 Walrus fallback
- **Verification**: Spot-check every 10th L1 fetch (metadata pre-fetch from Walrus committee)
- **Payment enforcement**: Mandatory — peer returns NOT_FOUND if no payment proof in request

## Architecture Flow
```
Peer C wants blob → L0 cache miss → L1 P2P path:
  1. GetProviders → find Peer A
  2. kad.get_record(peer_id_key) → Peer A's Sui address
  3. wallet.pay_wal(peer_a_sui_addr, FEE) → submit Sui tx → get tx_digest
  4. FetchBlob { blob_id, payment_tx_digest: tx_digest }
  5. Peer A checks payment_tx_digest present → serves blob
  6. Peer C hash-verifies blob (always, cheap)
  7. Every 10th fetch: fetch_metadata + verify_blob_against_metadata
If no wallet/WAL → skip L1 → L3 Walrus (free)
```

## Components

### Component 1: Sui Wallet Integration
**New file**: `zing-cdn-core/src/sui/wallet.rs`

```rust
pub struct ZingWallet {
    client: SuiContractClient,
    address: SuiAddress,
}

impl ZingWallet {
    /// Load from Sui CLI keystore (~/.sui/sui_config/)
    pub async fn from_keystore(keystore_path: &Path, contract_config: &ContractConfig, rpc_urls: &[String]) -> ZingResult<Self>;
    
    /// Get this wallet's Sui address
    pub fn address(&self) -> SuiAddress;
    
    /// Pay WAL tokens to a recipient. Returns the transaction digest (32 bytes).
    pub async fn pay_wal(&self, recipient: SuiAddress, amount: u64) -> ZingResult<[u8; 32]>;
    
    /// Check WAL balance
    pub async fn wal_balance(&self) -> ZingResult<u64>;
}
```

**Implementation** (using public walrus-sui APIs):
```rust
use walrus_sui::client::SuiContractClient;
use walrus_sui::wallet::Wallet;
use walrus_sui::config::load_wallet_context_from_path;

// 1. Load wallet
let ctx = load_wallet_context_from_path(&sui_config_dir)?;
let wallet = Wallet::new(ctx);
let address = wallet.active_address();

// 2. Create SuiContractClient
let client = SuiContractClient::new(
    wallet, rpc_urls, &contract_config, 
    backoff_config, None,  // gas_budget = auto
    Duration::from_secs(30)  // checkpoint_wait_timeout
).await?;

// 3. Pay WAL
let mut ptb = client.transaction_builder();
ptb.pay_wal(recipient, amount).await?;
let tx_data = ptb.build_transaction_data(None).await?;
let resp = client.sign_and_send_transaction(tx_data, "pay_wal").await?;
let digest = resp.transaction_digest;  // [u8; 32]
```

**Modify**: `zing-cdn-core/src/sui/mod.rs` — re-export `ZingWallet`
**Modify**: `zing-cdn-core/src/sui/client.rs` — add `wallet: Option<ZingWallet>` to `SuiClient`
**Modify**: `zing-cdn-core/src/client.rs` — `ZingClient::from_mainnet()` loads wallet if keystore exists

### Component 2: Peer Sui Address via Kad DHT
**Modify**: `zing-cdn-core/src/p2p/node.rs`

On startup (after swarm setup, if wallet is configured):
```rust
if let Some(wallet) = &wallet {
    let sui_addr_bytes = wallet.address().to_bytes();  // Vec<u8>
    let key = kad::RecordKey::new(&sui_addr_key(&local_peer_id));
    let record = kad::Record::new(key, sui_addr_bytes);
    match swarm.behaviour_mut().kad.put_record(record, Quorum::majority()) {
        Ok(_) => tracing::info!("Published Sui address to Kad DHT"),
        Err(e) => tracing::warn!(error = %e, "Failed to publish Sui address"),
    }
}
```

Key helper:
```rust
fn sui_addr_key(peer_id: &PeerId) -> Vec<u8> {
    let mut key = b"zing-sui-addr".to_vec();
    key.extend_from_slice(&peer_id.to_bytes());
    key
}
```

New P2P command:
```rust
P2pCommand::GetPeerSuiAddress {
    peer_id: PeerId,
    reply: oneshot::Sender<Option<SuiAddress>>,
}
```

Handler:
```rust
P2pCommand::GetPeerSuiAddress { peer_id, reply } => {
    let key = kad::RecordKey::new(&sui_addr_key(&peer_id));
    let query_id = swarm.behaviour_mut().kad.get_record(key);
    pending_sui_addr_queries.insert(query_id, reply);
}
```

Kad event handler:
```rust
kad::QueryResult::GetRecord(Ok(ok)) => {
    if let Some(record) = ok.records.first() {
        if let Some(sender) = pending_sui_addr_queries.remove(&id) {
            let addr = SuiAddress::from_bytes(&record.record.value);
            let _ = sender.send(Some(addr));
        }
    }
}
```

**Note**: `run()` needs an optional `wallet: Option<ZingWallet>` parameter.

### Component 3: Payment Proof in P2P Protocol
**Modify**: `zing-cdn-core/src/p2p/protocol.rs`

```rust
pub struct BlobRequest {
    pub blob_id: [u8; 32],
    pub version: u8,
    pub payment_tx_digest: [u8; 32],  // NEW — zeroed if no payment
}
```

Wire format change:
- Old: `blob_id (32 bytes) + version (1 byte) = 33 bytes`
- New: `blob_id (32 bytes) + version (1 byte) + payment_tx_digest (32 bytes) = 65 bytes`

Update `BinaryProtocolCodec::read_request`:
```rust
let len = read_u32_le(io).await? as usize;
if len != 65 { return Err(...); }
let mut buf = [0u8; 65];
io.read_exact(&mut buf).await?;
let mut blob_id = [0u8; 32];
blob_id.copy_from_slice(&buf[..32]);
let version = buf[32];
let mut payment_tx_digest = [0u8; 32];
payment_tx_digest.copy_from_slice(&buf[33..65]);
Ok(BlobRequest { blob_id, version, payment_tx_digest })
```

Update `write_request` similarly (serialize 65 bytes).

**Also update**: `RangeRequest` to include `payment_tx_digest: [u8; 32]` (48 + 32 = 80 bytes).

### Component 4: Payment Logic in Resolver
**Modify**: `zing-cdn-core/src/mesh/resolver.rs`

Add to `Resolver`:
```rust
wallet: Option<Arc<ZingWallet>>,
p2p_tx: Option<mpsc::Sender<P2pCommand>>,
fetch_counter: u32,  // for spot-check verification
```

Before `FetchBlob` in `resolve_from_l1`:
```rust
// 1. Check if wallet is available
let wallet = match &self.wallet {
    Some(w) => w.clone(),
    None => {
        tracing::info!("No Sui wallet, skipping L1 (payment required)");
        return None;  // fall through to L3
    }
};

// 2. Get peer's Sui address from Kad DHT
let sui_addr = self.get_peer_sui_address(peer, tx).await?;
let sui_addr = match sui_addr {
    Some(addr) => addr,
    None => {
        tracing::warn!(peer = %peer, "Peer has no Sui address in DHT, skipping L1");
        return None;
    }
};

// 3. Pay WAL
let tx_digest = match wallet.pay_wal(sui_addr, READ_FEE_WAL).await {
    Ok(digest) => digest,
    Err(e) => {
        tracing::warn!(error = %e, "WAL payment failed, skipping L1");
        return None;
    }
};

// 4. Fetch with payment proof
tx.send(P2pCommand::FetchBlob {
    peer_id: peer,
    blob_id: blob_id.0,
    payment_tx_digest: tx_digest,
    reply: fetch_reply,
}).await
```

New method:
```rust
async fn get_peer_sui_address(&self, peer: PeerId, tx: &mpsc::Sender<P2pCommand>) -> Option<SuiAddress> {
    let (reply, rx) = oneshot::channel();
    tx.send(P2pCommand::GetPeerSuiAddress { peer_id: peer, reply }).await.ok()?;
    tokio::time::timeout(Duration::from_secs(5), rx).await.ok()??.ok()?
}
```

### Component 5: Peer-side Payment Verification
**Modify**: `zing-cdn-core/src/p2p/handler.rs`

In `handle_inbound_request`:
```rust
pub async fn handle_inbound_request(store: &BlobStoreHandle, request: BlobRequest) -> ZingResponse {
    // Check payment proof
    if request.payment_tx_digest == [0u8; 32] {
        tracing::info!(blob_id = %BlobId(request.blob_id), "refusing request: no payment proof");
        return ZingResponse::NotFound;
    }
    
    // For MVP: trust the digest (don't verify on-chain yet)
    // TODO: verify tx digest on Sui (check amount, recipient, confirmation)
    
    // Serve the blob
    let blob_id_str = BlobId(request.blob_id).to_string();
    // ... existing logic ...
}
```

**Modify**: `zing-cdn-core/src/p2p/node.rs` — `FetchBlob` handler passes `payment_tx_digest` to `BlobRequest`:
```rust
P2pCommand::FetchBlob { peer_id, blob_id, payment_tx_digest, reply } => {
    let request = BlobRequest { blob_id, version: 0, payment_tx_digest };
    let request_id = swarm.behaviour_mut().data.send_request(&peer_id, request);
    pending_fetches.insert(request_id, reply);
}
```

### Component 6: Spot-check Trustless Verification
**Modify**: `zing-cdn-core/src/mesh/resolver.rs`

In `finalize_l1_fetch`:
```rust
async fn finalize_l1_fetch(&mut self, blob_id, blob_id_hex, peer, data) -> Option<...> {
    // Always: hash verification (cheap)
    self.verifier.verify_blob_by_id(blob_id, &data)?;
    
    // Spot-check: every 10th fetch, verify against Walrus committee metadata
    self.fetch_counter += 1;
    if self.fetch_counter % SPOT_CHECK_INTERVAL == 0 {
        tracing::info!(blob_id = %blob_id_hex, "Spot-check: fetching metadata from Walrus committee");
        if let Ok(metadata) = self.walrus_client.fetch_metadata(blob_id).await {
            match self.verifier.verify_blob_against_metadata(&metadata, &data) {
                Ok(()) => tracing::info!("Spot-check passed"),
                Err(e) => {
                    tracing::warn!(error = %e, "Spot-check FAILED — peer served tampered data!");
                    self.reputation.record_corruption(&peer);
                    return None;
                }
            }
        }
    }
    
    // ... cache + announce ...
}
```

### Component 7: Wiring
**Modify**: `zing-cdn/src/main.rs`
- Add `--sui-keystore` flag (default: `~/.sui/sui_config/`)
- Add `--read-fee` flag (default: 1_000_000 NANOS = 0.001 WAL)
- Pass wallet to `ZingClient::from_mainnet()` and `Resolver`

**Modify**: `zing-cdn-gui/src-tauri/src/main.rs`
- Load Sui wallet from `~/.sui/sui_config/` if exists
- Pass to `Resolver`
- Add `ZING_SUI_KEYSTORE` env var support

**Modify**: `zing-cdn-core/src/client.rs`
- `ZingClient` gains `wallet: Option<ZingWallet>`
- `from_mainnet()` tries to load wallet from keystore

**Modify**: `zing-cdn-core/src/p2p/node.rs`
- `run()` gains `wallet: Option<ZingWallet>` parameter
- Publishes Sui address to Kad DHT on startup

### Constants
```rust
const READ_FEE_WAL: u64 = 1_000_000;  // 0.001 WAL in NANOS
const SPOT_CHECK_INTERVAL: u32 = 10;  // verify every 10th L1 fetch
const SUI_ADDR_KEY_PREFIX: &[u8] = b"zing-sui-addr";
```

## Implementation Order
1. **Component 1** (Sui wallet) — foundational, everything depends on it
2. **Component 3** (P2P protocol change) — update BlobRequest/RangeRequest + codec
3. **Component 5** (Peer-side verification) — refuse without payment proof
4. **Component 2** (Kad DHT records) — Sui address publish/discover
5. **Component 4** (Payment logic in resolver) — ties it all together
6. **Component 6** (Spot-check verification) — last, adds trustless verification
7. **Component 7** (Wiring) — CLI flags, GUI env var

## Testing
1. Unit test: `ZingWallet::pay_wal` on testnet
2. Integration test: two local nodes, one pays WAL, other serves
3. E2e test: Peer A + Peer C + Fly, verify payment flow

## Risk/Complexity Notes
- **Sui transaction latency**: ~1-2s per transaction. L1 fetches will be slower by this amount.
- **Gas costs**: Each WAL transfer requires SUI for gas. Peers need both SUI and WAL.
- **SuiContractClient mutex**: Only one transaction at a time per client. Payment is serialized.
- **Backward incompatibility**: Protocol change (BlobRequest wire format) requires all peers to upgrade.
- **MVP trust model**: Peer trusts tx_digest without on-chain verification. Future: verify on Sui.
- **Fly bootstrap**: Fly needs a wallet too (for receiving fees when it serves blobs). Or Fly could be exempt from payment (free bootstrap).
