use std::path::{Path, PathBuf};

/// Persistent zing-cdn configuration, loaded from ~/.zing-cdn/config.toml.
/// CLI flags override config file values.
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct ZingConfig {
    #[serde(default)]
    pub settlement: Option<SettlementSection>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SettlementSection {
    pub package: Option<String>,
    pub settlement_object: Option<String>,
    pub vault_object: Option<String>,
    pub registry_object: Option<String>,
    pub registry_peers_table: Option<String>,
    pub registry_version: Option<u64>,
    pub settlement_version: Option<u64>,
    pub vault_version: Option<u64>,
}

impl ZingConfig {
    /// Load from ~/.zing-cdn/config.toml. Returns default if file doesn't exist.
    pub fn load() -> Self {
        let path = Self::default_path();
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(".zing-cdn").join("config.toml")
    }
}
