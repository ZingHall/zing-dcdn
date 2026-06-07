# Desktop App (Tauri v2 + Dioxus 0.7) Implementation Plan

**Goal:** Create a Tauri v2 desktop app with a Dioxus 0.7 frontend for zing-cdn. Supports blob fetch, preview, cache management, and P2P swarm monitoring.

**Architecture:** Workspace member is the Tauri v2 backend crate (`zing-cdn-gui/src-tauri/`) which wraps `zing-cdn-core` and exposes `#[tauri::command]` fns. The Dioxus web frontend (`zing-cdn-gui/src/`) compiles to Wasm and renders in Tauri's webview. IPC via wasm-bindgen invoke bridge (`window.__TAURI__.core.invoke`).

**Tech Stack:** Tauri v2, Dioxus 0.7 (web), wasm-bindgen, zing-cdn-core

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `zing-cdn/Cargo.toml` | Modify | Add `"zing-cdn-gui/src-tauri"` to workspace members |
| `zing-cdn-gui/index.html` | Create | Tauri webview entry that loads the Wasm bundle |
| `zing-cdn-gui/Dioxus.toml` | Create | Dioxus CLI config for wasm build |
| `zing-cdn-gui/src/main.rs` | Create | Dioxus app entry |
| `zing-cdn-gui/src/lib.rs` | Create | Root component with tab navigation |
| `zing-cdn-gui/src/ipc.rs` | Create | wasm-bindgen invoke bridge to Tauri |
| `zing-cdn-gui/src/components/mod.rs` | Create | Component module exports |
| `zing-cdn-gui/src/components/dashboard.rs` | Create | P2P swarm status tab |
| `zing-cdn-gui/src/components/blob.rs` | Create | Blob fetch + content preview tab |
| `zing-cdn-gui/src/components/cache.rs` | Create | Cache list + pin/unpin tab |
| `zing-cdn-gui/src-tauri/Cargo.toml` | Create | Tauri backend deps |
| `zing-cdn-gui/src-tauri/build.rs` | Create | tauri-build (auto-generated) |
| `zing-cdn-gui/src-tauri/capabilities/default.json` | Create | Tauri v2 capability permissions |
| `zing-cdn-gui/src-tauri/tauri.conf.json` | Create | Tauri v2 config |
| `zing-cdn-gui/src-tauri/src/main.rs` | Create | Tauri entry, Walrus + P2P startup |
| `zing-cdn-gui/src-tauri/src/commands.rs` | Create | `#[tauri::command]` bridge fns |
| `zing-cdn-gui/src-tauri/src/state.rs` | Create | Shared `AppState` |
| `zing-cdn-gui/src-tauri/gen/schemas/desktop-schema.json` | Create | Auto-generated schema |

---

### Task 1: Scaffold workspace member + backend Cargo.toml

**Files:**
- Modify: `zing-cdn/Cargo.toml`
- Create: `zing-cdn-gui/src-tauri/Cargo.toml`
- Create: `zing-cdn-gui/src-tauri/build.rs`

- [ ] **Step 1: Update workspace Cargo.toml**

Add `"zing-cdn-gui/src-tauri"` to the members list in `/Users/jareklin/dev/main/zing-bitTorrent-client/zing-cdn/Cargo.toml`:
```toml
members = [
    "zing-cdn-core",
    "zing-cdn",
    "zing-cdn-gui/src-tauri",
]
```

- [ ] **Step 2: Create backend Cargo.toml**

`/Users/jareklin/dev/main/zing-bitTorrent-client/zing-cdn-gui/src-tauri/Cargo.toml`:
```toml
[package]
name = "zing-cdn-gui"
version = "0.1.0"
edition = "2021"

[lib]
name = "zing_cdn_gui_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[dependencies]
zing-cdn-core = { path = "../../zing-cdn-core" }
tauri = { version = "2", features = ["tray-icon"] }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
chrono = { workspace = true }
libp2p = { workspace = true }
dirs = "5"

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

- [ ] **Step 3: Create build.rs**

`/Users/jareklin/dev/main/zing-bitTorrent-client/zing-cdn-gui/src-tauri/build.rs`:
```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **Step 4: Verify compiles**

```bash
cd /Users/jareklin/dev/main/zing-bitTorrent-client/zing-cdn && cargo check -p zing-cdn-gui
```

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: scaffold zing-cdn-gui Tauri v2 backend crate"
```

---

### Task 2: AppState + Tauri commands

**Files:**
- Create: `zing-cdn-gui/src-tauri/src/state.rs`
- Create: `zing-cdn-gui/src-tauri/src/commands.rs`

- [ ] **Step 1: Create state.rs**

```rust
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use libp2p::{PeerId, Multiaddr};

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::node::P2pCommand;
use zing_cdn_core::p2p::behaviour::BlobStoreHandle;

pub struct AppState {
    pub store: Arc<RwLock<BlobStore>>,
    pub pinning: Arc<RwLock<PinningManager>>,
    pub eviction: Arc<RwLock<EvictionManager>>,
    pub p2p_tx: mpsc::Sender<P2pCommand>,
    pub peer_id: PeerId,
    pub listen_addr: Multiaddr,
    pub p2p_store: BlobStoreHandle,
}
```

- [ ] **Step 2: Create commands.rs**

```rust
use std::sync::Arc;
use tokio::sync::{RwLock, oneshot};
use tauri::State;

use crate::state::AppState;

use zing_cdn_core::mesh::resolver::Resolver;
use zing_cdn_core::mesh::reputation::PeerReputationTable;
use zing_cdn_core::walrus::verify::BlobVerifier;
use zing_cdn_core::client::ZingClient;

#[derive(serde::Serialize)]
pub struct BlobInfo {
    pub blob_id: String,
    pub size: u64,
    pub source: String,
    pub cached: bool,
}

#[derive(serde::Serialize)]
pub struct CacheEntry {
    pub blob_id: String,
    pub size: u64,
    pub pinned: bool,
}

#[derive(serde::Serialize)]
pub struct DashboardInfo {
    pub peer_id: String,
    pub listen_addr: String,
    pub connected_peers: Vec<String>,
    pub cache_used: u64,
    pub cache_budget: u64,
    pub cache_count: usize,
}

struct ResolveSession {
    _client: ZingClient,
    resolver: Resolver,
}

async fn get_resolver(state: &State<'_, AppState>) -> Result<ResolveSession, String> {
    let client = ZingClient::from_mainnet().await.map_err(|e| e.to_string())?;
    let verifier = Arc::new(BlobVerifier::new(client.encoding_config_arc()));
    let mut resolver = Resolver::new(
        state.store.clone(),
        state.pinning.clone(),
        state.eviction.clone(),
        client.walrus_client_arc(),
        verifier,
        Arc::new(RwLock::new(PeerReputationTable::new())),
    );
    resolver.set_p2p_channel(state.p2p_tx.clone());
    Ok(ResolveSession { _client: client, resolver })
}

#[tauri::command]
pub async fn resolve_blob(blob_id: String, state: State<'_, AppState>) -> Result<BlobInfo, String> {
    let id: zing_cdn_core::BlobId = blob_id.parse().map_err(|_: zing_cdn_core::types::ZingError| format!("invalid blob id: {blob_id}"))?;
    let session = get_resolver(&state).await?;
    let result = session.resolver.resolve(&id).await.map_err(|e| e.to_string())?;
    let source = match result.resolution {
        zing_cdn_core::types::BlobResolution::LocalCache => "L0 local cache",
        zing_cdn_core::types::BlobResolution::L1Peer => "L1 peer",
        zing_cdn_core::types::BlobResolution::L3Walrus => "L3 Walrus",
    };
    Ok(BlobInfo {
        blob_id,
        size: result.data.len() as u64,
        source: source.to_string(),
        cached: result.cached,
    })
}

#[tauri::command]
pub async fn get_blob_content(blob_id: String, state: State<'_, AppState>) -> Result<Vec<u8>, String> {
    // Try local cache first
    {
        let store = state.store.read().await;
        if let Some(data) = store.get(&blob_id).map_err(|e| e.to_string())? {
            return Ok(data);
        }
    }
    // Resolve from network
    let id: zing_cdn_core::BlobId = blob_id.parse().map_err(|_: zing_cdn_core::types::ZingError| format!("invalid blob id: {blob_id}"))?;
    let session = get_resolver(&state).await?;
    let result = session.resolver.resolve(&id).await.map_err(|e| e.to_string())?;
    Ok(result.data)
}

#[tauri::command]
pub async fn list_cache(state: State<'_, AppState>) -> Result<Vec<CacheEntry>, String> {
    let store = state.store.read().await;
    let pinning = state.pinning.read().await;
    let mut entries = Vec::new();
    for id in store.list_blob_ids().map_err(|e| e.to_string())? {
        let size = store.blob_size(&id).map_err(|e| e.to_string())?.unwrap_or(0);
        let pinned = pinning.is_pinned(&id).map_err(|e| e.to_string())?;
        entries.push(CacheEntry { blob_id: id, size, pinned });
    }
    Ok(entries)
}

#[tauri::command]
pub async fn pin_blob(blob_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.pinning.read().await.pin(&blob_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unpin_blob(blob_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.pinning.read().await.unpin(&blob_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_blob(blob_id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.store.write().await.delete(&blob_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_dashboard_info(state: State<'_, AppState>) -> Result<DashboardInfo, String> {
    let store = state.store.read().await;
    let ids = store.list_blob_ids().map_err(|e| e.to_string())?;
    let cache_count = ids.len();
    let cache_used = store.total_size().map_err(|e| e.to_string())?;

    let (reply, rx) = oneshot::channel();
    state.p2p_tx.send(P2pCommand::GetConnectedPeers { reply }).await.map_err(|e| e.to_string())?;
    let connected = rx.await.map_err(|e| e.to_string())?;

    Ok(DashboardInfo {
        peer_id: state.peer_id.to_string(),
        listen_addr: state.listen_addr.to_string(),
        connected_peers: connected.iter().map(|p| p.to_string()).collect(),
        cache_used,
        cache_budget: 500 * 1024 * 1024,
        cache_count,
    })
}
```

- [ ] **Step 3: Verify compiles**

```bash
cd /Users/jareklin/dev/main/zing-bitTorrent-client/zing-cdn && cargo check -p zing-cdn-gui
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: add AppState and Tauri commands"
```

---

### Task 3: Tauri main.rs + config

**Files:**
- Create: `zing-cdn-gui/src-tauri/src/main.rs`
- Create: `zing-cdn-gui/src-tauri/tauri.conf.json`
- Create: `zing-cdn-gui/src-tauri/capabilities/default.json`

- [ ] **Step 1: Create main.rs**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use libp2p::Multiaddr;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::node::{ZingP2pNode};

use crate::state::AppState;

const CACHE_BUDGET: u64 = 500 * 1024 * 1024;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::resolve_blob,
            commands::get_blob_content,
            commands::list_cache,
            commands::pin_blob,
            commands::unpin_blob,
            commands::delete_blob,
            commands::get_dashboard_info,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
                let _guard = rt.enter();

                let cache_dir = dirs::home_dir()
                    .unwrap_or_default()
                    .join(".zing-cdn")
                    .join("cache");
                std::fs::create_dir_all(&cache_dir).expect("create cache dir");

                let store = Arc::new(RwLock::new(
                    BlobStore::open(&cache_dir).expect("open blob store"),
                ));

                // P2P swarm
                let (p2p_node, command_rx) = ZingP2pNode::new(store.clone());
                let p2p_tx = p2p_node.command_tx().clone();
                let p2p_key = p2p_node.key().clone();
                let peer_id = p2p_node.local_peer_id();
                let listen_addr: Multiaddr = "/ip4/0.0.0.0/udp/34291/quic-v1"
                    .parse()
                    .expect("valid listen addr");

                let p2p_store = store.clone();
                let listen_clone = listen_addr.clone();
                rt.spawn(async move {
                    let _ = ZingP2pNode::run(
                        p2p_key, command_rx, p2p_store, listen_clone, vec![],
                    ).await;
                });

                let pinning = Arc::new(RwLock::new(PinningManager::new(
                    store.blocking_read().clone(),
                )));
                let eviction = Arc::new(RwLock::new(EvictionManager::new(
                    store.blocking_read().clone(),
                    CACHE_BUDGET,
                )));

                let state = AppState {
                    store,
                    pinning,
                    eviction,
                    p2p_tx,
                    peer_id,
                    listen_addr,
                    p2p_store: store.clone(),
                };

                handle.insert(state).expect("insert state");
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running tauri");
}
```

- [ ] **Step 2: Create tauri.conf.json**

```json
{
  "productName": "zing-cdn",
  "version": "0.1.0",
  "identifier": "com.zing-cdn.app",
  "build": {
    "frontendDist": "../dist",
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "",
    "beforeBuildCommand": ""
  },
  "app": {
    "windows": [
      {
        "title": "zing-cdn — Walrus P2P Content Mesh",
        "width": 1024,
        "height": 768,
        "resizable": true
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ]
  }
}
```

- [ ] **Step 3: Create capabilities/default.json**

Tauri v2 requires capability files for IPC permissions. Create:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capabilities for the main window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "shell:allow-open"
  ]
}
```

- [ ] **Step 4: Create Dioxus.toml**

At `zing-cdn-gui/Dioxus.toml`:
```toml
[application]
name = "zing-cdn"
author = ""
version = "0.1.0"
build = "../.dioxus/dist"

[web.app]
title = "zing-cdn"
base_path = "/"
assets_dir = "public"
```

- [ ] **Step 5: Verify compiles**

```bash
cd /Users/jareklin/dev/main/zing-bitTorrent-client/zing-cdn && cargo check -p zing-cdn-gui
```

Note: The `setup` closure runs in a spawned thread because Tauri's setup hook requires `Send`. We use `handle.insert()` to register the managed state from the thread.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: add Tauri main entry, config, and capabilities"
```

---

### Task 4: Dioxus frontend — ipc bridge + entry point

**Files:**
- Create: `zing-cdn-gui/src/ipc.rs`
- Create: `zing-cdn-gui/src/main.rs`
- Create: `zing-cdn-gui/src/lib.rs`
- Create: `zing-cdn-gui/src/components/mod.rs`
- Create: `zing-cdn-gui/index.html`

These are Rust files compiled to WASM by `dx build` (Dioxus CLI). They DON'T need Cargo.toml in the workspace — Dioxus manages its own build.

- [ ] **Step 1: Create ipc.rs** (wasm-bindgen invoke bridge)

```rust
use wasm_bindgen::prelude::*;
use serde::Serialize;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

pub async fn invoke_cmd<T: serde::de::DeserializeOwned>(
    cmd: &str,
    args: impl Serialize,
) -> Result<T, String> {
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| e.to_string())?;
    let result = invoke(cmd, args).await;
    serde_wasm_bindgen::from_value::<T>(result).map_err(|e| e.to_string())
}

pub async fn invoke_void(cmd: &str, args: impl Serialize) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| e.to_string())?;
    invoke(cmd, args).await;
    Ok(())
}
```

- [ ] **Step 2: Create main.rs** (Dioxus entry)

```rust
use zing_cdn_gui::App;

fn main() {
    dioxus::launch(App);
}
```

- [ ] **Step 3: Create lib.rs** (Root component with tab navigation)

```rust
mod ipc;
mod components;

pub use components::*;

use dioxus::prelude::*;

#[derive(PartialEq, Clone)]
enum Tab {
    Dashboard,
    BlobBrowser,
    Cache,
}

#[component]
pub fn App() -> Element {
    let tab = use_signal(|| Tab::Dashboard);

    rsx! {
        div { class: "app",
            style: "font-family: system-ui, sans-serif; padding: 16px; max-width: 900px; margin: 0 auto;",
            h1 { style: "font-size: 1.2rem; margin: 0 0 8px 0;", "zing-cdn" }
            nav { style: "display: flex; gap: 8px; margin-bottom: 16px; border-bottom: 1px solid #ccc; padding-bottom: 8px;",
                button {
                    onclick: move |_| tab.set(Tab::Dashboard),
                    style: if tab() == Tab::Dashboard { "font-weight: bold" } else { "" },
                    "Dashboard"
                }
                button {
                    onclick: move |_| tab.set(Tab::BlobBrowser),
                    style: if tab() == Tab::BlobBrowser { "font-weight: bold" } else { "" },
                    "Blob Browser"
                }
                button {
                    onclick: move |_| tab.set(Tab::Cache),
                    style: if tab() == Tab::Cache { "font-weight: bold" } else { "" },
                    "Cache"
                }
            }
            div { class: "content",
                match tab() {
                    Tab::Dashboard => rsx! { Dashboard {} },
                    Tab::BlobBrowser => rsx! { BlobBrowser {} },
                    Tab::Cache => rsx! { Cache {} },
                }
            }
        }
    }
}
```

- [ ] **Step 4: Create components/mod.rs**

```rust
pub mod dashboard;
pub mod blob;
pub mod cache;
```

- [ ] **Step 5: Create index.html**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>zing-cdn</title>
  <style>
    body { margin: 0; background: #fafafa; }
    button { cursor: pointer; padding: 6px 14px; border: 1px solid #888; border-radius: 6px; background: #fff; font-size: 0.85rem; }
    button:hover { background: #eee; }
    input { padding: 6px 10px; border: 1px solid #ccc; border-radius: 6px; width: 100%; box-sizing: border-box; }
    table { width: 100%; border-collapse: collapse; }
    th, td { text-align: left; padding: 8px 12px; border-bottom: 1px solid #ddd; }
    pre { background: #f4f4f4; padding: 12px; border-radius: 6px; overflow-x: auto; }
    code { font-size: 0.8rem; }
    .card { background: #fff; border: 1px solid #ddd; border-radius: 8px; padding: 16px; }
  </style>
</head>
<body>
  <div id="main"></div>
  <script type="module">
    import init from '/target/wasm32-unknown-unknown/release/zing_cdn_gui.js';
    init('/target/wasm32-unknown-unknown/release/zing_cdn_gui_bg.wasm');
  </script>
</body>
</html>
```

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: add Dioxus frontend with IPC bridge and root app component"
```

---

### Task 5: Dioxus components (Dashboard, Blob Browser, Cache)

**Files:**
- Create: `zing-cdn-gui/src/components/dashboard.rs`
- Create: `zing-cdn-gui/src/components/blob.rs`
- Create: `zing-cdn-gui/src/components/cache.rs`

- [ ] **Step 1: Create dashboard.rs**

```rust
use dioxus::prelude::*;
use crate::ipc::invoke_cmd;

#[derive(serde::Deserialize, Clone)]
struct DashboardInfo {
    peer_id: String,
    listen_addr: String,
    connected_peers: Vec<String>,
    cache_used: u64,
    cache_budget: u64,
    cache_count: usize,
}

#[component]
pub fn Dashboard() -> Element {
    let info = use_resource(|| async move {
        invoke_cmd::<DashboardInfo>("get_dashboard_info", {}).await.ok()
    });

    let data = info.read().clone().unwrap_or(DashboardInfo {
        peer_id: "---".into(),
        listen_addr: "---".into(),
        connected_peers: vec![],
        cache_used: 0,
        cache_budget: 500 * 1024 * 1024,
        cache_count: 0,
    });

    let usage_pct = if data.cache_budget > 0 {
        (data.cache_used as f64 / data.cache_budget as f64 * 100.0) as u32
    } else {
        0
    };

    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 16px;",
            div { class: "card",
                h3 { style: "margin: 0 0 8px 0;", "P2P Node" }
                p { b { "Peer ID: " }, code { "{data.peer_id}" } }
                p { b { "Listen: " }, code { "{data.listen_addr}" } }
                p { b { "Connected peers: " }, "{data.connected_peers.len()}" }
                ul {
                    for p in &data.connected_peers {
                        li { code { "{p}" } }
                    }
                }
            }
            div { class: "card",
                h3 { style: "margin: 0 0 8px 0;", "Cache" }
                p { b { "Cached blobs: " }, "{data.cache_count}" }
                p { b { "Disk usage: " } }
                div { style: "background: #e0e0e0; border-radius: 4px; height: 20px; width: 100%;",
                    div { style: "background: #4caf50; height: 100%; width: {usage_pct}%; transition: width 0.3s;" }
                }
                p { style: "font-size: 0.85rem; color: #666;",
                    "{data.cache_used / (1024*1024)} MB / {data.cache_budget / (1024*1024)} MB"
                }
            }
        }
    }
}
```

- [ ] **Step 2: Create blob.rs**

```rust
use dioxus::prelude::*;
use crate::ipc::invoke_cmd;

#[derive(serde::Deserialize, Clone)]
struct BlobInfo {
    blob_id: String,
    size: u64,
    source: String,
    cached: bool,
}

#[component]
pub fn BlobBrowser() -> Element {
    let input = use_signal(|| String::new());
    let info = use_signal(|| None::<BlobInfo>);
    let data = use_signal(|| None::<Vec<u8>>);
    let err = use_signal(|| None::<String>);

    rsx! {
        div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 16px;",
            div { class: "card",
                h3 { "Fetch Blob" }
                input {
                    value: "{input}",
                    placeholder: "Blob ID (base64)",
                    oninput: move |e| input.set(e.value()),
                }
                button {
                    onclick: move |_| {
                        let id = input();
                        if id.is_empty() { return; }
                        err.set(None);
                        spawn(async move {
                            match invoke_cmd::<BlobInfo>("resolve_blob", serde_json::json!({"blobId": id})).await {
                                Ok(i) => {
                                    info.set(Some(i.clone()));
                                    data.set(
                                        invoke_cmd::<Vec<u8>>("get_blob_content", serde_json::json!({"blobId": id})).await.ok()
                                    );
                                }
                                Err(e) => err.set(Some(e)),
                            }
                        });
                    },
                    style: "margin-top: 8px;",
                    "Fetch"
                }
                if let Some(ref i) = *info.read() {
                    div { style: "margin-top: 12px;",
                        p { b { "Blob: " }, code { "{i.blob_id}" } }
                        p { b { "Size: " }, "{i.size} bytes" }
                        p { b { "Source: " }, "{i.source}" }
                        p { b { "Cached: " }, if i.cached { "yes" } else { "no" } }
                    }
                }
                if let Some(ref e) = *err.read() {
                    p { style: "color: red;", "{e}" }
                }
            }
            div { class: "card",
                h3 { "Preview" }
                if let Some(ref data) = *data.read() {
                    let text = if data.len() > 2000 {
                        format!("{}...", String::from_utf8_lossy(&data[..2000]))
                    } else {
                        String::from_utf8_lossy(data).to_string()
                    };
                    pre { "{text}" }
                    p { style: "font-size: 0.8rem; color: #888;",
                        "{data.len()} bytes total"
                    }
                } else {
                    p { style: "color: #999;", "Fetch a blob to preview" }
                }
            }
        }
    }
}
```

- [ ] **Step 3: Create cache.rs**

```rust
use dioxus::prelude::*;
use crate::ipc::{invoke_cmd, invoke_void};

#[derive(serde::Deserialize, Clone)]
struct CacheEntry {
    blob_id: String,
    size: u64,
    pinned: bool,
}

#[component]
pub fn Cache() -> Element {
    let entries = use_resource(|| async move {
        invoke_cmd::<Vec<CacheEntry>>("list_cache", {}).await.unwrap_or_default()
    });

    let list = entries.read();

    rsx! {
        div { class: "card",
            h3 { "Cached Blobs" }
            if list.is_empty() {
                p { "No cached blobs." }
            } else {
                table {
                    thead { tr { th { "Blob ID" } th { "Size" } th { "Pinned" } th { "Actions" } } }
                    tbody {
                        for entry in list.iter() {
                            tr {
                                td { code { "{entry.blob_id}" } }
                                td { "{entry.size} bytes" }
                                td { if entry.pinned { "✓" } else { "" } }
                                td {
                                    if entry.pinned {
                                        button {
                                            onclick: {
                                                let id = entry.blob_id.clone();
                                                move |_| {
                                                    let id = id.clone();
                                                    spawn(async move {
                                                        let _ = invoke_void("unpin_blob", serde_json::json!({"blobId": id})).await;
                                                        entries.restart();
                                                    });
                                                }
                                            },
                                            "Unpin"
                                        }
                                    } else {
                                        button {
                                            onclick: {
                                                let id = entry.blob_id.clone();
                                                move |_| {
                                                    let id = id.clone();
                                                    spawn(async move {
                                                        let _ = invoke_void("pin_blob", serde_json::json!({"blobId": id})).await;
                                                        entries.restart();
                                                    });
                                                }
                                            },
                                            "Pin"
                                        }
                                    }
                                    " "
                                    button {
                                        onclick: {
                                            let id = entry.blob_id.clone();
                                            move |_| {
                                                let id = id.clone();
                                                spawn(async move {
                                                    let _ = invoke_void("delete_blob", serde_json::json!({"blobId": id})).await;
                                                    entries.restart();
                                                });
                                            }
                                        },
                                        style: "color: #c00;",
                                        "Delete"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: add Dioxus components (Dashboard, BlobBrowser, Cache)"
```

---

### Task 6: Build + final verification

- [ ] **Step 1: Full workspace check**

```bash
cd /Users/jareklin/dev/main/zing-bitTorrent-client/zing-cdn && cargo check --workspace
```

Expected: all crates (zing-cdn-core, zing-cdn, zing-cdn-gui) compile.

- [ ] **Step 2: Run all tests**

```bash
cd /Users/jareklin/dev/main/zing-bitTorrent-client/zing-cdn && cargo test
```

Expected: 22 passed, 5 ignored.

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "chore: final build verification for desktop app"
```
