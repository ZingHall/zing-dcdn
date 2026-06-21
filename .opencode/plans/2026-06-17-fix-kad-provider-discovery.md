# Fix Kad DHT Provider Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix Kad DHT provider discovery so `start_providing` actually propagates provider records to other peers, enabling DHT-based blob discovery.

**Architecture:** The root cause is that `start_providing` sends zero ADD_PROVIDER RPCs when kbuckets are empty (the query completes "successfully" with no peers contacted). The fix has three parts: (1) handle `StartProviding` event results for visibility, (2) retry announce when no peers are connected so it fires after connections stabilize, (3) ensure kbuckets are populated before announcing by deferring bootstrap until connections are up.

**Tech Stack:** Rust, libp2p 0.54, tokio, existing ZingBehaviour/Kad/MemoryStore

---

### Task 1: Handle Kad StartProviding and other query results in node.rs

**Files:**
- Modify: `zing-cdn/zing-cdn-core/src/p2p/node.rs:391-417`

Currently the `OutboundQueryProgressed` handler only processes `GetProviders` — all other Kad query results (including `StartProviding`, `Bootstrap`) are silently discarded at line 414 (`_ => {}`). This makes it impossible to know whether `start_providing` succeeded or failed.

- [ ] **Step 1: Add handlers for StartProviding and Bootstrap results**

In `handle_behaviour_event`, expand the `OutboundQueryProgressed` match arm to handle `StartProviding` and `Bootstrap` results.

Replace the existing match arm at lines 391-417 with:

```rust
kad::Event::OutboundQueryProgressed { id, result, .. } => {
    match result {
        kad::QueryResult::GetProviders(Ok(ok)) => {
            match ok {
                kad::GetProvidersOk::FoundProviders { providers, .. } => {
                    if let Some(sender) = pending_finds.remove(&id) {
                        let peers: Vec<PeerId> = providers.into_iter().collect();
                        let _ = sender.send(peers);
                    }
                }
                kad::GetProvidersOk::FinishedWithNoAdditionalRecord { .. } => {
                    if let Some(sender) = pending_finds.remove(&id) {
                        let _ = sender.send(vec![]);
                    }
                }
            }
        }
        kad::QueryResult::GetProviders(Err(e)) => {
            tracing::warn!(?id, %e, "get_providers query failed");
            if let Some(sender) = pending_finds.remove(&id) {
                let _ = sender.send(vec![]);
            }
        }
        kad::QueryResult::StartProviding(Ok(ok)) => {
            tracing::info!(key = %ok.key, "Kad start_providing succeeded: provider record published");
        }
        kad::QueryResult::StartProviding(Err(e)) => {
            tracing::warn!(?id, %e, "Kad start_providing query failed");
        }
        kad::QueryResult::Bootstrap(Ok(ok)) => {
            tracing::info!(?id, remaining = ok.num_remaining, "Kad bootstrap progress");
        }
        kad::QueryResult::Bootstrap(Err(e)) => {
            tracing::warn!(?id, %e, "Kad bootstrap query failed");
        }
        _ => {
            tracing::debug!(?id, result = ?result, "Kad query progressed (unhandled)");
        }
    }
}
```

- [ ] **Step 2: Run tests to verify compilation**

Run: `cd zing-cdn && cargo test -p zing-cdn-core --lib --tests 2>&1 | tail -20`
Expected: All existing tests pass.

- [ ] **Step 3: Commit**

```bash
git add zing-cdn/zing-cdn-core/src/p2p/node.rs
git commit -m "feat: handle Kad StartProviding and Bootstrap query results"
```

---

### Task 2: Delay bootstrap until connections are established

**Files:**
- Modify: `zing-cdn/zing-cdn-core/src/p2p/node.rs:122-124` (remove immediate bootstrap)
- Modify: `zing-cdn/zing-cdn-core/src/p2p/node.rs:352-354` (trigger bootstrap on ConnectionEstablished)

Currently `kad.bootstrap()` is called immediately after `dial()` in `run()`, before the connection is established. This always fails with "No known peers" because the connection isn't up yet. Bootstrap should instead be triggered when a connection is established with a bootstrap peer.

- [ ] **Step 1: Track bootstrap peer IDs in run() state**

In `ZingP2pNode::run()`, add these after the `peer_addresses` HashMap (around line 143):

```rust
let mut bootstrap_peers: std::collections::HashSet<PeerId> = bootstrap_addrs.iter().map(|(pid, _)| *pid).collect();
let mut bootstrap_done = false;
```

- [ ] **Step 2: Remove immediate bootstrap() call**

Delete lines 122-124 in `run()`:

```rust
if let Err(e) = swarm.behaviour_mut().kad.bootstrap() {
    tracing::warn!(error = %e, "kad bootstrap");
}
```

- [ ] **Step 3: Trigger bootstrap on ConnectionEstablished with bootstrap peer**

Update `handle_swarm_event` signature to accept the new parameters and modify the `ConnectionEstablished` handler. The full signature becomes:

```rust
async fn handle_swarm_event(
    swarm: &mut Swarm<ZingBehaviour>,
    event: SwarmEvent<ZingBehaviourEvent, void::Void>,
    _pending_finds: &mut HashMap<kad::QueryId, oneshot::Sender<Vec<PeerId>>>,
    _pending_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
    _pending_range_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
    _pending_sliver_fetches: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<ZingResult<Vec<u8>>>>,
    _pending_addr_queries: &mut HashMap<request_response::OutboundRequestId, oneshot::Sender<Vec<Multiaddr>>>,
    _peer_addresses: &mut HashMap<PeerId, Vec<Multiaddr>>,
    bootstrap_peers: &mut std::collections::HashSet<PeerId>,
    bootstrap_done: &mut bool,
    _store: &BlobStoreHandle,
)
```

Replace the `ConnectionEstablished` handler:

```rust
SwarmEvent::ConnectionEstablished { peer_id, .. } => {
    tracing::info!(%peer_id, "P2P connection established");
    if !*bootstrap_done && bootstrap_peers.take(&peer_id).is_some() {
        match swarm.behaviour_mut().kad.bootstrap() {
            Ok(_) => {
                tracing::info!("kad bootstrap initiated after connection to bootstrap peer {}", peer_id);
                *bootstrap_done = true;
            }
            Err(e) => {
                tracing::warn!(error = %e, "kad bootstrap failed after connection to bootstrap peer {}", peer_id);
            }
        }
    }
}
```

Update the call site in `run()` to pass `&mut bootstrap_peers` and `&mut bootstrap_done`.

- [ ] **Step 4: Run tests to verify compilation**

Run: `cd zing-cdn && cargo test -p zing-cdn-core --lib --tests 2>&1 | tail -20`
Expected: All existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add zing-cdn/zing-cdn-core/src/p2p/node.rs
git commit -m "fix: defer Kad bootstrap until connection with bootstrap peer is established"
```

---

### Task 3: AnnounceBlob queues retry when no peers connected

**Files:**
- Modify: `zing-cdn/zing-cdn-core/src/p2p/node.rs` (AnnounceBlob handler + ConnectionEstablished handler)

When `start_providing` is called and zero peers are connected, the Kad query finds no peers and the provider record is never published. This task adds a retry: if no peers are connected at announce time, queue the blob_id and re-announce when a connection is established.

- [ ] **Step 1: Add pending_announces tracking in run()**

After the `bootstrap_done` variable (added in Task 2), add:

```rust
let mut pending_announces: Vec<[u8; 32]> = Vec::new();
```

- [ ] **Step 2: Modify AnnounceBlob handler to queue retries**

Change the `AnnounceBlob` handler from:

```rust
P2pCommand::AnnounceBlob { blob_id } => {
    let key = kad::RecordKey::new(&blob_id);
    if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
        tracing::warn!(error = %e, "Kad start_providing failed");
    }
}
```

To:

```rust
P2pCommand::AnnounceBlob { blob_id } => {
    let key = kad::RecordKey::new(&blob_id);
    if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
        tracing::warn!(error = %e, "Kad start_providing failed");
    }
    let connected_count = swarm.connected_peers().count();
    if connected_count == 0 {
        tracing::info!(blob_id = %hex::encode(blob_id), "No connected peers, queueing announce for retry");
        pending_announces.push(blob_id);
    }
}
```

Add `pending_announces: &mut Vec<[u8; 32]>` to `handle_command`'s signature and pass it from `run()`.

- [ ] **Step 3: Re-announce queued blobs on ConnectionEstablished**

In the `ConnectionEstablished` handler (modified in Task 2), after the bootstrap logic, add:

```rust
if !pending_announces.is_empty() {
    let announces: Vec<[u8; 32]> = pending_announces.drain(..).collect();
    for bid in announces {
        let key = kad::RecordKey::new(&bid);
        if let Err(e) = swarm.behaviour_mut().kad.start_providing(key) {
            tracing::warn!(error = %e, "Kad start_providing retry failed");
        } else {
            tracing::info!(blob_id = %hex::encode(bid), "Kad start_providing retry sent");
        }
    }
}
```

Add `pending_announces: &mut Vec<[u8; 32]>` to `handle_swarm_event`'s signature and pass it from `run()`.

Check that `hex` crate is available (it should be — used elsewhere in the project). If not, add it to `zing-cdn-core/Cargo.toml`.

- [ ] **Step 4: Run tests**

Run: `cd zing-cdn && cargo test -p zing-cdn-core --lib --tests 2>&1 | tail -20`
Expected: All existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add zing-cdn/zing-cdn-core/src/p2p/node.rs
git commit -m "feat: retry AnnounceBlob when no peers connected, re-announce on connection"
```

---

### Task 4: Add Kad DHT provider discovery integration test

**Files:**
- Create: `zing-cdn/zing-cdn-core/tests/kad_provider_test.rs`

This test verifies that `start_providing` on Node A results in `get_providers` finding Node A on Node B, after both are connected via Node A as bootstrap.

- [ ] **Step 1: Write the integration test**

Create `zing-cdn/zing-cdn-core/tests/kad_provider_test.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use libp2p::Multiaddr;
use libp2p::identity;
use tokio::sync::oneshot;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::p2p::node::ZingP2pNode;
use zing_cdn_core::p2p::P2pCommand;

fn create_keypair() -> identity::Keypair {
    identity::Keypair::generate_ed25519()
}

fn create_store() -> Arc<RwLock<BlobStore>> {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = BlobStore::open(dir.path()).expect("open store");
    Arc::new(RwLock::new(store))
}

#[tokio::test]
async fn test_kad_start_providing_and_get_providers() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("zing_cdn_core=debug"))
        .with_writer(std::io::stderr)
        .try_init();

    // Node A: bootstrap node + announcer
    let store_a = create_store();
    let (node_a, rx_a) = ZingP2pNode::new(store_a.clone(), create_keypair());
    let key_a = node_a.key().clone();
    let tx_a = node_a.command_tx().clone();
    let peer_a = node_a.local_peer_id();
    let listen_a: Multiaddr = "/ip4/127.0.0.1/udp/19101/quic-v1".parse().unwrap();

    let store_a_clone = store_a.clone();
    let join_a = tokio::spawn(async move {
        let _ = ZingP2pNode::run(key_a, rx_a, store_a_clone, listen_a.clone(), vec![]).await;
    });

    // Wait for Node A to start listening
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Node B: connects to A, queries providers
    let store_b = create_store();
    let (node_b, rx_b) = ZingP2pNode::new(store_b.clone(), create_keypair());
    let key_b = node_b.key().clone();
    let tx_b = node_b.command_tx().clone();
    let listen_b: Multiaddr = "/ip4/127.0.0.1/udp/19102/quic-v1".parse().unwrap();

    let store_b_clone = store_b.clone();
    let join_b = tokio::spawn(async move {
        let _ = ZingP2pNode::run(
            key_b,
            rx_b,
            store_b_clone,
            listen_b,
            vec![(peer_a, listen_a.clone())],
        ).await;
    });

    // Wait for Node B to connect, bootstrap, and Kad to stabilize
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Node A announces a blob
    let blob_id = [42u8; 32];
    tx_a.send(P2pCommand::AnnounceBlob { blob_id }).await.expect("announce");

    // Wait for provider record to propagate via ADD_PROVIDER
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Node B queries for providers
    let (reply, rx) = oneshot::channel();
    tx_b.send(P2pCommand::FindProviders { blob_id, reply }).await.expect("find providers");

    let providers = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("timeout waiting for find_providers")
        .expect("oneshot channel");

    eprintln!("Providers found: {:?}", providers);
    assert!(
        providers.contains(&peer_a),
        "Node B should find Node A as a provider. Got: {:?}",
        providers
    );

    join_a.abort();
    join_b.abort();
    let _ = join_a.await;
    let _ = join_b.await;
}
```

- [ ] **Step 2: Run the test**

Run: `cd zing-cdn && cargo test -p zing-cdn-core --test kad_provider_test -- --nocapture 2>&1 | tail -30`

Expected: With the fixes from Tasks 1-3, the test should pass with providers containing `peer_a`.

If the test fails, check debug logs for:
- `Kad start_providing succeeded: provider record published` on Node A
- `kad bootstrap initiated after connection` on Node B
- `Kad providers` log showing non-empty results

Increase wait times (1-2s) if the test is timing-dependent.

- [ ] **Step 3: Commit**

```bash
git add zing-cdn/zing-cdn-core/tests/kad_provider_test.rs
git commit -m "test: add Kad DHT provider discovery integration test"
```

---

### Task 5: End-to-end verification

**Files:**
- No new files — manual verification

- [ ] **Step 1: Run all existing tests**

Run: `cd zing-cdn && cargo test -p zing-cdn-core --lib --tests 2>&1 | tail -30`
Expected: All tests pass including the new `test_kad_start_providing_and_get_providers`.

- [ ] **Step 2: Build and deploy to Fly**

Rebuild and redeploy Fly. Verify logs show:
- `kad bootstrap initiated after connection to bootstrap peer` (instead of immediate "kad bootstrap" failure)
- `Kad start_providing succeeded: provider record published` when peers announce
- `Kad bootstrap progress` when bootstrap queries succeed

- [ ] **Step 3: Test full P2P flow with Fly + 2 local peers**

1. Start Peer A and Peer C with Fly as bootstrap
2. Peer A fetches blob from Walrus → AnnounceBlob
3. Verify: `Kad start_providing succeeded` in Peer A logs
4. Peer C resolves same blob → DHT path finds Peer A
5. Verify: `dht_peers=[<Peer A>]` in Peer C logs
6. Peer C fetches blob from Peer A via L1

Expected: Full L1 P2P transfer works without L3 fallback.