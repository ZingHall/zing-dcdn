use dioxus::prelude::*;
use crate::ipc;
use crate::components::toast::{add_toast, ToastLevel};

fn format_wal(amount: u64) -> String {
    format!("{}.{:09}", amount / 1_000_000_000, amount % 1_000_000_000)
}

#[component]
fn PeerRowDisplay(info: ipc::StakingPeerInfo, is_own: bool, row_style: String, addr_short: String, expanded: Signal<Option<String>>) -> Element {
    let is_expanded = expanded() == Some(info.sui_address.clone());
    let expand_indicator = if is_expanded { "\u{25BC}" } else { "\u{25B6}" };
    let addr = info.sui_address.clone();
    let mut delegate_mode = use_signal(|| false);
    let mut delegate_amount = use_signal(|| String::new());
    let vault_id = use_signal(|| info.vault_object_id.clone().unwrap_or_default());
    let has_vault = info.vault_object_id.is_some();
    rsx! {
        tr { style: "{row_style}",
            onclick: move |_| {
                if is_expanded {
                    expanded.set(None);
                } else {
                    expanded.set(Some(addr.clone()));
                }
            },
            td { style: "padding: 6px;",
                if is_own {
                    span { style: "color: #1976d2;", "► {expand_indicator}" }
                } else if info.is_live {
                    span { "● {expand_indicator}" }
                } else if info.is_active {
                    span { style: "color: #ff9800;", "● {expand_indicator}" }
                } else {
                    span { style: "color: #999;", "● {expand_indicator}" }
                }
            }
            td { style: "padding: 6px;", code { "{info.peer_id_short}" } }
            td { style: "padding: 6px;",
                code { style: "font-size: 0.8rem;", "{addr_short}" }
            }
            td { style: "padding: 6px; text-align: right;",
                "{info.bond / 1_000_000_000}.{info.bond % 1_000_000_000:09}"
            }
            td { style: "padding: 6px; text-align: center;",
                if is_own { "—" } else if info.is_active { "Yes" } else { "No" }
            }
            td { style: "padding: 6px; text-align: center;",
                if is_own { "—" } else if info.is_live { "Yes" } else { "No" }
            }
        }
        if is_expanded {
            tr { style: "background: #f5f5f5;",
                td { colspan: "6", style: "padding: 0;",
                    if let Some((_label, reserves_str, earnings_str, commission_pct, shares)) = info.vault_reserves.map(|_| {
                        let reserves = format_wal(info.vault_reserves.unwrap_or(0));
                        let earnings = format_wal(info.vault_peer_earnings.unwrap_or(0));
                        let commission = info.vault_commission_bps.unwrap_or(0) / 100;
                        let shares_val = info.vault_total_shares.unwrap_or(0);
                        ("", reserves, earnings, commission, shares_val)
                    }) {
                        div { style: "display: grid; grid-template-columns: 1fr 1fr 1fr 1fr auto; gap: 8px; padding: 8px 16px; font-size: 0.8rem;",
                        div { b { "Reserves" } br {} "{reserves_str} WAL" }
                        div { b { "Earnings" } br {} "{earnings_str} WAL" }
                        div { b { "Commission" } br {} "{commission_pct}%" }
                        div { b { "Shares" } br {} "{shares}" }
                        div { style: "display: flex; align-items: center; gap: 4px;",
                            if delegate_mode() {
                                input {
                                    value: "{delegate_amount()}",
                                    oninput: move |e| delegate_amount.set(e.value()),
                                    placeholder: "WAL",
                                    style: "width: 70px; padding: 3px 6px; border: 1px solid #ccc; border-radius: 3px; font-size: 0.8rem;",
                                }
                                button {
                                    onclick: move |_| {
                                        let vid = vault_id();
                                        let amt = delegate_amount();
                                        if !amt.is_empty() {
                                            spawn(async move {
                                                match ipc::delegate(&vid, &amt).await {
                                                    Ok(msg) => {
                                                        add_toast(&msg, ToastLevel::Success);
                                                        delegate_mode.set(false);
                                                        delegate_amount.set(String::new());
                                                    }
                                                    Err(e) => add_toast(&format!("Delegate failed: {}", e), ToastLevel::Error),
                                                }
                                            });
                                        }
                                    },
                                    style: "background: #4caf50; color: white; border: none; padding: 3px 8px; border-radius: 3px; cursor: pointer; font-size: 0.8rem;",
                                    "Go"
                                }
                            } else {
                                button {
                                    onclick: move |_| delegate_mode.set(true),
                                    style: "background: #1976d2; color: white; border: none; padding: 3px 8px; border-radius: 3px; cursor: pointer; font-size: 0.8rem; white-space: nowrap;",
                                    "Delegate"
                                }
                            }
                        }
                    div { style: "grid-column: 1 / -1; word-break: break-all;",
                        b { "Peer Object ID: " } code { "{info.peer_object_id}" }
                    }
                    if has_vault {
                        div { style: "grid-column: 1 / -1; word-break: break-all;",
                            b { "Vault Object ID: " } code { "{vault_id()}" }
                        }
                    }
                        }
                    } else {
                        div { style: "padding: 8px 16px; font-size: 0.8rem; color: #999;",
                            "No vault created yet"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ShareRowDisplay(info: ipc::ShareCertificateInfo) -> Element {
    let val = format_wal(info.estimated_value);
    let cert_id = info.cert_object_id.clone();
    rsx! {
        tr {
            td { style: "padding: 6px;", code { "{info.vault_address}" } }
            td { style: "padding: 6px; text-align: right;", "{info.shares}" }
            td { style: "padding: 6px; text-align: right;", "{val} WAL" }
            td { style: "padding: 6px; text-align: center;",
                button {
                    onclick: move |_| {
                        let id = cert_id.clone();
                        spawn(async move {
                            match ipc::undelegate(&id).await {
                                Ok(msg) => add_toast(&msg, ToastLevel::Success),
                                Err(e) => add_toast(&format!("Undelegate failed: {}", e), ToastLevel::Error),
                            }
                        });
                    },
                    style: "background: #e65100; color: white; border: none; padding: 4px 10px; border-radius: 4px; cursor: pointer; font-size: 0.8rem;",
                    "Undelegate"
                }
            }
        }
    }
}

#[component]
pub fn Staking() -> Element {
    let peers = use_resource(|| async move {
        ipc::list_staking().await.unwrap_or_default()
    });

    let my_peer = use_resource(|| async move {
        ipc::get_my_peer_info().await.ok()
    });

    let my_vault = use_resource(|| async move {
        ipc::get_my_vault_info().await.ok()
    });

    let my_shares = use_resource(|| async move {
        ipc::list_my_shares().await.unwrap_or_default()
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

    struct RowData {
        info: ipc::StakingPeerInfo,
        is_own: bool,
        row_style: &'static str,
        addr_short: String,
    }
    let rows: Vec<RowData> = list.iter().map(|p| {
        let is_own = my_wallet.as_ref().map(|w| w == &p.sui_address).unwrap_or(false);
        let addr_short = format!(
            "{}...{}",
            &p.sui_address[..6],
            &p.sui_address[p.sui_address.len().saturating_sub(4)..]
        );
        RowData {
            info: p.clone(),
            is_own,
            row_style: if is_own { "background: #e8f4fd; font-weight: 500; cursor: pointer;" } else { "cursor: pointer;" },
            addr_short,
        }
    }).collect();

    let expanded = use_signal(|| None::<String>);

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

    {
        let mut my_vault = my_vault.clone();
        use_effect(move || {
            spawn(async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(10000).await;
                    my_vault.restart();
                }
            });
        });
    }

    {
        let mut my_shares = my_shares.clone();
        use_effect(move || {
            spawn(async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(10000).await;
                    my_shares.restart();
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

    let on_create_vault = move |_| {
        spawn(async move {
            match ipc::create_vault().await {
                Ok(msg) => {
                    add_toast(&msg, ToastLevel::Success);
                }
                Err(e) => {
                    add_toast(&format!("Create vault failed: {}", e), ToastLevel::Error);
                }
            }
        });
    };

    let on_claim_earnings = move |_| {
        spawn(async move {
            match ipc::claim_earnings().await {
                Ok(msg) => {
                    add_toast(&msg, ToastLevel::Success);
                }
                Err(e) => {
                    add_toast(&format!("Claim earnings failed: {}", e), ToastLevel::Error);
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
                    p { b { "Peer Object ID: " } code { "{my.peer_object_id}" } }
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

    let vault_guard = my_vault.read();
    let vault_data: Option<ipc::MyVaultInfo> = match &*vault_guard {
        Some(Some(v)) => Some(v.clone()),
        _ => None,
    };
    drop(vault_guard);

    let my_vault_card = vault_data.as_ref().map(|v| {
        if v.has_vault {
            let reserves = format_wal(v.reserves.unwrap_or(0));
            let earnings = format_wal(v.peer_earnings.unwrap_or(0));
            let commission_pct = v.commission_bps.unwrap_or(0) / 100;
            let shares = v.total_shares.unwrap_or(0);
            let vault_id = v.vault_object_id.as_deref().unwrap_or("N/A");
            let has_earnings = v.peer_earnings.unwrap_or(0) > 0;
            rsx! {
                div { class: "card",
                    h3 { style: "margin: 0 0 8px 0;", "My Vault" }
                    p { b { "Reserves: " } "{reserves} WAL" }
                    p { b { "Earnings: " } "{earnings} WAL" }
                    p { b { "Commission: " } "{commission_pct}%" }
                    p { b { "Shares: " } "{shares}" }
                    p { b { "Vault Object ID: " } code { "{vault_id}" } }
                    if has_earnings {
                        button {
                            onclick: on_claim_earnings,
                            style: "background: #4caf50; color: white; border: none; padding: 8px 16px; border-radius: 4px; cursor: pointer; font-size: 0.9rem; margin-top: 8px;",
                            "Claim Earnings"
                        }
                    }
                }
            }
        } else {
            rsx! {
                div { class: "card", style: "display: flex; align-items: center; justify-content: space-between;",
                    div {
                        h3 { style: "margin: 0 0 4px 0;", "No Vault" }
                        p { style: "font-size: 0.85rem; color: #666; margin: 0;",
                            "Create a vault to start receiving earnings from serving data."
                        }
                    }
                    button {
                        onclick: on_create_vault,
                        style: "background: #4caf50; color: white; border: none; padding: 8px 16px; border-radius: 4px; cursor: pointer; font-size: 0.9rem;",
                        "Create Vault"
                    }
                }
            }
        }
    });

    let shares_guard = my_shares.read();
    let shares_list: Vec<ipc::ShareCertificateInfo> = match &*shares_guard {
        Some(v) => v.clone(),
        _ => vec![],
    };
    let shares_empty = shares_list.is_empty();
    drop(shares_guard);

    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 16px;",
            if let Some(card) = my_peer_card {
                {card}
            }
            if let Some(card) = my_vault_card {
                {card}
            }

            div { class: "card",
                h3 { "My Shares" }
                if shares_empty {
                    p { "No delegation shares found." }
                } else {
                    table {
                        style: "width: 100%; border-collapse: collapse;",
                        thead {
                            tr {
                                th { style: "text-align: left; padding: 6px; border-bottom: 2px solid #ccc;", "Vault" }
                                th { style: "text-align: right; padding: 6px; border-bottom: 2px solid #ccc;", "Shares" }
                                th { style: "text-align: right; padding: 6px; border-bottom: 2px solid #ccc;", "Est. Value" }
                                th { style: "text-align: center; padding: 6px; border-bottom: 2px solid #ccc;", "Action" }
                            }
                        }
                        tbody { for s in &shares_list {
                            ShareRowDisplay { info: s.clone() }
                        } }
                    }
                }
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
                        tbody { for r in &rows {
                            PeerRowDisplay {
                                info: r.info.clone(),
                                is_own: r.is_own,
                                row_style: r.row_style.to_string(),
                                addr_short: r.addr_short.clone(),
                                expanded: expanded,
                            }
                        } }
                    }
                }
            }
        }
    }
}