use dioxus::prelude::*;
use crate::ipc;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{EventSource, MessageEvent, Event};

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
    let mut loading = use_signal(|| false);
    let mut status = use_signal(|| String::new());
    let mut layer = use_signal(|| String::new());

    let blob_info = info.read().clone();
    let is_image = blob_info.as_ref().map(|i| i.mime_type.starts_with("image/")).unwrap_or(false);
    let img_src = blob_info.as_ref().map(|i| {
        if i.mime_type.starts_with("image/") && !i.data_base64.is_empty() {
            format!("data:{};base64,{}", i.mime_type, i.data_base64)
        } else {
            String::new()
        }
    }).unwrap_or_default();

    let status_text = status.read().clone();
    let layer_text = layer.read().clone();
    let loading_val = *loading.read();

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
                        info.set(None);
                        loading.set(true);
                        status.set("Starting...".into());
                        layer.set("".into());

                        let id_clone = id.clone();
                        spawn(async move {
                            let url = format!("{}/api/resolve_blob_stream?blob_id={}", ipc::base_url(), id_clone);
                            let es = match EventSource::new(&url) {
                                Ok(es) => es,
                                Err(e) => {
                                    err.set(Some(format!("Failed to connect: {e:?}")));
                                    loading.set(false);
                                    return;
                                }
                            };

                            let es_close = es.clone();
                            {
                                let es2 = es.clone();
                                let onerror = Closure::wrap(Box::new(move |_e: Event| {
                                    if es2.ready_state() == EventSource::CLOSED {
                                        let _ = es2.close();
                                    }
                                }) as Box<dyn FnMut(Event)>);
                                es.set_onerror(Some(onerror.as_ref().unchecked_ref()));
                                onerror.forget();
                            }

                            let onmessage = Closure::wrap(Box::new(move |e: MessageEvent| {
                                let raw = match e.data().as_string() {
                                    Some(s) => s,
                                    None => return,
                                };
                                let v: serde_json::Value = match serde_json::from_str(&raw) {
                                    Ok(v) => v,
                                    Err(_) => return,
                                };
                                let typ = v["type"].as_str().unwrap_or("");
                                match typ {
                                    "status" => {
                                        status.set(v["status"].as_str().unwrap_or("").into());
                                        layer.set(v["layer"].as_str().unwrap_or("").into());
                                    }
                                    "result" => {
                                        if let Ok(blob_info) = serde_json::from_value::<BlobInfo>(v["info"].clone()) {
                                            info.set(Some(blob_info));
                                        }
                                        loading.set(false);
                                        let _ = es_close.close();
                                    }
                                    "error" => {
                                        err.set(Some(v["error"].as_str().unwrap_or("").into()));
                                        loading.set(false);
                                        let _ = es_close.close();
                                    }
                                    _ => {}
                                }
                            }) as Box<dyn FnMut(MessageEvent)>);
                            es.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
                            onmessage.forget();
                        });
                    },
                    style: "margin-top: 8px;",
                    "Fetch"
                }

                if loading_val {
                    div { style: "margin-top: 12px; padding: 12px; background: #f0f4ff; border-radius: 6px;",
                        p { style: "margin: 0 0 4px 0; font-weight: 600;", "Fetching..." }
                        p { style: "margin: 0; font-size: 0.85rem; color: #555;",
                            "{status_text}"
                        }
                        if !layer_text.is_empty() {
                            p { style: "margin: 4px 0 0 0; font-size: 0.8rem; color: #888;",
                                "Layer: {layer_text}"
                            }
                        }
                    }
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
                if loading_val {
                    p { style: "color: #999;", "Loading preview..." }
                } else if is_image && !img_src.is_empty() {
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
