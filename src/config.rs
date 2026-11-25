use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

/// Create (truncate) a file readable and writable by the owner only.
fn open_private(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Create the config directory, owner-only on Unix.
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Application configuration, persisted as TOML.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub connections: Vec<SavedConnection>,
    #[serde(default)]
    pub settings: AppSettings,
    /// Set when the file on disk could not be read or set aside; saving would
    /// overwrite the user's connections, so it is refused.
    #[serde(skip)]
    save_blocked: Option<String>,
}

/// Result of loading the config: the config, plus a message for the user when
/// the file on disk could not be used as-is.
pub struct LoadedConfig {
    pub config: AppConfig,
    pub warning: Option<String>,
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
    #[serde(default = "default_auto_refresh_secs")]
    pub auto_refresh_secs: u64,
    pub log_to_file: bool,
}

pub fn default_auto_refresh_secs() -> u64 {
    30
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            peek_count: 25,
            auto_refresh_secs: 30,
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

    /// Load config from disk. A missing file yields defaults.
    ///
    /// A file that cannot be parsed is renamed aside (never silently replaced),
    /// so the next save cannot wipe the user's saved connections. If it cannot
    /// even be moved, saving is disabled for the session.
    pub fn load() -> LoadedConfig {
        Self::load_from(&Self::config_path())
    }

    fn load_from(path: &Path) -> LoadedConfig {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return LoadedConfig {
                    config: Self::default(),
                    warning: None,
                };
            }
            Err(e) => {
                let reason = format!(
                    "Could not read {} ({}); saving is disabled",
                    path.display(),
                    e
                );
                return LoadedConfig {
                    config: Self::blocked(reason.clone()),
                    warning: Some(reason),
                };
            }
        };

        match toml::from_str::<AppConfig>(&content) {
            Ok(config) => LoadedConfig {
                config,
                warning: None,
            },
            Err(parse_err) => {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let backup = path.with_extension(format!("toml.broken-{}", stamp));
                match std::fs::rename(path, &backup) {
                    Ok(()) => LoadedConfig {
                        config: Self::default(),
                        warning: Some(format!(
                            "Config could not be parsed ({}); moved it to {} and started fresh",
                            first_line(&parse_err.to_string()),
                            backup.display()
                        )),
                    },
                    Err(e) => {
                        let reason = format!(
                            "Config could not be parsed or moved aside ({}); saving is disabled",
                            e
                        );
                        LoadedConfig {
                            config: Self::blocked(reason.clone()),
                            warning: Some(reason),
                        }
                    }
                }
            }
        }
    }

    fn blocked(reason: String) -> Self {
        Self {
            save_blocked: Some(reason),
            ..Self::default()
        }
    }

    /// Save config to disk.
    ///
    /// Written to a temporary file and renamed into place, so a crash mid-write
    /// cannot leave a truncated file. The file holds SAS keys, so on Unix it is
    /// created readable by the owner only.
    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(ref reason) = self.save_blocked {
            anyhow::bail!("{}", reason);
        }
        self.save_to(&Self::config_path())
    }

    fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        {
            let mut file = open_private(&tmp)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, path)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_auto_refresh_secs_is_30() {
        let settings = AppSettings::default();
        assert_eq!(settings.auto_refresh_secs, 30);
    }

    #[test]
    fn auto_refresh_secs_zero_means_disabled() {
        let settings = AppSettings {
            auto_refresh_secs: 0,
            ..Default::default()
        };
        assert_eq!(settings.auto_refresh_secs, 0);
    }

    #[test]
    fn deserialize_auto_refresh_from_toml() {
        let toml_str = r#"
            [settings]
            peek_count = 25
            auto_refresh_secs = 60
            log_to_file = false
        "#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.settings.auto_refresh_secs, 60);
    }

    #[test]
    fn deserialize_zero_auto_refresh_secs() {
        let toml_str = r#"
            [settings]
            peek_count = 25
            auto_refresh_secs = 0
            log_to_file = false
        "#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.settings.auto_refresh_secs, 0);
    }

    #[test]
    fn deserialize_missing_auto_refresh_uses_default() {
        let toml_str = r#"
            [settings]
            peek_count = 25
            log_to_file = false
        "#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.settings.auto_refresh_secs, 30);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sb-explorer-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn unparseable_config_is_moved_aside_not_overwritten() {
        let dir = temp_dir("broken");
        let path = dir.join("config.toml");
        std::fs::write(&path, "connections = [ this is not toml").expect("write");

        let loaded = AppConfig::load_from(&path);
        assert!(loaded.warning.is_some());
        assert!(loaded.config.connections.is_empty());
        assert!(!path.exists(), "broken file moved away");
        let backups: Vec<_> = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("broken-"))
            .collect();
        assert_eq!(backups.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_round_trips_and_is_owner_only() {
        let dir = temp_dir("save");
        let path = dir.join("nested").join("config.toml");
        let mut config = AppConfig::default();
        config.add_connection(
            "prod".into(),
            "Endpoint=sb://x/;SharedAccessKeyName=k;SharedAccessKey=s".into(),
        );
        config.save_to(&path).expect("save");

        let loaded = AppConfig::load_from(&path);
        assert!(loaded.warning.is_none());
        assert_eq!(loaded.config.connections.len(), 1);
        assert!(!path.with_extension("toml.tmp").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn blocked_config_refuses_to_save() {
        let config = AppConfig::blocked("unreadable".into());
        assert!(config.save().is_err());
    }
}
