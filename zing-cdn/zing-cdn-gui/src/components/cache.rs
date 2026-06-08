use dioxus::prelude::*;
use crate::ipc;

#[component]
pub fn Cache() -> Element {
    let mut entries = use_resource(|| async move {
        ipc::list_cache().await.unwrap_or_default()
    });

    let guard = entries.read();
    let list: Vec<ipc::CacheEntry> = match &*guard {
        Some(v) => v.clone(),
        _ => vec![],
    };
    let is_empty = list.is_empty();
    drop(guard);

    rsx! {
        div { class: "card",
            h3 { "Cached Blobs" }
            if is_empty {
                p { "No cached blobs." }
            } else {
                table {
                    thead {
                        tr { th { "Blob ID" } th { "Size" } th { "Pinned" } th { "Actions" } }
                    }
                    tbody {
                        for entry in &list {
                            tr {
                                td { code { "{entry.blob_id}" } }
                                td { "{entry.size} bytes" }
                                td { if entry.pinned { "✓" } else { "" } }
                                td {
                                    button {
                                        onclick: {
                                            let id = entry.blob_id.clone();
                                            let p = entry.pinned;
                                            move |_| {
                                                let id = id.clone();
                                                spawn(async move {
                                                    let _ = if p { ipc::unpin_blob(&id).await } else { ipc::pin_blob(&id).await };
                                                    entries.restart();
                                                });
                                            }
                                        },
                                        if entry.pinned { "Unpin" } else { "Pin" }
                                    }
                                    " "
                                    button {
                                        onclick: {
                                            let id = entry.blob_id.clone();
                                            move |_| {
                                                let id = id.clone();
                                                spawn(async move {
                                                    let _ = ipc::delete_blob(&id).await;
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
