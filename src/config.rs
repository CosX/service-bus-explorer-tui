use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application configuration, persisted as TOML.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub connections: Vec<SavedConnection>,
    #[serde(default)]
    pub settings: AppSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConnection {
    pub name: String,
    /// SAS connection string. `None` for Azure AD connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_string: Option<String>,
    /// Fully-qualified namespace for Azure AD connections.
    /// E.g. `mynamespace.servicebus.windows.net`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Authentication type tag: "sas" (default) or "azure_ad".
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
}

fn default_auth_type() -> String {
    "sas".to_string()
}

impl SavedConnection {
    pub fn is_azure_ad(&self) -> bool {
        self.auth_type == "azure_ad"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub peek_count: i32,
    pub auto_refresh_secs: u64,
    pub log_to_file: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            peek_count: 25,
            auto_refresh_secs: 0, // 0 = disabled
            log_to_file: false,
        }
    }
}

impl AppConfig {
    /// Standard config file path: ~/.config/sb-explorer/config.toml
    pub fn config_path() -> PathBuf {
        let base = dirs_fallback();
        base.join("sb-explorer").join("config.toml")
    }

    /// Load config from disk. Returns default if file doesn't exist.
    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            toml::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    /// Save config to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn add_connection(&mut self, name: String, connection_string: String) {
        // Remove existing with same name
        self.connections.retain(|c| c.name != name);
        self.connections.push(SavedConnection {
            name,
            connection_string: Some(connection_string),
            namespace: None,
            auth_type: "sas".to_string(),
        });
    }

    pub fn add_azure_ad_connection(&mut self, name: String, namespace: String) {
        self.connections.retain(|c| c.name != name);
        self.connections.push(SavedConnection {
            name,
            connection_string: None,
            namespace: Some(namespace),
            auth_type: "azure_ad".to_string(),
        });
    }

    pub fn remove_connection(&mut self, name: &str) {
        self.connections.retain(|c| c.name != name);
    }
}

/// Cross-platform config directory fallback.
fn dirs_fallback() -> PathBuf {
    // Try XDG_CONFIG_HOME, then platform defaults
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support");
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config");
        }
    }

    // Fallback to current dir
    PathBuf::from(".")
}
