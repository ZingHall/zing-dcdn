# Desktop App Design

## Overview

Build a Dioxus 0.7 desktop GUI for zing-cdn. The app wraps `zing-cdn-core` with a reactive UI for fetching, viewing, and managing Walrus blobs while running a background P2P swarm.

**MVP shell**: `dioxus-desktop` standalone (same WebView engine as Tauri, no Wasm compilation). Migration to full Tauri v2 is a straightforward crate swap later — the architecture (commands.rs layer, Dioxus components, AppState) is identical to the Tauri pattern.

## Architecture

**New workspace member**: `zing-cdn-gui/`

```
zing-cdn/
├── zing-cdn-core/     (library — unchanged)
├── zing-cdn/          (CLI binary — unchanged)
└── zing-cdn-gui/      (desktop app — NEW)
    ├── Cargo.toml
    ├── src/
    │   ├── main.rs           App entry, starts Walrus + P2P + Dioxus
    │   ├── lib.rs            Dioxus app root + component tree
    │   ├── commands.rs       "Commands" layer (isolated, same pattern as Tauri #[command])
    │   ├── state.rs          Shared AppState
    │   └── components/
    │       ├── mod.rs
    │       ├── dashboard.rs  P2P swarm status
    │       ├── blob.rs       Blob fetch + content preview
    │       └── cache.rs      Cache list + pin/unpin
    └── assets/
        └── icon.png
```

**Data flow:**

```
Dioxus Component  →  commands::cmd_resolve()  →  zing_cdn_core library
←  Result<T>      ←  (direct Rust call, no serialization)
```

The "commands" layer is a set of free functions that take `&AppState` and return `Result<T>`. This mirrors the `#[tauri::command]` pattern exactly. When Tauri v2 is added later, each function gets a `#[tauri::command]` attribute and `State<'_, AppState>` parameter — no logic changes.

**Startup sequence (main.rs):**

1. Open RocksDB cache (`BlobStore::open`)
2. Connect to Walrus mainnet (`ZingClient::from_mainnet`)
3. Start P2P swarm in background tokio task
4. Construct `AppState` with all handles
5. Launch Dioxus-desktop window

**Shutdown:** Window close event drops `AppState`, which drops `P2pCommand` sender, swarm exits.

## State Management

```rust
pub struct AppState {
    pub client: Arc<ZingClient>,
    pub store: Arc<RwLock<BlobStore>>,
    pub pinning: Arc<RwLock<PinningManager>>,
    pub eviction: Arc<RwLock<EvictionManager>>,
    pub resolver: Resolver,
    pub p2p_tx: mpsc::Sender<P2pCommand>,
    pub peer_id: PeerId,
    pub listen_addr: Multiaddr,
}
```

`AppState` is stored in `Arc<RwLock<AppState>>` and passed to Dioxus via `use_shared_state::<Arc<RwLock<AppState>>>()`. Commands clone the `Arc` and operate on it.

## Screens

### Tab 1: Dashboard

Polls every 2 seconds via `use_coroutine` timer + `GetConnectedPeers`.

| Widget | Source |
|---|---|
| PeerId card | `state.peer_id` |
| Listen address | `state.listen_addr` |
| Connected peers | `commands::get_connected_peers(&state)` |
| Cache usage bar | `commands::get_cache_usage(&state)` → progress bar: XX MB / 500 MB |
| Cached blob count | `commands::get_cached_blob_count(&state)` |

### Tab 2: Blob Browser

Two-panel split layout:

**Left — Fetch:**
- Text input for blob ID + "Fetch" button
- Shows resolve result: source (L0/L1/L3), size, resolution time
- Calls `commands::resolve_blob()` which calls `resolver.resolve()`

**Right — Preview:**
- Content sniffing: if bytes start with `<!DOCTYPE`, `<html`, render as HTML via `dangerous_inner_html`. Otherwise plain text. Binary → hex dump.
- "Copy" button (clipboard API)
- "Save As..." button (Tauri dialog when available, otherwise browser download)

### Tab 3: Cache

Table with columns: Blob ID, Size, Pinned.

Per-row actions: Pin | Unpin | Delete.

## Commands Layer (commands.rs)

All functions take `&AppState` and return `Result<T, String>`.

| Function | Calls |
|---|---|
| `resolve_blob(state, blob_id) -> BlobInfo` | `resolver.resolve()` |
| `get_blob_content(state, blob_id) -> Vec<u8>` | `resolver.resolve()` |
| `list_cache(state) -> Vec<CacheEntry>` | `store.list_blob_ids()`, `blob_size` |
| `pin_blob(state, blob_id) -> ()` | `pinning.pin()` |
| `unpin_blob(state, blob_id) -> ()` | `pinning.unpin()` |
| `delete_blob(state, blob_id) -> ()` | `store.delete()` |
| `get_dashboard_info(state) -> DashboardInfo` | `GetConnectedPeers`, `total_size()` |
| `verify_blob(state, blob_id) -> bool` | `verifier.verify_blob_against_metadata()` |
| `get_connected_peers(state) -> Vec<PeerId>` | `GetConnectedPeers` |

## Dependencies (new)

```toml
# zing-cdn-gui/Cargo.toml
[package]
name = "zing-cdn-gui"
version = "0.1.0"
edition = "2021"

[dependencies]
zing-cdn-core = { path = "../zing-cdn-core" }
dioxus = { version = "0.7", features = ["desktop"] }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
```

## Workspace changes

Add `"zing-cdn-gui"` to workspace `members` in root Cargo.toml.

## Testing

- Unit tests for each `commands.rs` function with a temp RocksDB + mock client
- `cargo test` for `zing-cdn-gui`
- Manual smoke test: launch window, verify dashboard populates, fetch known blob, preview renders, pin/unpin works, cache list updates

## Out of Scope (Future)

- Full Tauri v2 integration (system tray, auto-update, file dialog)
- Walrus site rendering via embedded HTTP server + webview (needs on-chain Article metadata)
- Content type detection by MIME
- Background cache eviction daemon
- Settings panel (cache dir, P2P port)
- Progress bar for blob fetch
