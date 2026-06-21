use dioxus::prelude::*;
use crate::ipc;
use crate::components::toast::{add_toast, ToastLevel};

#[component]
pub fn Staking() -> Element {
    let peers = use_resource(|| async move {
        ipc::list_staking().await.unwrap_or_default()
    });

    let my_peer = use_resource(|| async move {
        ipc::get_my_peer_info().await.ok()
    });

    let guard = peers.read();
    let list: Vec<ipc::StakingPeerInfo> = match &*guard {
        Some(v) => v.clone(),
        _ => vec![],
    };
    let is_empty = list.is_empty();
    drop(guard);

    let my_info = my_peer.read();
    let my_peer_data: Option<ipc::MyPeerInfo> = match &*my_info {
        Some(Some(info)) => Some(info.clone()),
        _ => None,
    };
    drop(my_info);

    let my_wallet = my_peer_data.as_ref().map(|m| m.wallet_address.clone());

    {
        let mut peers = peers.clone();
        use_effect(move || {
            spawn(async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(10000).await;
                    peers.restart();
                }
            });
        });
    }

    {
        let mut my_peer = my_peer.clone();
        use_effect(move || {
            spawn(async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(10000).await;
                    my_peer.restart();
                }
            });
        });
    }

    let on_register = move |_| {
        spawn(async move {
            match ipc::register_peer().await {
                Ok(msg) => {
                    add_toast(&msg, ToastLevel::Success);
                }
                Err(e) => {
                    add_toast(&format!("Register failed: {}", e), ToastLevel::Error);
                }
            }
        });
    };

    let on_update_peer_id = move |_| {
        spawn(async move {
            match ipc::update_peer_id().await {
                Ok(msg) => {
                    add_toast(&msg, ToastLevel::Success);
                }
                Err(e) => {
                    add_toast(&format!("Update failed: {}", e), ToastLevel::Error);
                }
            }
        });
    };

    let my_peer_card = if let Some(my) = &my_peer_data {
        if my.is_registered {
            Some(rsx! {
                div { class: "card",
                    h3 { style: "margin: 0 0 8px 0;", "My Peer" }
                    p { b { "Wallet: " } code { "{my.wallet_address}" } }
                    p { b { "Peer ID: " } code { "{my.peer_id_short.as_deref().unwrap_or(\"N/A\")}" } }
                    p { b { "Bond: " }
                        if let Some(bond) = my.bond {
                            "{bond / 1_000_000_000}.{bond % 1_000_000_000:09} WAL"
                        } else {
                            "N/A"
                        }
                    }
                    p { b { "Active: " }
                        if my.is_active.unwrap_or(false) { "Yes" } else { "No" }
                    }
                    if my.needs_update {
                        p { style: "color: #e65100; font-size: 0.85rem; margin: 4px 0;",
                            "on-chain peer ID differs from current. Update to match."
                        }
                        button {
                            onclick: on_update_peer_id,
                            style: "background: #e65100; color: white; border: none; padding: 8px 16px; border-radius: 4px; cursor: pointer; font-size: 0.9rem; margin-top: 8px;",
                            "Update Peer ID"
                        }
                    }
                }
            })
        } else {
            Some(rsx! {
                div { class: "card", style: "display: flex; align-items: center; justify-content: space-between;",
                    div {
                        h3 { style: "margin: 0 0 4px 0;", "Not Registered" }
                        p { style: "font-size: 0.85rem; color: #666; margin: 0;",
                            "Register your peer on-chain to participate in the network."
                        }
                    }
                    button {
                        onclick: on_register,
                        style: "background: #4caf50; color: white; border: none; padding: 8px 16px; border-radius: 4px; cursor: pointer; font-size: 0.9rem;",
                        "Register Peer"
                    }
                }
            })
        }
    } else {
        None
    };

    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 16px;",
            if let Some(card) = my_peer_card {
                {card}
            }

            div { class: "card",
                h3 { "All Registered Peers" }
                if is_empty {
                    p { "No registered peers found. Ensure wallet and settlement config are set." }
                } else {
                    table {
                        style: "width: 100%; border-collapse: collapse;",
                        thead {
                            tr {
                                th { style: "text-align: left; padding: 6px; border-bottom: 2px solid #ccc;" }
                                th { style: "text-align: left; padding: 6px; border-bottom: 2px solid #ccc;", "Peer ID" }
                                th { style: "text-align: left; padding: 6px; border-bottom: 2px solid #ccc;", "Address" }
                                th { style: "text-align: right; padding: 6px; border-bottom: 2px solid #ccc;", "Bond (WAL)" }
                                th { style: "text-align: center; padding: 6px; border-bottom: 2px solid #ccc;", "Active" }
                                th { style: "text-align: center; padding: 6px; border-bottom: 2px solid #ccc;", "Live" }
                            }
                        }
                        tbody {
                            for p in &list {
                                let is_own = my_wallet.as_ref().map(|w| w == &p.sui_address).unwrap_or(false);
                                let row_style = if is_own {
                                    "background: #e8f4fd; font-weight: 500;"
                                } else {
                                    ""
                                };
                                tr { style: "{row_style}",
                                    td { style: "padding: 6px;",
                                        if is_own {
                                            span { style: "color: #1976d2;", "►" }
                                        } else if p.is_live {
                                            span { style: "color: #4caf50;", "●" }
                                        } else if p.is_active {
                                            span { style: "color: #ff9800;", "●" }
                                        } else {
                                            span { style: "color: #999;", "●" }
                                        }
                                    }
                                    td { style: "padding: 6px;", code { "{p.peer_id_short}" } }
                                    td { style: "padding: 6px;",
                                        code { style: "font-size: 0.8rem;",
                                            "{&p.sui_address[..6]}...{&p.sui_address[p.sui_address.len() - 4..]}"
                                        }
                                    }
                                    td { style: "padding: 6px; text-align: right;",
                                        "{p.bond / 1_000_000_000}.{p.bond % 1_000_000_000:09}"
                                    }
                                    td { style: "padding: 6px; text-align: center;",
                                        if is_own { "—" } else if p.is_active { "Yes" } else { "No" }
                                    }
                                    td { style: "padding: 6px; text-align: center;",
                                        if is_own { "—" } else if p.is_live { "Yes" } else { "No" }
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
