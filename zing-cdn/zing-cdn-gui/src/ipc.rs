use serde::{Serialize, de::DeserializeOwned};
use gloo_net::http::Request;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

const API_PORT: u16 = 13420;

fn get_api_port() -> u16 {
    if let Some(port) = js_sys::Reflect::get(
        &js_sys::global(),
        &wasm_bindgen::JsValue::from_str("ZING_API_PORT"),
    )
    .ok()
    .and_then(|v| v.as_f64())
    .map(|v| v as u16)
    {
        if port > 0 {
            return port;
        }
    }
    if let Some(window) = web_sys::window() {
        if let Some(doc) = window.document() {
            if let Some(html) = doc.document_element() {
                if let Some(val) = html.get_attribute("data-api-port") {
                    if let Ok(p) = val.parse::<u16>() {
                        if p > 0 {
                            return p;
                        }
                    }
                }
            }
        }
    }
    API_PORT
}

pub fn base_url() -> String {
    format!("http://127.0.0.1:{}", get_api_port())
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

    let raw = response
        .text()
        .await
        .map_err(|e| format!("http error: {e}"))?;

    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("json error: {e}"))?;

    if let Some(msg) = value.get("error").and_then(|v| v.as_str()) {
        return Err(msg.to_string());
    }

    serde_json::from_value::<T>(value)
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
    pub wallet_address: Option<String>,
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

#[derive(serde::Deserialize, Clone)]
pub struct PeersInfo {
    pub bootstrap: Vec<String>,
    pub connected: Vec<String>,
    pub listen_addr: String,
    pub cache_dir: String,
    pub p2p_addr: String,
}

pub async fn list_peers() -> Result<PeersInfo, String> {
    invoke_cmd::<PeersInfo>("peers", {}).await
}

pub async fn add_peer(addr: &str) -> Result<(), String> {
    let url = format!("{}/api/peers/add", base_url());
    let body = serde_json::json!({"addr": addr});
    Request::post(&url)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&body).map_err(|e| e.to_string())?)
        .map_err(|e| format!("http error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;
    Ok(())
}

pub async fn remove_peer(addr: &str) -> Result<(), String> {
    let url = format!("{}/api/peers/remove", base_url());
    let body = serde_json::json!({"addr": addr});
    Request::post(&url)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&body).map_err(|e| e.to_string())?)
        .map_err(|e| format!("http error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;
    Ok(())
}

#[derive(serde::Deserialize, Clone)]
pub struct StakingPeerInfo {
    pub sui_address: String,
    pub peer_id_short: String,
    pub bond: u64,
    pub is_active: bool,
    pub is_live: bool,
}

pub async fn list_staking() -> Result<Vec<StakingPeerInfo>, String> {
    let url = format!("{}/api/staking", base_url());
    log(&format!("GET {url}"));
    let response = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;
    let raw = response.text().await.map_err(|e| format!("http error: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("json error: {e}"))?;
    if let Some(msg) = value.get("error").and_then(|v| v.as_str()) {
        return Err(msg.to_string());
    }
    serde_json::from_value::<Vec<StakingPeerInfo>>(value)
        .map_err(|e| format!("json error: {e}"))
}

#[derive(serde::Deserialize, Clone)]
pub struct MyPeerInfo {
    pub wallet_address: String,
    pub peer_id_short: Option<String>,
    pub bond: Option<u64>,
    pub is_active: Option<bool>,
    pub is_live: Option<bool>,
    pub is_registered: bool,
}

pub async fn get_my_peer_info() -> Result<MyPeerInfo, String> {
    let url = format!("{}/api/my_peer", base_url());
    log(&format!("GET {url}"));
    let response = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;
    let raw = response.text().await.map_err(|e| format!("http error: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("json error: {e}"))?;
    if let Some(msg) = value.get("error").and_then(|v| v.as_str()) {
        return Err(msg.to_string());
    }
    serde_json::from_value::<MyPeerInfo>(value)
        .map_err(|e| format!("json error: {e}"))
}

#[derive(serde::Deserialize, Clone)]
pub struct WalBalance {
    pub balance: u64,
    pub balance_wal: String,
}

pub async fn get_wal_balance() -> Result<WalBalance, String> {
    let url = format!("{}/api/balance", base_url());
    log(&format!("GET {url}"));
    let response = Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;
    let raw = response.text().await.map_err(|e| format!("http error: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("json error: {e}"))?;
    if let Some(msg) = value.get("error").and_then(|v| v.as_str()) {
        return Err(msg.to_string());
    }
    serde_json::from_value::<WalBalance>(value)
        .map_err(|e| format!("json error: {e}"))
}

pub async fn register_peer() -> Result<String, String> {
    let url = format!("{}/api/register", base_url());
    log(&format!("POST {url}"));
    let response = Request::post(&url)
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;
    let raw = response.text().await.map_err(|e| format!("http error: {e}"))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("json error: {e}"))?;
    if let Some(msg) = value.get("error").and_then(|v| v.as_str()) {
        return Err(msg.to_string());
    }
    value.get("message")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "No message in response".to_string())
}
