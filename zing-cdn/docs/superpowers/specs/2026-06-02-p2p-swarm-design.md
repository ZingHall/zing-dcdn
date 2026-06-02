# P2P Swarm Design

## Overview

Wire a real libp2p P2P swarm into `zing-cdn-core`, replacing the current `ZingP2pNode` stub. The swarm enables L1 blob resolution: peers discover blob providers via Kademlia DHT and transfer data over QUIC substreams.

## Protocol Stack

| Protocol | Purpose | Implementation |
|---|---|---|
| `/ipfs/kad/1.0.0` | Provider announce/lookup | `Kademlia<MemoryStore>` |
| `/zing-cdn/data/1.0` | Blob request + byte stream | Custom `NetworkBehaviour` |
| `/x/identify/1.0.0` | Peer identification | `Identify` |
| `/ping/1.0.0` | Keepalive + latency | `Ping` |

Transport: QUIC only (no TCP for MVP).

## Substream Framing (`/zing-cdn/data/1.0`)

All messages use **unsigned-varint length-prefix**, **cbor payload**:

```
Client → Server:
  [varint][BlobRequest (cbor)]    // {blob_id: [u8;32], version: u8}

Server → Client:
  [varint][BlobResponse (cbor)]   // Have{size: u64} | NotFound
  // If Have, stream follows immediately:
  [varint]payload_chunk...[varint=0]   // 64 KiB chunks, varint=0 = EOF
```

## Architecture

### `ZingBehaviour` (composed NetworkBehaviour)

```rust
#[derive(NetworkBehaviour)]
struct ZingBehaviour {
    kad: Kademlia<MemoryStore>,
    data: ZingDataProtocol,
    identify: Identify,
    ping: Ping,
}
```

### `ZingDataProtocol` (custom behaviour)

- Manages `/zing-cdn/data/1.0` substreams
- On inbound: read `BlobRequest`, look up `BlobStore`, respond `BlobResponse`, stream chunks if Have
- On outbound: open substream, send `BlobRequest`, read `BlobResponse`, reassemble chunks

### `ZingP2pNode` (async actor)

Owned by the CLI process, runs in a background tokio task:
- Holds `Swarm<ZingBehaviour>`
- Processes `SwarmEvent`s in a `select!` loop
- Accepts commands from `Resolver` via `mpsc` channel

```rust
enum P2pCommand {
    AnnounceBlob { blob_id: [u8; 32] },
    FindProviders { blob_id: [u8; 32], reply: oneshot::Sender<Vec<PeerId>> },
    FetchBlob { peer_id: PeerId, blob_id: [u8; 32], reply: oneshot::Sender<ZingResult<Vec<u8>>> },
    GetConnectedPeers { reply: oneshot::Sender<Vec<PeerId>> },
}
```

### Resolver Integration (L1)

Replace the stub in `Resolver::resolve()`:
1. Send `FindProviders { blob_id }` to swarm actor
2. Await `oneshot` with 5s timeout
3. If providers found, pick highest-reputation peer
4. Send `FetchBlob { peer_id, blob_id }` to swarm actor
5. Verify received bytes with `BlobVerifier`
6. Cache + return, record peer success/corruption
7. If no providers or timeout → fall through to L3 Walrus

## File Changes

| File | Change |
|---|---|
| `zing-cdn-core/src/p2p/node.rs` | Rewrite: `ZingP2pNode` becomes async actor with swarm + mpsc |
| `zing-cdn-core/src/p2p/behaviour.rs` | New: `ZingBehaviour` + `ZingDataProtocol` |
| `zing-cdn-core/src/p2p/protocol.rs` | Rewrite: varint+cbor codec, streaming frame types |
| `zing-cdn-core/src/p2p/handler.rs` | Rewrite: async handler using `BlobStoreHandle` |
| `zing-cdn-core/src/p2p/discovery.rs` | New: Kademlia bootstrap + provider record logic |
| `zing-cdn-core/src/p2p/mod.rs` | Update: re-exports |
| `zing-cdn-core/src/mesh/resolver.rs` | Wire L1: send `P2pCommand` instead of stub |
| `zing-cdn-core/src/client.rs` | Add `ZingClient::p2p_node()` + `p2p_command_tx()` accessors |

## Testing

All tests are **in-process** (no Docker). Create 2-3 `Swarm`s on `127.0.0.1` in a single test:

| Test | What it validates |
|---|---|
| `test_kad_bootstrap` | Two swarms connect and bootstrap Kademlia |
| `test_announce_and_find_provider` | Node A announces blob via `kad.start_providing`, Node B finds it via `kad.get_providers` |
| `test_blob_fetch_l1` | Full L1 flow: announce → find provider → fetch via `/zing-cdn/data/1.0` |
| `test_blob_fetch_fallback_l3` | No providers → resolver falls back to L3 (mocked) |
| `test_peer_reputation_tracking` | Failed verification decreases peer score |
