use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub version: u32,
    pub theme: ThemePreference,
    #[serde(default = "default_language")]
    pub language: LanguagePreference,
    #[serde(default = "default_update_channel")]
    pub update_channel: UpdateChannel,
    #[serde(default)]
    pub developer_mode: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            theme: ThemePreference::System,
            language: LanguagePreference::Automatic,
            update_channel: UpdateChannel::Stable,
            developer_mode: false,
        }
    }
}

pub fn developer_mode_forced() -> bool {
    cfg!(debug_assertions)
}

pub fn developer_mode_enabled(config: &AppConfig) -> bool {
    developer_mode_forced() || config.developer_mode
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemePreference {
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguagePreference {
    Automatic,
    En,
    De,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpdateChannel {
    Stable,
    Beta,
}

fn default_language() -> LanguagePreference {
    LanguagePreference::Automatic
}

fn default_update_channel() -> UpdateChannel {
    UpdateChannel::Stable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn developer_mode_defaults_to_opt_in_outside_forced_builds() {
        let config = AppConfig::default();

        assert!(!config.developer_mode);
        assert_eq!(developer_mode_enabled(&config), developer_mode_forced());
    }

    #[test]
    fn persisted_developer_mode_is_enabled() {
        let config = AppConfig {
            developer_mode: true,
            ..AppConfig::default()
        };

        assert!(developer_mode_enabled(&config));
    }
}
