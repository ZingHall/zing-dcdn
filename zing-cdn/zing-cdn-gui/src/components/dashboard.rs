use dioxus::prelude::*;
use crate::ipc;

#[component]
pub fn Dashboard() -> Element {
    let info = use_resource(|| async move {
        ipc::get_dashboard().await.ok()
    });

    let guard = info.read();
    let inner: Option<&ipc::DashboardInfo> = match &*guard {
        Some(Some(d)) => Some(d),
        _ => None,
    };

    let peer_id = inner.map(|d| d.peer_id.clone()).unwrap_or_else(|| "connecting...".into());
    let listen_addr = inner.map(|d| d.listen_addr.clone()).unwrap_or_else(|| "connecting...".into());
    let connected = inner.map(|d| d.connected_peers.clone()).unwrap_or_default();
    let cache_count = inner.map(|d| d.cache_count).unwrap_or(0);
    let cache_used = inner.map(|d| d.cache_used).unwrap_or(0);
    let cache_budget = inner.map(|d| d.cache_budget).unwrap_or(500 * 1024 * 1024);
    let usage_pct = if cache_budget > 0 {
        (cache_used as f64 / cache_budget as f64 * 100.0) as u32
    } else {
        0
    };
    drop(guard);

    // Auto-refresh every 3 seconds
    {
        let mut info = info.clone();
        use_effect(move || {
            spawn(async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(3000).await;
                    info.restart();
                }
            });
        });
    }

    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 16px;",
            div { class: "card",
                 h3 { style: "margin: 0 0 8px 0;", "P2P Node" }
                 p { b { "Peer ID: " } code { "{peer_id}" } }
                 p { b { "Listen: " } code { "{listen_addr}" } }
                 p { b { "Connected peers: " } "{connected.len()}" }
                 ul {
                     for p in connected {
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
                     "{cache_used / (1024*1024)} MB / {cache_budget / (1024*1024)} MB"
                 }
            }
        }
    }
}
