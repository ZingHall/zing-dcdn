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
                    thead {
                        tr { th { "Blob ID" } th { "Size" } th { "Pinned" } th { "Actions" } }
                    }
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
