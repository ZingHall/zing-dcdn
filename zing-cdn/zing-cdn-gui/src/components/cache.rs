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
    let mut entries = use_resource(|| async move {
        invoke_cmd::<Vec<CacheEntry>>("list_cache", {}).await.unwrap_or_default()
    });

    let list = (*entries.read()).clone().unwrap_or_default();

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
                        for cache_entry in &list {
                            tr {
                                td { code { "{cache_entry.blob_id}" } }
                                td { "{cache_entry.size} bytes" }
                                td { if cache_entry.pinned { "✓" } else { "" } }
                                td {
                                    button {
                                        onclick: {
                                            let id = cache_entry.blob_id.clone();
                                            let pinned = cache_entry.pinned;
                                            move |_| {
                                                let id = id.clone();
                                                spawn(async move {
                                                    let cmd = if pinned { "unpin_blob" } else { "pin_blob" };
                                                    let _ = invoke_void(cmd, serde_json::json!({"blobId": id})).await;
                                                    entries.restart();
                                                });
                                            }
                                        },
                                        if cache_entry.pinned { "Unpin" } else { "Pin" }
                                    }
                                    " "
                                    button {
                                        onclick: {
                                            let id = cache_entry.blob_id.clone();
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
