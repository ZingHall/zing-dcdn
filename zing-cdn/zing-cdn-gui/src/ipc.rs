use serde::{Serialize, de::DeserializeOwned};
use gloo_net::http::Request;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

const API_PORT: u16 = 13420;

fn base_url() -> String {
    format!("http://127.0.0.1:{API_PORT}")
}

pub async fn invoke_cmd<T: DeserializeOwned>(
    cmd: &str,
    args: impl Serialize,
) -> Result<T, String> {
    let args_value = serde_json::to_value(&args).map_err(|e| e.to_string())?;
    let mut query = String::new();
    if let Some(obj) = args_value.as_object() {
        for (i, (k, v)) in obj.iter().enumerate() {
            if i == 0 { query.push('?'); } else { query.push('&'); }
            query.push_str(k);
            query.push('=');
            // URL-encode the value (simple: just use the string representation)
            let val = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            query.push_str(&val);
        }
    }
    let url = format!("{}/api/{}{query}", base_url(), cmd);

    log(&format!("GET {url}"));
    let response = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;

    response
        .json::<T>()
        .await
        .map_err(|e| format!("json error: {e}"))
}

// Dashboard specific endpoint
pub async fn get_dashboard() -> Result<DashboardInfo, String> {
    invoke_cmd::<DashboardInfo>("dashboard", {}).await
}

// Cache-specific endpoints
pub async fn list_cache() -> Result<Vec<CacheEntry>, String> {
    invoke_cmd::<Vec<CacheEntry>>("cache", {}).await
}

pub async fn pin_blob(blob_id: &str) -> Result<(), String> {
    let url = format!("{}/api/pin?blob_id={}", base_url(), blob_id);
    log(&format!("GET {url}"));
    Request::get(&url).send().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn unpin_blob(blob_id: &str) -> Result<(), String> {
    let url = format!("{}/api/unpin?blob_id={}", base_url(), blob_id);
    Request::get(&url).send().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn delete_blob(blob_id: &str) -> Result<(), String> {
    let url = format!("{}/api/delete?blob_id={}", base_url(), blob_id);
    Request::get(&url).send().await.map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(serde::Deserialize, Clone)]
pub struct DashboardInfo {
    pub peer_id: String,
    pub listen_addr: String,
    pub connected_peers: Vec<String>,
    pub cache_used: u64,
    pub cache_budget: u64,
    pub cache_count: usize,
}

#[derive(serde::Deserialize, Clone)]
pub struct CacheEntry {
    pub blob_id: String,
    pub size: u64,
    pub pinned: bool,
}
