use dioxus::prelude::*;
use crate::ipc::invoke_cmd;

#[derive(serde::Deserialize, Clone)]
struct BlobInfo {
    blob_id: String,
    size: u64,
    source: String,
    cached: bool,
}

#[component]
pub fn BlobBrowser() -> Element {
    let input = use_signal(|| String::new());
    let info = use_signal(|| None::<BlobInfo>);
    let data = use_signal(|| None::<Vec<u8>>);
    let err = use_signal(|| None::<String>);

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
                        spawn(async move {
                            match invoke_cmd::<BlobInfo>("resolve_blob", serde_json::json!({"blobId": id})).await {
                                Ok(i) => {
                                    info.set(Some(i.clone()));
                                    data.set(
                                        invoke_cmd::<Vec<u8>>("get_blob_content", serde_json::json!({"blobId": id})).await.ok()
                                    );
                                }
                                Err(e) => err.set(Some(e)),
                            }
                        });
                    },
                    style: "margin-top: 8px;",
                    "Fetch"
                }
                if let Some(ref i) = *info.read() {
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
                if let Some(ref d) = *data.read() {
                    let text = if d.len() > 2000 {
                        format!("{}...", String::from_utf8_lossy(&d[..2000]))
                    } else {
                        String::from_utf8_lossy(d).to_string()
                    };
                    pre { "{text}" }
                    p { style: "font-size: 0.8rem; color: #888;",
                        "{d.len()} bytes total"
                    }
                } else {
                    p { style: "color: #999;", "Fetch a blob to preview content" }
                }
            }
        }
    }
}
