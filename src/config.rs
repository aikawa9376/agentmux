use serde::Deserialize;
use std::{env, fs, path::PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub ui: UiConfig,
    pub refresh: RefreshConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub workspace_ratio: f32,
    pub preview_min_width: u16,
    pub show_plain_panes: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RefreshConfig {
    pub preview_ms: u64,
    pub status_ms: u64,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: Config,
    pub warning: Option<String>,
    pub path: PathBuf,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            workspace_ratio: 0.38,
            preview_min_width: 120,
            show_plain_panes: false,
        }
    }
}

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            preview_ms: 50,
            status_ms: 250,
        }
    }
}

impl LoadedConfig {
    pub fn load() -> Self {
        let path = config_path();
        let Some(contents) = fs::read_to_string(&path).ok() else {
            return Self {
                config: Config::default(),
                warning: None,
                path,
            };
        };
        match toml::from_str::<Config>(&contents) {
            Ok(mut config) => {
                config.ui.workspace_ratio = config.ui.workspace_ratio.clamp(0.2, 0.75);
                config.refresh.preview_ms = config.refresh.preview_ms.clamp(16, 1_000);
                config.refresh.status_ms = config.refresh.status_ms.clamp(50, 5_000);
                Self {
                    config,
                    warning: None,
                    path,
                }
            }
            Err(error) => Self {
                config: Config::default(),
                warning: Some(format!(
                    "{} is invalid; using defaults: {error}",
                    path.display()
                )),
                path,
            },
        }
    }

    pub fn default_toml() -> &'static str {
        r#"[ui]
workspace_ratio = 0.38
preview_min_width = 120
show_plain_panes = false

[refresh]
preview_ms = 50
status_ms = 250
"#
    }
}

fn config_path() -> PathBuf {
    if let Some(path) = env::var_os("AGENTMUX_CONFIG_PATH") {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("agentmux/config.toml");
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/agentmux/config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_config() {
        let config: Config = toml::from_str(LoadedConfig::default_toml()).unwrap();
        assert_eq!(config.refresh.status_ms, 250);
        assert!(!config.ui.show_plain_panes);
    }
}
