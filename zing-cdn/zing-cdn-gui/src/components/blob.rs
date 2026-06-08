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
    mime_type: String,
    data_base64: String,
}

#[component]
pub fn BlobBrowser() -> Element {
    let mut input = use_signal(|| String::new());
    let mut info = use_signal(|| None::<BlobInfo>);
    let mut err = use_signal(|| None::<String>);

    let blob_info = info.read().clone();

    let is_image = blob_info.as_ref().map(|i| i.mime_type.starts_with("image/")).unwrap_or(false);
    let img_src = blob_info.as_ref().map(|i| {
        if i.mime_type.starts_with("image/") && !i.data_base64.is_empty() {
            format!("data:{};base64,{}", i.mime_type, i.data_base64)
        } else {
            String::new()
        }
    }).unwrap_or_default();

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
                if is_image && !img_src.is_empty() {
                    img {
                        src: "{img_src}",
                        style: "max-width: 100%; max-height: 500px; border-radius: 6px;",
                        alt: "Blob image preview"
                    }
                } else if let Some(ref i) = blob_info {
                    pre { style: "white-space: pre-wrap; word-break: break-all; font-size: 0.8rem; max-height: 400px; overflow-y: auto;",
                        "{i.content}"
                    }
                    if i.size > 2000 {
                        p { style: "font-size: 0.8rem; color: #888;",
                            "{i.size} bytes total"
                        }
                    }
                } else {
                    p { style: "color: #999;", "Fetch a blob to preview content" }
                }
            }
        }
    }
}
