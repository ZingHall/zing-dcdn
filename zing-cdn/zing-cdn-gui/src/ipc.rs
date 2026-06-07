use wasm_bindgen::prelude::*;
use serde::Serialize;
use js_sys::Reflect;

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

pub fn is_tauri_available() -> bool {
    let result = std::panic::catch_unwind(|| {
        if let Some(window) = web_sys::window() {
            match Reflect::get(&window, &JsValue::from_str("__TAURI__")) {
                Ok(val) => {
                    let available = !val.is_undefined();
                    if !available {
                        console_log("__TAURI__ found but is undefined");
                    }
                    available
                }
                Err(e) => {
                    console_log(&format!("Reflect error: {e:?}"));
                    false
                }
            }
        } else {
            console_log("no window object");
            false
        }
    });
    result.unwrap_or(false)
}

pub async fn invoke_cmd<T: serde::de::DeserializeOwned>(
    cmd: &str,
    args: impl Serialize,
) -> Result<T, String> {
    if !is_tauri_available() {
        console_log("__TAURI__ not available");
        return Err("Internal connection not ready — wait a moment then retry".into());
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
    if !is_tauri_available() {
        return Err("Internal connection not ready — wait a moment then retry".into());
    }
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| {
        console_log(&format!("serialize error: {e}"));
        e.to_string()
    })?;
    console_log(&format!("invoke: {cmd}"));
    invoke(cmd, args).await;
    Ok(())
}
