use dioxus::prelude::*;
use crate::ipc;
use serde::Deserialize;

#[derive(Deserialize, Clone)]
struct BlobInfo {
    blob_id: String,
    size: u64,
    source: String,
    cached: bool,
    content: String,
}

#[component]
pub fn BlobBrowser() -> Element {
    let mut input = use_signal(|| String::new());
    let mut info = use_signal(|| None::<BlobInfo>);
    let mut err = use_signal(|| None::<String>);

    let blob_info = info.read().clone();

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
                        let id_clone = id.clone();
                        spawn(async move {
                            match ipc::invoke_cmd::<BlobInfo>("resolve_blob", serde_json::json!({"blob_id": id_clone})).await {
                                Ok(i) => {
                                    info.set(Some(i));
                                }
                                Err(e) => err.set(Some(e)),
                            }
                        });
                    },
                    style: "margin-top: 8px;",
                    "Fetch"
                }
                if let Some(ref i) = blob_info {
                    div { style: "margin-top: 12px;",
                        p { b { "Blob: " } code { "{i.blob_id}" } }
                        p { b { "Size: " } "{i.size} bytes" }
                        p { b { "Source: " } "{i.source}" }
                        p { b { "Cached: " } if i.cached { "yes" } else { "no" } }
                    }
                }
                if let Some(ref e) = *err.read() {
                    p { style: "color: red;", "{e}" }
                }
            }
            div { class: "card",
                h3 { "Preview" }
                if let Some(ref i) = blob_info {
                    pre { style: "white-space: pre-wrap; word-break: break-all; font-size: 0.8rem; max-height: 400px; overflow-y: auto;",
                        "{i.content}"
                    }
                    if i.size > 2000 {
                        p { style: "font-size: 0.8rem; color: #888;",
                            "{i.size} bytes total (showing first 2000 characters)"
                        }
                    }
                } else {
                    p { style: "color: #999;", "Fetch a blob to preview content" }
                }
            }
        }
    }
}
