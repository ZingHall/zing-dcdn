use wasm_bindgen::prelude::*;
use serde::Serialize;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
}

pub async fn invoke_cmd<T: serde::de::DeserializeOwned>(
    cmd: &str,
    args: impl Serialize,
) -> Result<T, String> {
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| e.to_string())?;
    let result = invoke(cmd, args).await;
    serde_wasm_bindgen::from_value::<T>(result).map_err(|e| e.to_string())
}

pub async fn invoke_void(cmd: &str, args: impl Serialize) -> Result<(), String> {
    let args = serde_wasm_bindgen::to_value(&args).map_err(|e| e.to_string())?;
    invoke(cmd, args).await;
    Ok(())
}
