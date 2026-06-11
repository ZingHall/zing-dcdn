use dioxus::prelude::*;
use crate::ipc;

fn default_peers() -> ipc::PeersInfo {
    ipc::PeersInfo {
        bootstrap: vec![],
        connected: vec![],
        listen_addr: "loading...".into(),
        cache_dir: "loading...".into(),
        p2p_addr: "loading...".into(),
    }
}

async fn refresh_peers(mut signal: Signal<ipc::PeersInfo>) {
    if let Ok(info) = ipc::list_peers().await {
        signal.set(info);
    }
}

#[component]
pub fn Settings() -> Element {
    let mut input_addr = use_signal(|| String::new());
    let mut status = use_signal(|| String::new());
    let peers = use_signal(default_peers);

    spawn(refresh_peers(peers.clone()));

    let info = peers.read();
    let status_text = status.read().clone();
    let listen = info.listen_addr.clone();
    let cache = info.cache_dir.clone();
    let p2p_addr = info.p2p_addr.clone();

    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 16px;",

            div { class: "card",
                h3 { style: "margin: 0 0 8px 0;", "Network" }
                p { b { "Connected peers: " } "{info.connected.len()}" }
                ul {
                    for p in &info.connected {
                        li { code { "{p}" } }
                    }
                }
            }

            div { class: "card",
                h3 { style: "margin: 0 0 8px 0;", "Bootstrap Peers" }

                div { style: "display: flex; gap: 8px; margin-bottom: 12px;",
                    input {
                        value: "{input_addr}",
                        placeholder: "/ip4/.../udp/.../quic-v1/p2p/12D3KooW...",
                        style: "flex: 1;",
                        oninput: move |e| input_addr.set(e.value()),
                    }
                    button {
                        onclick: move |_| {
                            let addr = input_addr();
                            if addr.is_empty() { return; }
                            status.set("Adding...".into());
                            let a = addr.clone();
                            let p = peers;
                            spawn(async move {
                                match ipc::add_peer(&a).await {
                                    Ok(()) => {
                                        status.set("Added".into());
                                        input_addr.set(String::new());
                                        refresh_peers(p).await;
                                    }
                                    Err(e) => status.set(format!("Error: {e}")),
                                }
                            });
                        },
                        "Add"
                    }
                }

                if !status_text.is_empty() {
                    p { style: "font-size: 0.85rem; margin-bottom: 8px;", "{status_text}" }
                }

                if info.bootstrap.is_empty() {
                    p { style: "color: #999;", "No bootstrap peers configured." }
                } else {
                    for addr in &info.bootstrap {
                        div { style: "display: flex; align-items: center; gap: 8px; padding: 6px 0; border-bottom: 1px solid #eee;",
                            code { style: "flex: 1; font-size: 0.75rem; word-break: break-all;", "{addr}" }
                            button {
                                onclick: {
                                    let a = addr.clone();
                                    let p = peers;
                                    move |_| {
                                        let a = a.clone();
                                        spawn(async move {
                                            let _ = ipc::remove_peer(&a).await;
                                            refresh_peers(p).await;
                                        });
                                    }
                                },
                                style: "color: #c00; flex-shrink: 0;",
                                "Remove"
                            }
                        }
                    }
                }
            }

            div { class: "card",
                h3 { style: "margin: 0 0 8px 0;", "Info" }
                p { style: "font-size: 0.85rem;",
                    b { "Your P2P address: " }
                }
                div { style: "background: #f0f0f0; padding: 8px 12px; border-radius: 4px; margin-bottom: 8px;",
                    code { style: "font-size: 0.7rem; word-break: break-all; user-select: all;", "{p2p_addr}" }
                }
                p { style: "font-size: 0.85rem;",
                    b { "Cache: " } "{cache} (500 MB)"
                }
                p { style: "font-size: 0.85rem;",
                    b { "Listen: " } "{listen}"
                }
            }
        }
    }
}
