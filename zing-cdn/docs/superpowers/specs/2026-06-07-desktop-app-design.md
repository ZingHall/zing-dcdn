# Desktop App Design

## Overview

Build a Tauri v2 + Dioxus 0.7 desktop GUI for zing-cdn. The app wraps `zing-cdn-core` with a reactive UI for fetching, viewing, and managing Walrus blobs while running a background P2P swarm.

**Stack**: Tauri v2 (native shell) + Dioxus 0.7 `web` feature (compiles to Wasm, runs in Tauri's webview). IPC via Tauri commands.

## Architecture

**New workspace member**: `zing-cdn-gui/`

```
zing-cdn/
├── zing-cdn-core/            (library — unchanged)
├── zing-cdn/                 (CLI binary — unchanged)
└── zing-cdn-gui/             (desktop app — NEW)
    ├── Cargo.toml
    ├── index.html             Tauri webview entry point
    ├── src-tauri/
    │   ├── Cargo.toml         Tauri backend deps
    │   ├── tauri.conf.json    Tauri config
    │   ├── build.rs           tauri-build
    │   ├── src/
    │   │   ├── main.rs        Tauri entry, starts Walrus + P2P + Dioxus
    │   │   ├── commands.rs    #[tauri::command] bridge to zing-cdn-core
    │   │   └── state.rs       Shared AppState
    │   └── icons/
    └── src/                   (Dioxus web frontend, compiled to Wasm)
        ├── main.rs            Dioxus app entry
        ├── lib.rs             Component tree
        └── components/
            ├── dashboard.rs   P2P swarm status
            ├── blob.rs        Blob fetch + content preview
            └── cache.rs       Cache list + pin/unpin
```

**Data flow:**

```
Dioxus UI (Wasm in webview)  →  invoke("cmd_name", args)  →  #[tauri::command] fn in Rust
                           ←  Result<T> via IPC            ←  zing_cdn_core
```

**Startup sequence (src-tauri/main.rs):**

1. Open RocksDB cache (`BlobStore::open`)
2. Connect to Walrus mainnet (`ZingClient::from_mainnet`)
3. Start P2P swarm in background tokio task
4. Construct `AppState` with all handles
5. Register Tauri commands + state
6. Launch Tauri window (loads index.html → Dioxus Wasm)

**Shutdown:** Tauri window close → `AppState` drops → `P2pCommand` sender drops → swarm exits.

## State Management

```rust
// src-tauri/state.rs
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

Managed by Tauri via `app.manage(app_state)`. Commands receive `State<'_, AppState>`.

## Screens

### Tab 1: Dashboard

Polls every 2 seconds via Dioxus `use_coroutine` timer + `use_invoke`.

| Widget | Tauri command |
|---|---|
| PeerId card | `get_peer_id` |
| Listen address | `get_listen_addr` |
| Connected peers | `get_connected_peers` |
| Cache usage bar | `get_cache_usage` (total_size / budget) |
| Cached blob count | `get_cache_count` |

### Tab 2: Blob Browser

Two-panel split layout:

**Left — Fetch:**
- Text input for blob ID + "Fetch" button
- Calls `resolve_blob` Tauri command
- Shows result: source (L0/L1/L3), size

**Right — Preview:**
- Content sniffing: if bytes start with `<!DOCTYPE` or `<html`, render as HTML via `dangerous_inner_html`. Otherwise plain text. Binary → hex dump.
- "Copy" button (clipboard via Tauri or web API)
- "Save As..." button (Tauri file dialog)

### Tab 3: Cache

Table: Blob ID, Size, Pinned.

Per-row actions: Pin | Unpin | Delete — each calls corresponding Tauri command.

## Tauri Commands

All live in `src-tauri/src/commands.rs`. Each is an `async fn` with `#[tauri::command]` returning `Result<T, String>`.

| Command | Calls | Frontend |
|---|---|---|
| `resolve_blob(blob_id) -> BlobInfo` | `resolver.resolve()` | Fetch blob |
| `get_blob_content(blob_id) -> Vec<u8>` | `resolver.resolve()` | Download raw |
| `list_cache() -> Vec<CacheEntry>` | `store.list_blob_ids()` | Cache tab |
| `pin_blob(blob_id)` | `pinning.pin()` | Cache tab |
| `unpin_blob(blob_id)` | `pinning.unpin()` | Cache tab |
| `delete_blob(blob_id)` | `store.delete()` | Cache tab |
| `get_connected_peers() -> Vec<PeerId>` | P2P command | Dashboard |
| `get_cache_usage() -> (u64, u64)` | `store.total_size()` | Dashboard |
| `get_cache_count() -> usize` | `store.list_blob_ids().len()` | Dashboard |
| `get_peer_id() -> String` | `state.peer_id` | Dashboard |
| `get_listen_addr() -> String` | `state.listen_addr` | Dashboard |

## Dependencies

```toml
# zing-cdn-gui/Cargo.toml — Dioxus frontend (wasm)
[dependencies]
serde = { version = "1", features = ["derive"] }
wasm-bindgen = "0.2"
dioxus = { version = "0.7", features = ["web"] }
```

```toml
# zing-cdn-gui/src-tauri/Cargo.toml — Tauri backend
[dependencies]
zing-cdn-core = { path = "../../zing-cdn-core" }
tauri = { version = "2", features = [] }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
chrono = { workspace = true }
libp2p = { workspace = true }

[build-dependencies]
tauri-build = "2"
```

## Workspace changes

Add `"zing-cdn-gui"` to workspace `members` in root Cargo.toml. The Tauri backend crate is at `zing-cdn-gui/src-tauri` and needs its own workspace entry (or we make the workspace member `zing-cdn-gui` with the Tauri crate as a relative path).

## Build & dev workflow

1. `cd zing-cdn-gui` and build the frontend (wasm): `dx build` (Dioxus CLI)
2. `cd src-tauri && cargo build` — builds Tauri backend and bundles frontend
3. Or use `tauri dev` for hot-reload

For MVP, the frontend is a simple Dioxus web app. Tauri config points to the built frontend files. During development, `trunk serve` provides hot-reload for the web app while `tauri dev` runs the native shell.

## Testing

- Unit tests for Tauri commands with a temp RocksDB + mocked `ZingClient`
- `cargo test` for the `src-tauri` crate
- Manual smoke test: launch, verify dashboard populates, fetch known blob, pin/unpin, cache list updates

## Out of Scope (Future)

- Auto-update (Tauri updater)
- System tray icon
- Walrus site rendering via embedded webview (needs on-chain Article metadata)
- Content type detection by MIME
- Background cache eviction daemon
- Settings panel (cache dir, P2P port, bootstrap peers)
- Progress bar for blob fetch
