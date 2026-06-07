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

    let data = info.read().clone().flatten().unwrap_or(DashboardInfo {
        peer_id: String::new(),
        listen_addr: String::new(),
        connected_peers: vec![],
        cache_used: 0,
        cache_budget: 0,
        cache_count: 0,
    });
    let peer_id = data.peer_id.clone();
    let listen_addr = data.listen_addr.clone();
    let connected = data.connected_peers.clone();
    let cache_count = data.cache_count;
    let cache_used_mb = data.cache_used / (1024 * 1024);
    let cache_budget_mb = data.cache_budget / (1024 * 1024);
    let usage_pct = if data.cache_budget > 0 {
        (data.cache_used as f64 / data.cache_budget as f64 * 100.0) as u32
    } else {
        0
    };

    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 16px;",
            div { class: "card",
                 h3 { style: "margin: 0 0 8px 0;", "P2P Node" }
                 p { b { "Peer ID: " } code { "{peer_id}" } }
                 p { b { "Listen: " } code { "{listen_addr}" } }
                 p { b { "Connected peers: " } "{connected.len()}" }
                 ul {
                     for p in &connected {
                         li { code { "{p}" } }
                     }
                 }
            }
            div { class: "card",
                 h3 { style: "margin: 0 0 8px 0;", "Cache" }
                 p { b { "Cached blobs: " } "{cache_count}" }
                 p { b { "Disk usage: " } }
                 div {
                     style: "background: #e0e0e0; border-radius: 4px; height: 20px; width: 100%;",
                     div {
                         style: "background: #4caf50; height: 100%; width: {usage_pct}%; transition: width 0.3s;",
                     }
                 }
                 p { style: "font-size: 0.85rem; color: #666;",
                     "{cache_used_mb} MB / {cache_budget_mb} MB"
                 }
            }
        }
    }
}
