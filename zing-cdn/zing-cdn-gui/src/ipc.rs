use wasm_bindgen::prelude::*;
use serde::Serialize;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;

    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

fn is_tauri_available() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("__TAURI__"))
        .map(|v| !v.is_undefined())
        .unwrap_or(false)
}

fn try_log(msg: &str) {
    if is_tauri_available() {
        log(msg);
    }
}

pub async fn invoke_cmd<T: serde::de::DeserializeOwned>(
    cmd: &str,
    args: impl Serialize,
) -> Result<T, String> {
    if !is_tauri_available() {
        let msg = format!("zing-cdn: Tauri IPC not available (run inside Tauri window)");
        return Err(msg);
    }
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| {
        let msg = format!("zing-cdn: serialize error: {e}");
        try_log(&msg);
        msg
    })?;
    try_log(&format!("zing-cdn: invoke {cmd}"));
    let result = invoke(cmd, args).await;
    serde_wasm_bindgen::from_value::<T>(result).map_err(|e| {
        let msg = format!("zing-cdn: deserialize error: {e}");
        try_log(&msg);
        msg
    })
}

pub async fn invoke_void(cmd: &str, args: impl Serialize) -> Result<(), String> {
    if !is_tauri_available() {
        return Err("zing-cdn: Tauri IPC not available (run inside Tauri window)".into());
    }
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| format!("zing-cdn: serialize error: {e}"))?;
    try_log(&format!("zing-cdn: invoke {cmd}"));
    invoke(cmd, args).await;
    Ok(())
}
