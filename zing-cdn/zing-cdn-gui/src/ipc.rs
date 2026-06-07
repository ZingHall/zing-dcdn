use wasm_bindgen::prelude::*;
use serde::Serialize;
use js_sys::Reflect;
use gloo_timers::future::TimeoutFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;

    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

fn console_log(msg: &str) {
    std::panic::catch_unwind(|| log(msg)).ok();
}

/// Walk window.__TAURI__.core.invoke — each level must be defined and non-undefined.
fn is_tauri_available() -> bool {
    let ok = std::panic::catch_unwind(|| -> bool {
        let window = match web_sys::window() {
            Some(w) => w,
            None => return false,
        };
        let tauri = match Reflect::get(&window, &JsValue::from_str("__TAURI__")) {
            Ok(v) if !v.is_undefined() => v,
            _ => return false,
        };
        let core = match Reflect::get(&tauri, &JsValue::from_str("core")) {
            Ok(v) if !v.is_undefined() => v,
            _ => return false,
        };
        let invoke_fn = match Reflect::get(&core, &JsValue::from_str("invoke")) {
            Ok(v) if !v.is_undefined() => v,
            _ => return false,
        };
        invoke_fn.is_function()
    });
    ok.unwrap_or(false)
}

/// Poll up to ~3s (30 × 100ms) until Tauri IPC is ready.
async fn await_tauri() {
    for _ in 0..30 {
        if is_tauri_available() {
            return;
        }
        TimeoutFuture::new(100).await;
    }
    console_log("await_tauri: timeout after 3s");
}

pub async fn invoke_cmd<T: serde::de::DeserializeOwned>(
    cmd: &str,
    args: impl Serialize,
) -> Result<T, String> {
    await_tauri().await;
    if !is_tauri_available() {
        return Err("Tauri IPC not available — wait a moment then retry".into());
    }
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| {
        let msg = format!("serialize error: {e}");
        console_log(&msg);
        msg
    })?;
    console_log(&format!("invoke: {cmd}"));
    let result = invoke(cmd, args).await;
    serde_wasm_bindgen::from_value::<T>(result).map_err(|e| {
        let msg = format!("deserialize error: {e}");
        console_log(&msg);
        msg
    })
}

pub async fn invoke_void(cmd: &str, args: impl Serialize) -> Result<(), String> {
    await_tauri().await;
    if !is_tauri_available() {
        return Err("Tauri IPC not available — wait a moment then retry".into());
    }
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| {
        console_log(&format!("serialize error: {e}"));
        e.to_string()
    })?;
    console_log(&format!("invoke: {cmd}"));
    invoke(cmd, args).await;
    Ok(())
}
