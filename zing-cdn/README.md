# zing-cdn

Walrus-native P2P content distribution mesh. Fetches blobs from Walrus storage nodes and serves them via local cache with P2P peer distribution (WIP).

## Commands

### `zing-cdn get <blob-id>`

Fetch a blob from Walrus mainnet and cache it locally in RocksDB.

```
$ zing-cdn get jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Blob:    jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Size:    1.02 KiB (1047 bytes)
Source:  L3 Walrus
Cached:  now cached
```

Second call hits local cache instantly:

```
$ zing-cdn get jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Blob:    jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Size:    1.02 KiB (1047 bytes)
Source:  L0 local cache
Cached:  yes (at /Users/me/.zing-cdn/cache)
```

### `zing-cdn cat <blob-id>`

Write raw blob bytes to stdout. Useful for piping to a file.

```
$ zing-cdn cat <blob-id> > output.bin
```

Tracing output goes to stderr; only raw bytes go to stdout, so pipes and redirects work cleanly.

### `zing-cdn metadata <blob-id>`

Fetch and display blob metadata from Walrus (no caching, direct network query).

```
$ zing-cdn metadata jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Blob ID:         jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Unencoded:       1047 bytes
Encoding Type:    RS2
```

Shows the unencoded length (actual blob size before Walrus erasure coding) and the encoding type (e.g., RS2 for RedStuff/Reed-Solomon).

### `zing-cdn status <blob-id>`

Check a blob's status on Walrus (permanent, deletable, invalid, or nonexistent).

```
$ zing-cdn status jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Blob ID:    jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Status:     deletable
Certified:  epoch 31
```

Useful for checking whether a blob still exists on the network before trying to fetch it.

### `zing-cdn verify <blob-id>`

Read the blob from Walrus, fetch its metadata, and cryptographically verify that the blob data matches the expected Blake2b-256 blob ID. This is the same verification Walrus uses internally (`EncodingFactory::compute_blob_id()`).

```
$ zing-cdn verify jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Verification: ✅ PASSED
  Computed: jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
  Expected: jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
```

### `zing-cdn list`

List all blobs currently cached in the local RocksDB store.

```
$ zing-cdn list
Cached Blobs:
  jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q  1.02 KiB  pinned: no
```

### `zing-cdn pin <blob-id>`

Prevent a cached blob from being evicted (pinned blobs are skipped by LRU eviction).

```
$ zing-cdn pin jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Blob jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q pinned
```

### `zing-cdn unpin <blob-id>`

Allow a pinned blob to be evicted by the LRU cache eviction policy.

```
$ zing-cdn unpin jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Blob jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q unpinned
```

### `zing-cdn info <blob-id>`

Show details about a cached blob (state, pin status, size).

```
$ zing-cdn info jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
Blob ID:  jiuehgokj6HWjr6NbgVcg119r8ZFSFREzwNnHnh4h9Q
State:    Cached
Pinned:   no
Size:     1.02 KiB (1047 bytes)
```

## Global Options

| Flag | Description |
|------|-------------|
| `--cache-dir <path>` | Override cache directory (default: `~/.zing-cdn/cache`) |
| `-v`, `--verbose` | Enable debug-level logging |

## Examples

```bash
# Quick status check
zing-cdn status <blob-id>

# Download and cache
zing-cdn get <blob-id>

# Save to file
zing-cdn cat <blob-id> > document.bin

# Verify integrity
zing-cdn verify <blob-id>

# Pin important blobs
zing-cdn get <blob-id> && zing-cdn pin <blob-id>
```

## Architecture

- **zing-cdn-core** — Library crate with Walrus client, blob cache (RocksDB), L3/L1 resolution, P2P protocol types
- **zing-cdn** — CLI binary with the commands above
- Cache location: `~/.zing-cdn/cache/` (500 MB LRU budget)
- Backend: Walrus mainnet via `https://fullnode.mainnet.sui.io:443`

## Milestones

### Phase 1 — Blob-level P2P CDN ✅

L0 RocksDB cache with LRU eviction and pinning. L1 P2P blob transfer via
`/zing-cdn/data/2.0` (binary length-prefixed framing). L3 Walrus mainnet
fallback. Kademlia DHT for provider records (`/zing-cdn/kad/1.0.0`).
Tauri v2 + Dioxus 0.7 desktop app with Dashboard, Blob Browser, Cache, and
Settings tabs. HTTP API (axum on localhost) for frontend-backend IPC with
SSE streaming resolve. Multi-instance dev via env vars.

### Phase 2 — Bootstrapped P2P mesh 🚧

Hardcoded bootstrap nodes for zero-config peer discovery. Kademlia DHT
auto-discovery (connect to one seed → find all peers). Auto-dial bootstrap
peers at startup. Tracing subscriber with file-based logging for the GUI.
Atomic keypair file creation (O_EXCL, no race conditions between instances).

### Phase 3 — Sliver-level P2P (planned)

Sliver-based request/response protocol replacing whole-blob transfer.
K-of-N reconstruction from slivers fetched from different peers. Sliver-aware
caching (nodes can cache individual slivers, not entire blobs). Parallel
sliver fetch from multiple peers simultaneously.

### Phase 4 — Economic layer (planned)

On-chain peer registration and staking on Sui. Proof-of-retrievability for
sliver storage verification. WAL-based payment for bandwidth and storage
contributions. Smart contract reward distribution proportional to verified
contributions.
