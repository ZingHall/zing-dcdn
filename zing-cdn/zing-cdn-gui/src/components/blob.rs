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
    let mut input = use_signal(|| String::new());
    let mut info = use_signal(|| None::<BlobInfo>);
    let mut data = use_signal(|| None::<Vec<u8>>);
    let mut err = use_signal(|| None::<String>);

    let info_read = (*info.read()).clone();
    let data_read = (*data.read()).clone();
    let error_read = (*err.read()).clone();

    let preview_text = data_read.as_ref().map(|d| {
        if d.len() > 2000 {
            format!("{}...", String::from_utf8_lossy(&d[..2000]))
        } else {
            String::from_utf8_lossy(d).to_string()
        }
    });

    let total_len = data_read.as_ref().map(|d| d.len());

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
                            let id_clone = id.clone();
                            match invoke_cmd::<BlobInfo>("resolve_blob", serde_json::json!({"blobId": id_clone})).await {
                                Ok(i) => {
                                    info.set(Some(i.clone()));
                                    let content = invoke_cmd::<Vec<u8>>("get_blob_content", serde_json::json!({"blobId": id_clone})).await.ok();
                                    data.set(content);
                                }
                                Err(e) => err.set(Some(e)),
                            }
                        });
                    },
                    style: "margin-top: 8px;",
                    "Fetch"
                }
                if let Some(ref i) = info_read {
                    div { style: "margin-top: 12px;",
                        p { b { "Blob: " } code { "{i.blob_id}" } }
                        p { b { "Size: " } "{i.size} bytes" }
                        p { b { "Source: " } "{i.source}" }
                        p { b { "Cached: " } if i.cached { "yes" } else { "no" } }
                    }
                }
                if let Some(ref e) = error_read {
                    p { style: "color: red;", "{e}" }
                }
            }
            div { class: "card",
                h3 { "Preview" }
                if let Some(ref text) = preview_text {
                    pre { "{text}" }
                    if let Some(len) = total_len {
                        p { style: "font-size: 0.8rem; color: #888;", "{len} bytes total" }
                    }
                } else {
                    p { style: "color: #999;", "Fetch a blob to preview content" }
                }
            }
        }
    }
}
