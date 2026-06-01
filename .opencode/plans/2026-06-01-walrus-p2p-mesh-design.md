# Zing: Walrus-Native P2P Content Distribution Mesh

**Date:** 2026-06-01
**Status:** Approved

## 1. System Overview

**Name:** Zing — a Walrus-native P2P content distribution mesh (not a BitTorrent client, but inspired by its principles).

**Purpose:** A Rust desktop application (Tauri + Dioxus) that aggregates content from the Walrus decentralized storage network and redistributes it peer-to-peer, creating a decentralized CDN layer.

**Core Concept Mapping:**

| BitTorrent Concept | Zing Equivalent |
|---|---|
| Info-hash | Blob ID (Merkle root commitment) |
| .torrent file | Article on-chain object (`Article` Sui struct) |
| Piece | Sliver (with Merkle proof) |
| Bitfield | Sliver availability map |
| Piece hash (SHA-1) | Merkle proof validation against Blob ID |
| Tracker | libp2p Kademlia DHT (peer discovery), Sui on-chain (metadata) |
| Choke/Unchoke | L1 stream availability / L3 fallback |
| Leech | L3 downloader |
| Seeder | L1 cached blob server |

**Three-Tier Data Resolution:**

```
Request Blob ID
      │
      ▼
┌──────────────┐    YES    ┌─────────────────────┐
│ Any L1 seeder │──────────▶│ Stream full blob    │
│ in DHT?      │           │ 1:1 QUIC connection │
└──────┬───────┘           └─────────────────────┘
       │ NO
       ▼
┌──────────────┐    YES    ┌─────────────────────┐
│ L3: Walrus   │──────────▶│ Fetch slivers from  │
│ epoch active?│           │ storage nodes,      │
└──────┬───────┘           │ reconstruct, cache,  │
       │ NO                │ promote to L1 seeder │
       ▼                   └─────────────────────┘
┌──────────────┐
│ 404: Blob    │
│ expired      │
└──────────────┘
```

**Technology Stack:**

- **Language:** Rust (entire stack including UI framework)
- **Desktop:** Tauri + Dioxus (desktop target only)
- **P2P:** libp2p (QUIC transport, Kademlia DHT, gossipsub)
- **Walrus:** Walrus Rust SDK (direct storage node communication)
- **Sui:** Sui Rust SDK (on-chain reads: Article objects, epoch data)
- **Storage:** RocksDB (local cache for blobs and metadata)

## 2. Component Architecture

```
zing/
├── zing-core/           # Core library (no UI dependency)
│   ├── walrus/          # L3: Walrus SDK integration
│   │   ├── client.rs       # Connect to storage nodes, fetch slivers
│   │   ├── reconstruct.rs  # Red Stuff erasure decoding
│   │   └── epoch.rs        # Epoch tracking, expiry detection
│   ├── p2p/             # L1: libp2p mesh layer
│   │   ├── node.rs         # libp2p Swarm setup (QUIC, Kademlia, gossipsub)
│   │   ├── protocol.rs     # Custom blob-stream protocol (/zing/blob/1.0)
│   │   └── discovery.rs    # Kademlia DHT for peer lookup by Blob ID
│   ├── cache/            # Local storage management
│   │   ├── store.rs        # RocksDB wrapper for blob storage
│   │   ├── pinning.rs      # Explicit pin/unpin operations
│   │   └── eviction.rs     # LRU eviction for unpinned blobs
│   ├── sui/              # Sui on-chain interaction
│   │   ├── client.rs       # Sui Rust SDK client
│   │   ├── article.rs      # Read Article objects, extract Blob IDs
│   │   └── epoch.rs        # Read epoch/committee data
│   └── mesh/             # Orchestration layer
│       ├── resolver.rs     # L1/L3 resolution logic
│       └── lifecycle.rs    # Blob cache state machine
├── zing-app/             # Tauri + Dioxus desktop app
│   ├── src/
│   │   ├── main.rs         # Tauri entry point
│   │   ├── app.rs          # Dioxus root component
│   │   └── views/          # UI pages
│   └── Cargo.toml
└── Cargo.toml            # Workspace root
```

**Key architectural decisions:**

- `zing-core` has no UI dependency — testable in isolation as a library
- `mesh/resolver.rs` is the brain: given a Blob ID, it checks L1 cache → queries DHT → falls back to L3
- `walrus/client.rs` handles L3 (direct storage node communication), L1 is entirely in `p2p/`
- `sui/` is read-only for the MVP: Article lookups and epoch checks. No on-chain writes.

## 3. Data Flow & Protocol

### 3.1 Blob Request Lifecycle (MVP: L1 + L3)

```
User requests Blob ID "X"
       │
       ▼
┌──────────────────┐
│ Check local cache │─────── HIT ──────▶ Stream from RocksDB
└──────┬───────────┘
       │ MISS
       ▼
┌──────────────────┐
│ Fetch metadata    │─────── Extract signed SHA-256
│ from Walrus       │         for verification
│ (tiny payload)    │
└──────┬───────────┘
       ▼
┌──────────────────┐      Found peers
│ Query Kademlia    │─────── via DHT ────────▶ Connect to seeder
│ DHT for Blob "X"  │                          Stream full blob
└──────┬───────────┘                          Verify vs SHA-256
       │ No seeders                             Cache locally
       ▼                                        Announce to DHT
┌──────────────────┐
│ Check Walrus epoch│────── Expired ──▶ 404: Blob unavailable
└──────┬───────────┘
       │ Active
       ▼
┌──────────────────┐
│ Fetch slivers from │
│ Walrus storage    │
│ nodes (f+1 quorum)│
│ Reconstruct blob  │
│ Verify vs metadata│
│ Cache locally     │
│ Announce to DHT   │
└──────────────────┘
```

### 3.2 Blob Verification

The Walrus Blob ID is derived from a Merkle root of the Red Stuff erasure-coded matrix, not a flat SHA-256 of the original file. Therefore, L1 stream verification uses **metadata pre-fetch**:

1. Before streaming from an L1 seeder, the client fetches the blob's metadata from Walrus (tiny payload)
2. The metadata contains a certified SHA-256 hash of the original blob content
3. The client streams the full blob from the L1 seeder
4. On completion, the client hashes the streamed blob and compares against the certified SHA-256 from metadata
5. Mismatch → discard data, flag peer in local reputation table, try next seeder

### 3.3 Custom libp2p Protocol `/zing/blob/1.0`

```
Handshake:
  → BlobRequest  { blob_id, version: 1 }
  ← BlobResponse { status: HAVE | NOT_FOUND,
                    size: u64 }

Data transfer (if status == HAVE):
  ← BlobStream   { chunk: bytes }
  ← BlobStream   { chunk: bytes }
  ← ...
  ← BlobComplete { }
```

Verification is client-side using the metadata pre-fetch SHA-256 (see 3.2).

### 3.4 Peer Discovery

All peer discovery uses libp2p Kademlia DHT. No on-chain SeededBlob contract.

- When a node caches a blob, it announces itself as a provider for that Blob ID in the DHT
- When a node needs a blob, it queries the DHT for providers of that Blob ID
- DHT naturally prunes offline peers (no stale entries problem)
- Zero gas cost for peer announcements

On-chain reads are limited to:
- `Article` objects: what blobs exist, their metadata
- Walrus epoch/committee data: which storage nodes are active

## 4. Sui On-Chain Interaction

### 4.1 On-Chain Reads (Sui Rust SDK)

```
Article Object (existing on-chain struct):
┌─────────────────────────────────┐
│ id: UID                         │
│ owner: address                  │  → Publisher address
│ deleted: bool                   │  → Soft-delete flag
│ created_at: u64                  │  → Timestamp
│ subscription_level: Option<u8>  │  → Content tier
│ blobs: vector<Blob>             │  → Blob IDs for content
│ files: VecMap<String, File>     │  → File name/path mapping
└─────────────────────────────────┘

Client reads:
1. Look up Article by ID → get Blob IDs
2. Check deleted flag → skip if true
3. Extract file metadata for display
```

```
Walrus Epoch Contract:
┌─────────────────────────────────┐
│ committee → Storage node list    │
│ epoch_start → Begin timestamp    │
│ epoch_end → Expiry timestamp    │
└─────────────────────────────────┘

Client reads:
1. Check epoch_end vs current time
2. If expired → blob is 404, remove from cache
3. If active → proceed with L3 fetch
```

### 4.2 On-Chain Writes

None for MVP. No SeededBlob contract. Peer discovery is DHT-only.

## 5. Local Storage & Cache

### 5.1 MVP Cache State Machine (L2 deferred)

```
         pin()
    ─────────────▶┌──────────┐
    │             │  PINNED   │
    │             │ (manual)  │
    │             └────┬─────┘
    │                  │ unpin()
    │                  ▼
    │             ┌──────────┐
    │             │  CACHED   │
    └─────────────│ FULL BLOB │───────▶ EVICTED (deleted)
                  │(L1 seeder)│  LRU
                  └──────────┘
```

| Cache State | Eviction Policy | Role in Swarm |
|---|---|---|
| Pinned Full Blob | Manual only | Permanent L1 seeder |
| Cached Full Blob | LRU → evicted | Opportunistic L1 seeder |

L2 sliver state is deferred to Milestone 3. In the MVP, when a cached blob hits LRU, it is fully evicted.

### 5.2 Database

- RocksDB for local blob storage and metadata
- User-configurable disk budget (e.g., 50GB default)
- Explicit pin/unpin operations
- LRU eviction for unpinned blobs within budget

## 6. Error Handling & Edge Cases

| Scenario | Handling |
|---|---|
| No seeders in DHT, epoch active | Fall back to L3 Walrus fetch |
| No seeders in DHT, epoch expired | Return 404 to user, evict from cache |
| L1 peer sends corrupted data | SHA-256 mismatch → discard, flag peer, try next seeder |
| All L1 seeders unresponsive | Fall back to L3 |
| L1 stream interrupted mid-transfer | Resume from DHT (find another seeder) or fall back to L3 |
| Peer claims to have blob but sends NOT_FOUND | DHT record stale → remove peer from DHT provider list |
| Disk full during cache write | Evict LRU unpinned blobs. If all pinned → error to user |
| Walrus storage nodes refuse slivers (no bounty) | Retry with different node subset. If quorum unreachable → error to user |
| Metadata pre-fetch fails | Cannot safely verify L1 stream → force L3 (safest fallback) |

### 6.1 Peer Reputation (Local-Only, No On-Chain)

```
┌────────────────────────────────────────────┐
│          Local Reputation Table             │
├────────────────────────────────────────────┤
│  Peer ID  │  Score  │  Last Seen           │
│  0xabc... │  +10    │  2026-06-01 10:00    │
│  0xdef... │  -5     │  2026-06-01 09:30    │
│                                             │
│  +1  → Successful blob stream               │
│  -3  → Corrupted data (SHA-256 mismatch)   │
│  -1  → Connection dropped mid-transfer       │
│  -5  → Claimed HAVE but sent NOT_FOUND      │
│                                             │
│  Score < -10 → Blacklist peer locally       │
│  Score decays toward 0 over time             │
└────────────────────────────────────────────┘
```

## 7. Milestone Roadmap

### Milestone 1 — MVP (L3 + L1)

| Component | Scope |
|---|---|
| `zing-core/walrus/` | Connect to storage nodes, fetch slivers, Red Stuff reconstruction, metadata fetch for SHA-256 verification |
| `zing-core/sui/` | Read Article objects, read epoch/committee data. No on-chain writes |
| `zing-core/p2p/` | libp2p node (QUIC + Kademlia + gossipsub), custom `/zing/blob/1.0` protocol for full blob streaming, DHT provider announcements |
| `zing-core/cache/` | RocksDB store, explicit pin/unpin, LRU eviction (pinned → cached → evicted, no L2 sliver state) |
| `zing-core/mesh/` | Resolver: L1 cache → metadata pre-fetch → DHT lookup → L3 fallback. Lifecycle state machine. Peer reputation table |
| `zing-app/` | Minimal Tauri + Dioxus desktop UI: search Article by ID, view files, download/seed blobs, pin management, status display |

### Milestone 2 — Fee Layer

- Sui smart contracts for micropayments
- API keys, conditional content unlocks
- On-chain writes for bounty/reward distribution

### Milestone 3 — L2 Sliver Swarm

- Bitfield gossiping (sliver-level availability)
- Sliver trading protocol
- Edge-based Merkle verification for individual slivers
- Cache state machine update: cached full blob → slivers → evicted

### Milestone 4 — Bounties

- Optional on-chain bounties attached to Articles
- Incentive-driven seeding for rare content
- Voluntary seeding by default, bounties for content at risk of expiry

## 8. MVP Exclusions (YAGNI for Milestone 1)

- No sliver-level P2P trading (L2)
- No on-chain writes (no SeededBlob, no bounty contracts)
- No payment/fee integration
- No subscription/gated content
- No multi-blob concatenation (`/v1alpha/blobs/concat`) — single blob per Article only
- Desktop target only (no mobile)
- No L2 sliver cache state (cached blobs evict directly to deleted)