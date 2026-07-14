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
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: 1,
            theme: ThemePreference::System,
            language: LanguagePreference::Automatic,
            update_channel: UpdateChannel::Stable,
        }
    }
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
