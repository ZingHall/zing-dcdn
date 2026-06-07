#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use tokio::sync::RwLock;
use libp2p::Multiaddr;
use tauri::Manager;

use zing_cdn_core::cache::store::BlobStore;
use zing_cdn_core::cache::pinning::PinningManager;
use zing_cdn_core::cache::eviction::EvictionManager;
use zing_cdn_core::p2p::node::ZingP2pNode;

use zing_cdn_gui_lib::state::AppState;
use zing_cdn_gui_lib::commands;

const CACHE_BUDGET: u64 = 500 * 1024 * 1024;

fn main() {
    tauri::Builder::default()
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

            let cache_dir = dirs::home_dir()
                .unwrap_or_default()
                .join(".zing-cdn")
                .join("cache");
            std::fs::create_dir_all(&cache_dir).expect("create cache dir");

            let store = Arc::new(RwLock::new(
                BlobStore::open(&cache_dir).expect("open blob store"),
            ));

            // Create P2P node (synchronous)
            let (p2p_node, command_rx) = ZingP2pNode::new(store.clone());
            let p2p_tx = p2p_node.command_tx().clone();
            let p2p_key = p2p_node.key().clone();
            let peer_id = p2p_node.local_peer_id();
            let listen_addr: Multiaddr = "/ip4/0.0.0.0/udp/34291/quic-v1"
                .parse()
                .expect("valid listen addr");

            // Create managers
            let pinning = Arc::new(RwLock::new(PinningManager::new(
                store.blocking_read().clone(),
            )));
            let eviction = Arc::new(RwLock::new(EvictionManager::new(
                store.blocking_read().clone(),
                CACHE_BUDGET,
            )));

            // Register AppState synchronously before spawning background tasks
            handle.manage(AppState {
                store: store.clone(),
                pinning,
                eviction,
                p2p_tx: p2p_tx.clone(),
                peer_id,
                listen_addr: listen_addr.clone(),
                p2p_store: store.clone(),
            });

            // Spawn P2P background task via Tauri's async runtime
            tauri::async_runtime::spawn(async move {
                let _ = ZingP2pNode::run(
                    p2p_key, command_rx, store, listen_addr, vec![],
                ).await;
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
