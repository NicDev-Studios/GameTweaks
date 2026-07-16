use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::redirect::Policy;
use reqwest::{Client, StatusCode, Url};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tempfile::{Builder as TempBuilder, NamedTempFile};
use tokio::io::AsyncWriteExt;
use zip::ZipArchive;

use crate::agent::{self, AgentConnectionStatus};
use crate::bepinex::{
    analyze_windows_installation, ensure_game_stopped, resolve_game, valid_install_marker,
    BepInExArchitecture, BepInExAvailability, BepInExRuntime, InstallTarget,
};
use crate::core::error::{AppError, AppResult, ErrorResponse};
use crate::core::state::AppState;

const CATALOG_RAW_ROOT: &str =
    "https://raw.githubusercontent.com/NicDev-Studios/GameTweaks-Games/main";
const RELEASE_ROOT: &str = "https://github.com/NicDev-Studios/GameTweaks-Games/releases/download";
const CATALOG_MAX_BYTES: usize = 512 * 1024;
const MOD_ARCHIVE_MAX_BYTES: u64 = 64 * 1024 * 1024;
const MOD_ARCHIVE_MAX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const MOD_ARCHIVE_MAX_ENTRIES: usize = 1_000;
const CONFIG_MAX_BYTES: u64 = 2 * 1024 * 1024;
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);
const CACHE_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MOD_PROGRESS_EVENT: &str = "gametweaks-mod-install-progress";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalizedText {
    pub en: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub de: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModIntegration {
    Agent,
    ConfigFile,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConfigApplyMode {
    Live,
    RestartRequired,
    NextLaunch,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", tag = "control")]
pub enum ConfigField {
    Boolean {
        id: String,
        section: String,
        key: String,
        label: LocalizedText,
        #[serde(default)]
        description: Option<LocalizedText>,
        #[serde(default)]
        locked: bool,
        default: bool,
        apply_mode: ConfigApplyMode,
        #[serde(default)]
        display: BooleanDisplay,
    },
    String {
        id: String,
        section: String,
        key: String,
        label: LocalizedText,
        #[serde(default)]
        description: Option<LocalizedText>,
        #[serde(default)]
        locked: bool,
        default: String,
        max_length: u16,
        apply_mode: ConfigApplyMode,
    },
    Integer {
        id: String,
        section: String,
        key: String,
        label: LocalizedText,
        #[serde(default)]
        description: Option<LocalizedText>,
        #[serde(default)]
        locked: bool,
        default: i64,
        min: i64,
        max: i64,
        step: i64,
        apply_mode: ConfigApplyMode,
    },
    Decimal {
        id: String,
        section: String,
        key: String,
        label: LocalizedText,
        #[serde(default)]
        description: Option<LocalizedText>,
        #[serde(default)]
        locked: bool,
        default: f64,
        min: f64,
        max: f64,
        step: f64,
        apply_mode: ConfigApplyMode,
    },
    SingleSelect {
        id: String,
        section: String,
        key: String,
        label: LocalizedText,
        #[serde(default)]
        description: Option<LocalizedText>,
        #[serde(default)]
        locked: bool,
        default: String,
        options: Vec<SelectOption>,
        apply_mode: ConfigApplyMode,
        #[serde(default)]
        display: SelectionDisplay,
    },
    MultiSelect {
        id: String,
        section: String,
        key: String,
        label: LocalizedText,
        #[serde(default)]
        description: Option<LocalizedText>,
        #[serde(default)]
        locked: bool,
        default: Vec<String>,
        options: Vec<SelectOption>,
        apply_mode: ConfigApplyMode,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BooleanDisplay {
    #[default]
    Switch,
    Checkbox,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectionDisplay {
    #[default]
    Dropdown,
    Radio,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectOption {
    pub value: String,
    pub label: LocalizedText,
}

impl ConfigField {
    fn id(&self) -> &str {
        match self {
            Self::Boolean { id, .. }
            | Self::String { id, .. }
            | Self::Integer { id, .. }
            | Self::Decimal { id, .. }
            | Self::SingleSelect { id, .. }
            | Self::MultiSelect { id, .. } => id,
        }
    }

    pub(crate) fn id_for_agent(&self) -> &str {
        self.id()
    }

    pub(crate) fn is_locked(&self) -> bool {
        match self {
            Self::Boolean { locked, .. }
            | Self::String { locked, .. }
            | Self::Integer { locked, .. }
            | Self::Decimal { locked, .. }
            | Self::SingleSelect { locked, .. }
            | Self::MultiSelect { locked, .. } => *locked,
        }
    }

    pub(crate) fn set_locked(&mut self, value: bool) {
        match self {
            Self::Boolean { locked, .. }
            | Self::String { locked, .. }
            | Self::Integer { locked, .. }
            | Self::Decimal { locked, .. }
            | Self::SingleSelect { locked, .. }
            | Self::MultiSelect { locked, .. } => *locked = value,
        }
    }

    pub(crate) fn valid_for_agent(&self) -> bool {
        let (section, key) = self.section_key();
        !self.is_locked()
            && valid_identifier(self.id())
            && valid_config_name(section)
            && valid_config_name(key)
            && self.metadata_is_valid()
            && self.serialize_value(&self.default_value()).is_ok()
    }

    fn metadata_is_valid(&self) -> bool {
        let (label, description) = match self {
            Self::Boolean {
                label, description, ..
            }
            | Self::String {
                label, description, ..
            }
            | Self::Integer {
                label, description, ..
            }
            | Self::Decimal {
                label, description, ..
            }
            | Self::SingleSelect {
                label, description, ..
            }
            | Self::MultiSelect {
                label, description, ..
            } => (label, description),
        };
        if !valid_localized_text(label)
            || description
                .as_ref()
                .is_some_and(|description| !valid_localized_text(description))
        {
            return false;
        }
        match self {
            Self::Boolean { .. } => true,
            Self::String {
                default,
                max_length,
                ..
            } => *max_length > 0 && default.len() <= usize::from(*max_length),
            Self::Integer {
                default,
                min,
                max,
                step,
                ..
            } => {
                min <= max
                    && *step > 0
                    && default >= min
                    && default <= max
                    && default
                        .checked_sub(*min)
                        .is_some_and(|offset| offset % *step == 0)
            }
            Self::Decimal {
                default,
                min,
                max,
                step,
                ..
            } => {
                default.is_finite()
                    && min.is_finite()
                    && max.is_finite()
                    && step.is_finite()
                    && min <= max
                    && *step > 0.0
                    && default >= min
                    && default <= max
            }
            Self::SingleSelect {
                default, options, ..
            } => valid_options(options) && options.iter().any(|option| option.value == *default),
            Self::MultiSelect {
                default, options, ..
            } => {
                valid_options(options)
                    && default.len() <= options.len()
                    && default.iter().collect::<HashSet<_>>().len() == default.len()
                    && default
                        .iter()
                        .all(|value| options.iter().any(|option| option.value == *value))
            }
        }
    }

    fn section_key(&self) -> (&str, &str) {
        match self {
            Self::Boolean { section, key, .. }
            | Self::String { section, key, .. }
            | Self::Integer { section, key, .. }
            | Self::Decimal { section, key, .. }
            | Self::SingleSelect { section, key, .. }
            | Self::MultiSelect { section, key, .. } => (section, key),
        }
    }

    fn default_value(&self) -> Value {
        match self {
            Self::Boolean { default, .. } => Value::Bool(*default),
            Self::String { default, .. } | Self::SingleSelect { default, .. } => {
                Value::String(default.clone())
            }
            Self::Integer { default, .. } => Value::from(*default),
            Self::Decimal { default, .. } => Value::from(*default),
            Self::MultiSelect { default, .. } => Value::Array(
                default
                    .iter()
                    .map(|value| Value::String(value.clone()))
                    .collect(),
            ),
        }
    }

    fn serialize_value(&self, value: &Value) -> AppResult<String> {
        match self {
            Self::Boolean { .. } => value
                .as_bool()
                .map(|value| value.to_string())
                .ok_or_else(|| mod_error("mod_config_invalid", "a Boolean setting was invalid")),
            Self::String { max_length, .. } => {
                let value = value.as_str().ok_or_else(|| {
                    mod_error("mod_config_invalid", "a string setting was invalid")
                })?;
                if value.len() > usize::from(*max_length)
                    || value
                        .chars()
                        .any(|character| matches!(character, '\r' | '\n'))
                    || value.contains('\0')
                {
                    return Err(mod_error(
                        "mod_config_invalid",
                        "a string setting exceeded its safe limits",
                    ));
                }
                Ok(value.to_owned())
            }
            Self::Integer { min, max, step, .. } => {
                let value = value.as_i64().ok_or_else(|| {
                    mod_error("mod_config_invalid", "an integer setting was invalid")
                })?;
                if value < *min || value > *max || *step <= 0 || (value - *min) % *step != 0 {
                    return Err(mod_error(
                        "mod_config_invalid",
                        "an integer setting was outside its allowed range",
                    ));
                }
                Ok(value.to_string())
            }
            Self::Decimal { min, max, step, .. } => {
                let value = value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        mod_error("mod_config_invalid", "a decimal setting was invalid")
                    })?;
                if value < *min || value > *max || !step.is_finite() || *step <= 0.0 {
                    return Err(mod_error(
                        "mod_config_invalid",
                        "a decimal setting was outside its allowed range",
                    ));
                }
                Ok(value.to_string())
            }
            Self::SingleSelect { options, .. } => {
                let value = value.as_str().ok_or_else(|| {
                    mod_error("mod_config_invalid", "a selection setting was invalid")
                })?;
                options
                    .iter()
                    .any(|option| option.value == value)
                    .then(|| value.to_owned())
                    .ok_or_else(|| {
                        mod_error("mod_config_invalid", "a selection value was not allowed")
                    })
            }
            Self::MultiSelect { options, .. } => {
                let values = value.as_array().ok_or_else(|| {
                    mod_error(
                        "mod_config_invalid",
                        "a multi-selection setting was invalid",
                    )
                })?;
                let mut parsed = Vec::with_capacity(values.len());
                for value in values {
                    let value = value.as_str().ok_or_else(|| {
                        mod_error("mod_config_invalid", "a multi-selection value was invalid")
                    })?;
                    if !options.iter().any(|option| option.value == value)
                        || parsed.contains(&value)
                    {
                        return Err(mod_error(
                            "mod_config_invalid",
                            "a multi-selection value was not allowed",
                        ));
                    }
                    parsed.push(value);
                }
                Ok(parsed.join(","))
            }
        }
    }
}

fn valid_options(options: &[SelectOption]) -> bool {
    !options.is_empty()
        && options.len() <= 256
        && options.iter().all(|option| {
            !option.value.is_empty()
                && option.value.len() <= 128
                && !option
                    .value
                    .chars()
                    .any(|character| matches!(character, '\0' | '\r' | '\n'))
                && valid_localized_text(&option.label)
        })
        && options
            .iter()
            .map(|option| &option.value)
            .collect::<HashSet<_>>()
            .len()
            == options.len()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameCatalog {
    schema_version: u32,
    app_id: u32,
    name: LocalizedText,
    mods: Vec<ModReference>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModReference {
    mod_id: String,
    file: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModDependency {
    pub mod_id: String,
    pub minimum_version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModRelease {
    repository: String,
    tag: String,
    asset: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModCompatibility {
    runtime: BepInExRuntime,
    architectures: Vec<BepInExArchitecture>,
    #[serde(default)]
    minimum_agent_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModDefinition {
    schema_version: u32,
    mod_id: String,
    guid: String,
    version: String,
    official: bool,
    name: LocalizedText,
    description: LocalizedText,
    integration: ModIntegration,
    compatibility: ModCompatibility,
    release: ModRelease,
    #[serde(default)]
    dependencies: Vec<ModDependency>,
    #[serde(default)]
    conflicts: Vec<String>,
    #[serde(default)]
    config: Vec<ConfigField>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GameSupportStatus {
    Supported,
    Unsupported,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GameModStatus {
    NotInstalled,
    Installed,
    UpdateAvailable,
    Blocked,
    External,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameMod {
    pub mod_id: String,
    pub guid: String,
    pub version: String,
    pub installed_version: Option<String>,
    pub official: bool,
    pub external: bool,
    pub name: LocalizedText,
    pub description: LocalizedText,
    pub integration: ModIntegration,
    pub status: GameModStatus,
    pub dependencies: Vec<ModDependency>,
    pub conflicts: Vec<String>,
    pub config: Vec<ConfigField>,
    pub values: HashMap<String, Value>,
    pub restart_required: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameSupport {
    pub app_id: u32,
    pub status: GameSupportStatus,
    pub name: Option<LocalizedText>,
    pub mods: Vec<GameMod>,
    pub agent_status: AgentConnectionStatus,
    pub cached: bool,
}

#[derive(Clone)]
struct CachedCatalog {
    catalog: GameCatalog,
    definitions: HashMap<String, ModDefinition>,
    expires_at: Instant,
}

#[derive(Clone)]
enum PreparedAction {
    Install {
        app_id: u32,
        mod_ids: Vec<String>,
        required_mod_ids: Vec<String>,
        replace_existing: Vec<String>,
    },
    UninstallMod {
        app_id: u32,
        mod_id: String,
        remove_config: bool,
    },
    UninstallBepInEx {
        app_id: u32,
    },
}

#[derive(Clone)]
struct PreparedPlan {
    action: PreparedAction,
    expires_at: Instant,
}

#[derive(Default)]
pub struct GameModsState {
    cache: HashMap<u32, CachedCatalog>,
    pending: HashMap<String, PreparedPlan>,
    busy_games: HashSet<u32>,
    pub(crate) restart_required: HashSet<(u32, String)>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModActionPlan {
    pub plan_id: String,
    pub app_id: u32,
    pub mod_ids: Vec<String>,
    pub installs_agent: bool,
    pub action: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BepInExUninstallPlan {
    pub plan_id: String,
    pub app_id: u32,
    pub version: String,
    pub additional_file_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModInstallMarker {
    schema_version: u32,
    app_id: u32,
    mod_id: String,
    guid: String,
    version: String,
    sha256: String,
    files: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum ModProgressStage {
    Downloading,
    Verifying,
    Installing,
    Completed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModProgress {
    app_id: u32,
    mod_id: String,
    stage: ModProgressStage,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percentage: Option<u64>,
}

pub async fn get_support(app: &AppHandle, state: &AppState, app_id: u32) -> AppResult<GameSupport> {
    let game = resolve_game(app_id).await?;
    match load_catalog(app, state, app_id).await {
        Ok((catalog, definitions, cached)) => {
            let target = analyze_windows_installation(&game.install_directory)
                .ok()
                .map(|(_, target)| target);
            let mods = definitions
                .values()
                .map(|definition| describe_mod(target.as_ref(), app_id, definition))
                .collect::<AppResult<Vec<_>>>()?;
            let mut mods = mods;
            agent::overlay_runtime_mods(state, app_id, &mut mods).await;
            mods.sort_by(|left, right| {
                left.name
                    .en
                    .to_lowercase()
                    .cmp(&right.name.en.to_lowercase())
            });
            mods.extend(agent::external_mods(state, app_id).await);
            apply_restart_flags(state, app_id, &mut mods).await;
            Ok(GameSupport {
                app_id,
                status: GameSupportStatus::Supported,
                name: Some(catalog.name),
                mods,
                agent_status: agent::connection_status(state, app_id).await,
                cached,
            })
        }
        Err(error) if error.code == "game_not_supported" => {
            support_without_catalog(state, app_id, GameSupportStatus::Unsupported).await
        }
        Err(_) => support_without_catalog(state, app_id, GameSupportStatus::Unavailable).await,
    }
}

async fn support_without_catalog(
    state: &AppState,
    app_id: u32,
    status: GameSupportStatus,
) -> AppResult<GameSupport> {
    let mut mods = agent::external_mods(state, app_id).await;
    apply_restart_flags(state, app_id, &mut mods).await;
    Ok(GameSupport {
        app_id,
        status,
        name: None,
        mods,
        agent_status: agent::connection_status(state, app_id).await,
        cached: false,
    })
}

async fn apply_restart_flags(state: &AppState, app_id: u32, mods: &mut [GameMod]) {
    let game_mods = state.game_mods.lock().await;
    for game_mod in mods {
        game_mod.restart_required = game_mods
            .restart_required
            .contains(&(app_id, game_mod.mod_id.clone()));
    }
}

async fn load_catalog(
    app: &AppHandle,
    state: &AppState,
    app_id: u32,
) -> AppResult<(GameCatalog, HashMap<String, ModDefinition>, bool)> {
    {
        let mods = state.game_mods.lock().await;
        if let Some(cached) = mods
            .cache
            .get(&app_id)
            .filter(|cached| cached.expires_at > Instant::now())
        {
            return Ok((cached.catalog.clone(), cached.definitions.clone(), true));
        }
    }

    match fetch_catalog(app_id).await {
        Ok((catalog, definitions)) => {
            persist_catalog_cache(app, app_id, &catalog, &definitions)?;
            state.game_mods.lock().await.cache.insert(
                app_id,
                CachedCatalog {
                    catalog: catalog.clone(),
                    definitions: definitions.clone(),
                    expires_at: Instant::now() + CACHE_LIFETIME,
                },
            );
            Ok((catalog, definitions, false))
        }
        Err(error) => {
            if error.code == "game_not_supported" {
                return Err(error);
            }
            if let Some((catalog, definitions)) = read_catalog_cache(app, app_id) {
                return Ok((catalog, definitions, true));
            }
            Err(error)
        }
    }
}

async fn fetch_catalog(app_id: u32) -> AppResult<(GameCatalog, HashMap<String, ModDefinition>)> {
    let client = catalog_client()?;
    let game_url = format!("{CATALOG_RAW_ROOT}/games/{app_id}/game.json");
    let response = client.get(game_url).send().await.map_err(|_| {
        mod_error(
            "catalog_unavailable",
            "the game catalog could not be reached",
        )
    })?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(mod_error(
            "game_not_supported",
            "the game has no catalog definition",
        ));
    }
    let raw = read_response_limited(
        response
            .error_for_status()
            .map_err(|_| mod_error("catalog_unavailable", "the game catalog response failed"))?,
    )
    .await?;
    let catalog: GameCatalog = serde_json::from_slice(&raw)
        .map_err(|_| mod_error("catalog_invalid", "the game catalog was invalid"))?;
    validate_catalog(app_id, &catalog)?;

    let mut definitions = HashMap::new();
    for reference in &catalog.mods {
        let url = format!("{CATALOG_RAW_ROOT}/games/{app_id}/{}", reference.file);
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|_| {
                mod_error(
                    "catalog_unavailable",
                    "a mod definition could not be reached",
                )
            })?
            .error_for_status()
            .map_err(|_| mod_error("catalog_invalid", "a mod definition was missing"))?;
        let raw = read_response_limited(response).await?;
        if sha256_hex(&raw) != reference.sha256.to_ascii_lowercase() {
            return Err(mod_error(
                "catalog_invalid",
                "a mod definition failed integrity verification",
            ));
        }
        let definition: ModDefinition = serde_json::from_slice(&raw)
            .map_err(|_| mod_error("catalog_invalid", "a mod definition was invalid"))?;
        validate_mod_definition(&definition)?;
        if definition.mod_id != reference.mod_id
            || definitions
                .insert(definition.mod_id.clone(), definition)
                .is_some()
        {
            return Err(mod_error(
                "catalog_invalid",
                "the game catalog contained inconsistent mod identifiers",
            ));
        }
    }
    validate_relations(&definitions)?;
    Ok((catalog, definitions))
}

fn validate_catalog(app_id: u32, catalog: &GameCatalog) -> AppResult<()> {
    if catalog.schema_version != 1
        || catalog.app_id != app_id
        || catalog.mods.len() > 256
        || !valid_localized_text(&catalog.name)
    {
        return Err(mod_error(
            "catalog_invalid",
            "the game catalog contract was invalid",
        ));
    }
    let mut ids = HashSet::new();
    for reference in &catalog.mods {
        let expected = format!("mods/{}.json", reference.mod_id);
        if !valid_identifier(&reference.mod_id)
            || reference.file != expected
            || !valid_sha256(&reference.sha256)
            || !ids.insert(reference.mod_id.as_str())
        {
            return Err(mod_error(
                "catalog_invalid",
                "a game mod reference was invalid",
            ));
        }
    }
    Ok(())
}

fn validate_mod_definition(definition: &ModDefinition) -> AppResult<()> {
    if definition.schema_version != 1
        || !valid_identifier(&definition.mod_id)
        || !valid_guid(&definition.guid)
        || Version::parse(&definition.version).is_err()
        || !valid_localized_text(&definition.name)
        || !valid_localized_text(&definition.description)
        || definition.compatibility.architectures.is_empty()
        || definition.compatibility.architectures.len() > 2
        || !valid_repository(&definition.release.repository)
        || !valid_release_part(&definition.release.tag)
        || !valid_release_part(&definition.release.asset)
        || !valid_release_part(&catalog_release_tag(definition))
        || !definition
            .release
            .asset
            .to_ascii_lowercase()
            .ends_with(".zip")
        || !valid_lower_sha256(&definition.release.sha256)
        || definition.dependencies.len() > 32
        || definition.conflicts.len() > 32
        || definition.config.len() > 512
    {
        return Err(mod_error(
            "catalog_invalid",
            "a mod definition contract was invalid",
        ));
    }
    if definition
        .compatibility
        .minimum_agent_version
        .as_deref()
        .is_some_and(|version| Version::parse(version).is_err())
    {
        return Err(mod_error(
            "catalog_invalid",
            "a minimum agent version was invalid",
        ));
    }
    let mut field_ids = HashSet::new();
    let mut config_keys = HashSet::new();
    for field in &definition.config {
        let (section, key) = field.section_key();
        if !valid_identifier(field.id())
            || !valid_config_name(section)
            || !valid_config_name(key)
            || field.is_locked()
            || !field.metadata_is_valid()
            || !field_ids.insert(field.id())
            || !config_keys.insert((section.to_ascii_lowercase(), key.to_ascii_lowercase()))
        {
            return Err(mod_error(
                "catalog_invalid",
                "a mod config field was invalid",
            ));
        }
        field.serialize_value(&field.default_value())?;
    }
    for dependency in &definition.dependencies {
        if !valid_identifier(&dependency.mod_id)
            || Version::parse(&dependency.minimum_version).is_err()
        {
            return Err(mod_error("catalog_invalid", "a mod dependency was invalid"));
        }
    }
    if definition
        .conflicts
        .iter()
        .any(|conflict| !valid_identifier(conflict))
    {
        return Err(mod_error("catalog_invalid", "a mod conflict was invalid"));
    }
    Ok(())
}

fn validate_relations(definitions: &HashMap<String, ModDefinition>) -> AppResult<()> {
    for definition in definitions.values() {
        for dependency in &definition.dependencies {
            let Some(required) = definitions.get(&dependency.mod_id) else {
                return Err(mod_error("catalog_invalid", "a mod dependency was missing"));
            };
            if Version::parse(&required.version).ok()
                < Version::parse(&dependency.minimum_version).ok()
            {
                return Err(mod_error(
                    "catalog_invalid",
                    "a mod dependency version was unavailable",
                ));
            }
        }
        if definition
            .conflicts
            .iter()
            .any(|conflict| conflict == &definition.mod_id)
        {
            return Err(mod_error("catalog_invalid", "a mod conflicted with itself"));
        }
    }
    Ok(())
}

fn describe_mod(
    target: Option<&InstallTarget>,
    app_id: u32,
    definition: &ModDefinition,
) -> AppResult<GameMod> {
    let marker =
        target.and_then(|target| read_mod_marker(&mod_directory(target, &definition.mod_id)));
    let compatible = target.is_some_and(|target| {
        target.runtime == definition.compatibility.runtime
            && definition
                .compatibility
                .architectures
                .contains(&target.architecture)
    });
    let (status, installed_version) = match marker {
        Some(marker) if marker.app_id == app_id && marker.mod_id == definition.mod_id => {
            let installed = Version::parse(&marker.version).ok();
            let available = Version::parse(&definition.version).ok();
            (
                if installed < available {
                    GameModStatus::UpdateAvailable
                } else {
                    GameModStatus::Installed
                },
                Some(marker.version),
            )
        }
        Some(_) => (GameModStatus::Blocked, None),
        None if compatible => (GameModStatus::NotInstalled, None),
        None => (GameModStatus::Blocked, None),
    };
    let values = target
        .map(|target| read_config_values(target, definition))
        .transpose()?
        .unwrap_or_default();
    Ok(GameMod {
        mod_id: definition.mod_id.clone(),
        guid: definition.guid.clone(),
        version: definition.version.clone(),
        installed_version,
        official: definition.official,
        external: false,
        name: definition.name.clone(),
        description: definition.description.clone(),
        integration: definition.integration,
        status,
        dependencies: definition.dependencies.clone(),
        conflicts: definition.conflicts.clone(),
        config: definition.config.clone(),
        values,
        restart_required: false,
    })
}

pub async fn prepare_install(
    app: &AppHandle,
    state: &AppState,
    app_id: u32,
    requested: Vec<String>,
    update: bool,
) -> AppResult<ModActionPlan> {
    if !cfg!(windows) {
        return Err(mod_error(
            "mod_unsupported",
            "automatic mod installation is only available on Windows",
        ));
    }
    if requested.is_empty() || requested.len() > 32 {
        return Err(mod_error(
            "mod_invalid_request",
            "no valid mods were selected",
        ));
    }
    let game = resolve_game(app_id).await?;
    let (status, target) = analyze_windows_installation(&game.install_directory)
        .map_err(|_| mod_error("mod_blocked", "the game installation is not compatible"))?;
    if status.status != BepInExAvailability::Installed {
        return Err(mod_error(
            "mod_requires_bepinex",
            "BepInEx must be installed first",
        ));
    }
    ensure_game_stopped(&target.executable)?;
    let (_, definitions, _) = load_catalog(app, state, app_id).await?;
    let required_mod_ids = resolve_install_set(&definitions, &requested)?;
    let (mod_ids, replace_existing) = resolve_action_set(
        app_id,
        &target,
        &definitions,
        &requested,
        &required_mod_ids,
        update,
    )?;
    validate_install_set(
        app_id,
        &target,
        &definitions,
        &mod_ids,
        &required_mod_ids,
        &replace_existing,
    )?;
    let requires_agent = required_mod_ids.iter().any(|mod_id| {
        definitions
            .get(mod_id)
            .is_some_and(|definition| definition.integration == ModIntegration::Agent)
    });
    let installs_agent = requires_agent && !agent::agent_is_current(&target, app_id);

    let plan_id = random_token("mod_install_error")?;
    let mut mods = state.game_mods.lock().await;
    prune_plans(&mut mods.pending);
    if mods.busy_games.contains(&app_id) {
        return Err(mod_error(
            "mod_busy",
            "another mod action is already running",
        ));
    }
    mods.pending.insert(
        plan_id.clone(),
        PreparedPlan {
            action: PreparedAction::Install {
                app_id,
                mod_ids: mod_ids.clone(),
                required_mod_ids,
                replace_existing: replace_existing.iter().cloned().collect(),
            },
            expires_at: Instant::now() + PLAN_LIFETIME,
        },
    );
    Ok(ModActionPlan {
        plan_id,
        app_id,
        mod_ids,
        installs_agent,
        action: if update { "update" } else { "install" },
    })
}

pub async fn execute_install(
    app: &AppHandle,
    state: &AppState,
    plan_id: String,
) -> AppResult<GameSupport> {
    let PreparedAction::Install {
        app_id,
        mod_ids,
        required_mod_ids,
        replace_existing,
    } = consume_plan(state, &plan_id).await?
    else {
        return Err(mod_error(
            "mod_plan_expired",
            "the mod installation plan was invalid",
        ));
    };
    mark_busy(state, app_id).await?;
    let replace_existing = replace_existing.into_iter().collect();
    let result = execute_install_inner(
        app,
        state,
        app_id,
        &mod_ids,
        &required_mod_ids,
        &replace_existing,
    )
    .await;
    state.game_mods.lock().await.busy_games.remove(&app_id);
    result?;
    get_support(app, state, app_id).await
}

async fn execute_install_inner(
    app: &AppHandle,
    state: &AppState,
    app_id: u32,
    mod_ids: &[String],
    required_mod_ids: &[String],
    replace_existing: &HashSet<String>,
) -> AppResult<()> {
    let game = resolve_game(app_id).await?;
    let (status, target) = analyze_windows_installation(&game.install_directory)
        .map_err(|_| mod_error("mod_blocked", "the game installation changed"))?;
    if status.status != BepInExAvailability::Installed {
        return Err(mod_error(
            "mod_requires_bepinex",
            "BepInEx is no longer installed",
        ));
    }
    ensure_game_stopped(&target.executable)?;
    let (_, definitions, _) = load_catalog(app, state, app_id).await?;
    validate_install_set(
        app_id,
        &target,
        &definitions,
        mod_ids,
        required_mod_ids,
        replace_existing,
    )?;

    let staging = TempBuilder::new()
        .prefix(".gametweaks-mods-")
        .tempdir_in(&target.game_root)
        .map_err(|_| {
            mod_error(
                "mod_install_error",
                "a mod staging directory could not be created",
            )
        })?;
    let mut prepared = Vec::new();
    if required_mod_ids.iter().any(|mod_id| {
        definitions
            .get(mod_id)
            .is_some_and(|definition| definition.integration == ModIntegration::Agent)
    }) && !agent::agent_is_current(&target, app_id)
    {
        agent::stage_bundled_agent(app, &target, staging.path())?;
    }
    for mod_id in mod_ids {
        let definition = definitions.get(mod_id).ok_or_else(|| {
            mod_error(
                "catalog_invalid",
                "a selected mod disappeared from the catalog",
            )
        })?;
        let archive = download_mod(app, app_id, definition, staging.path()).await?;
        emit_mod_progress(app, app_id, mod_id, ModProgressStage::Verifying, 0, None);
        let content = staging.path().join(format!("content-{mod_id}"));
        fs::create_dir(&content)
            .map_err(|_| mod_error("mod_install_error", "a mod staging directory was invalid"))?;
        let files = extract_mod_archive(&archive, &content)?;
        let marker = ModInstallMarker {
            schema_version: 1,
            app_id,
            mod_id: mod_id.clone(),
            guid: definition.guid.clone(),
            version: definition.version.clone(),
            sha256: definition.release.sha256.to_ascii_lowercase(),
            files,
        };
        write_json_new(&content.join(".gametweaks-mod.json"), &marker)?;
        prepared.push((definition.clone(), content));
    }

    ensure_game_stopped(&target.executable)?;
    let mut committed = Vec::new();
    let rollback_root = staging.path().join("rollback");
    fs::create_dir(&rollback_root).map_err(|_| {
        mod_error(
            "mod_install_error",
            "the rollback directory could not be created",
        )
    })?;
    if let Some(agent_commit) =
        agent::commit_staged_agent(&target, staging.path(), app_id, &rollback_root)?
    {
        committed.push(agent_commit);
    }
    for (definition, content) in &prepared {
        emit_mod_progress(
            app,
            app_id,
            &definition.mod_id,
            ModProgressStage::Installing,
            0,
            None,
        );
        let destination = mod_directory(&target, &definition.mod_id);
        let old = rollback_root.join(&definition.mod_id);
        let had_old = destination.exists();
        if had_old {
            if !replace_existing.contains(&definition.mod_id)
                || read_mod_marker(&destination).is_none()
            {
                rollback_mod_commits(&committed);
                return Err(mod_error(
                    "mod_collision",
                    "an existing mod directory is not managed by GameTweaks",
                ));
            }
            fs::rename(&destination, &old).map_err(|_| {
                mod_error(
                    "mod_install_error",
                    "the previous mod version could not be staged",
                )
            })?;
        }
        if let Err(error) = fs::rename(content, &destination) {
            if had_old {
                let _ = fs::rename(&old, &destination);
            }
            rollback_mod_commits(&committed);
            tracing::warn!(%error, "failed to commit a GameTweaks mod");
            return Err(mod_error(
                "mod_install_error",
                "a mod could not be committed",
            ));
        }
        committed.push((destination, had_old.then_some(old)));
    }
    for (destination, old) in &committed {
        if let Some(old) = old {
            let _ = fs::remove_dir_all(old);
        }
        let mod_id = destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        emit_mod_progress(app, app_id, mod_id, ModProgressStage::Completed, 0, None);
    }
    Ok(())
}

fn rollback_mod_commits(committed: &[(PathBuf, Option<PathBuf>)]) {
    for (destination, old) in committed.iter().rev() {
        let _ = fs::remove_dir_all(destination);
        if let Some(old) = old {
            let _ = fs::rename(old, destination);
        }
    }
}

pub async fn prepare_mod_uninstall(
    app: &AppHandle,
    state: &AppState,
    app_id: u32,
    mod_id: String,
    remove_config: bool,
) -> AppResult<ModActionPlan> {
    let game = resolve_game(app_id).await?;
    let (_, target) = analyze_windows_installation(&game.install_directory)
        .map_err(|_| mod_error("mod_blocked", "the game installation is not compatible"))?;
    ensure_game_stopped(&target.executable)?;
    let (_, definitions, _) = load_catalog(app, state, app_id).await?;
    let definition = definitions
        .get(&mod_id)
        .ok_or_else(|| mod_error("mod_not_found", "the mod was not found in the catalog"))?;
    let marker = read_mod_marker(&mod_directory(&target, &mod_id))
        .ok_or_else(|| mod_error("mod_not_managed", "the mod is not managed by GameTweaks"))?;
    if marker.app_id != app_id || marker.mod_id != mod_id {
        return Err(mod_error(
            "mod_not_managed",
            "the mod marker did not match this game",
        ));
    }
    for other in definitions.values() {
        if other.mod_id != mod_id
            && read_mod_marker(&mod_directory(&target, &other.mod_id)).is_some()
            && other
                .dependencies
                .iter()
                .any(|dependency| dependency.mod_id == mod_id)
        {
            return Err(mod_error(
                "mod_required",
                "another installed mod depends on this mod",
            ));
        }
    }
    let plan_id = random_token("mod_uninstall_error")?;
    state.game_mods.lock().await.pending.insert(
        plan_id.clone(),
        PreparedPlan {
            action: PreparedAction::UninstallMod {
                app_id,
                mod_id: mod_id.clone(),
                remove_config,
            },
            expires_at: Instant::now() + PLAN_LIFETIME,
        },
    );
    Ok(ModActionPlan {
        plan_id,
        app_id,
        mod_ids: vec![definition.mod_id.clone()],
        installs_agent: false,
        action: "uninstall",
    })
}

pub async fn uninstall_mod(
    app: &AppHandle,
    state: &AppState,
    plan_id: String,
) -> AppResult<GameSupport> {
    let PreparedAction::UninstallMod {
        app_id,
        mod_id,
        remove_config,
    } = consume_plan(state, &plan_id).await?
    else {
        return Err(mod_error(
            "mod_plan_expired",
            "the mod uninstall plan was invalid",
        ));
    };
    mark_busy(state, app_id).await?;
    let result = async {
        let game = resolve_game(app_id).await?;
        let (_, target) = analyze_windows_installation(&game.install_directory)
            .map_err(|_| mod_error("mod_blocked", "the game installation changed"))?;
        ensure_game_stopped(&target.executable)?;
        let (_, definitions, _) = load_catalog(app, state, app_id).await?;
        let definition = definitions
            .get(&mod_id)
            .ok_or_else(|| mod_error("mod_not_found", "the mod was not found"))?;
        for other in definitions.values() {
            if other.mod_id != mod_id
                && read_mod_marker(&mod_directory(&target, &other.mod_id)).is_some()
                && other
                    .dependencies
                    .iter()
                    .any(|dependency| dependency.mod_id == mod_id)
            {
                return Err(mod_error(
                    "mod_required",
                    "another installed mod now depends on this mod",
                ));
            }
        }
        let directory = mod_directory(&target, &mod_id);
        let marker = read_mod_marker(&directory)
            .ok_or_else(|| mod_error("mod_not_managed", "the mod is not managed by GameTweaks"))?;
        if marker.app_id != app_id || marker.mod_id != mod_id || marker.guid != definition.guid {
            return Err(mod_error(
                "mod_not_managed",
                "the mod marker no longer matches the selected game",
            ));
        }
        let staging = TempBuilder::new()
            .prefix(".gametweaks-uninstall-")
            .tempdir_in(&target.game_root)
            .map_err(|_| {
                mod_error(
                    "mod_uninstall_error",
                    "the mod uninstall staging directory could not be created",
                )
            })?;
        let mut moves = marker
            .files
            .iter()
            .map(|relative| {
                Ok((
                    safe_join(&directory, relative)?,
                    safe_join(&staging.path().join("mod"), relative)?,
                ))
            })
            .collect::<AppResult<Vec<_>>>()?;
        let marker_path = directory.join(".gametweaks-mod.json");
        moves.push((marker_path, staging.path().join("mod/.gametweaks-mod.json")));
        if remove_config {
            let config = config_path(&target, &definition.guid);
            if config.exists() {
                moves.push((config, staging.path().join("config/mod.cfg")));
            }
        }
        stage_files_for_removal(&moves, "mod_uninstall_error")?;
        prune_empty_directories(&directory, &directory);
        Ok::<(), ErrorResponse>(())
    }
    .await;
    state.game_mods.lock().await.busy_games.remove(&app_id);
    result?;
    get_support(app, state, app_id).await
}

pub async fn set_config(
    app: &AppHandle,
    state: &AppState,
    app_id: u32,
    mod_id: String,
    changes: HashMap<String, Value>,
) -> AppResult<GameSupport> {
    if changes.is_empty() || changes.len() > 128 {
        return Err(mod_error(
            "mod_config_invalid",
            "no valid settings were provided",
        ));
    }
    let game = resolve_game(app_id).await?;
    let (status, target) = analyze_windows_installation(&game.install_directory)
        .map_err(|_| mod_error("mod_blocked", "the game installation is not compatible"))?;
    if status.status != BepInExAvailability::Installed {
        return Err(mod_error(
            "mod_requires_bepinex",
            "BepInEx is not installed",
        ));
    }
    let dynamic_fields = agent::runtime_fields(state, app_id, &mod_id).await;
    let managed_marker = read_mod_marker(&mod_directory(&target, &mod_id));
    let catalog = load_catalog(app, state, app_id).await;
    let (integration, guid, fields) = match catalog {
        Ok((_, definitions, _)) => {
            if let Some(definition) = definitions.get(&mod_id) {
                let marker = managed_marker.ok_or_else(|| {
                    mod_error("mod_not_managed", "the catalog mod is not installed")
                })?;
                if marker.app_id != app_id
                    || marker.mod_id != mod_id
                    || marker.guid != definition.guid
                {
                    return Err(mod_error(
                        "mod_not_managed",
                        "the mod marker no longer matches the selected game",
                    ));
                }
                let mut fields = definition.config.clone();
                if let Some(dynamic) = &dynamic_fields {
                    agent::merge_runtime_fields(&mut fields, dynamic);
                }
                (
                    definition.integration,
                    Some(definition.guid.clone()),
                    fields,
                )
            } else if managed_marker.is_some() {
                return Err(mod_error(
                    "mod_not_found",
                    "the installed catalog mod was no longer defined",
                ));
            } else {
                (
                    ModIntegration::Agent,
                    None,
                    dynamic_fields.clone().ok_or_else(|| {
                        mod_error("mod_not_found", "the external mod was not connected")
                    })?,
                )
            }
        }
        Err(error) if managed_marker.is_some() => return Err(error),
        Err(_) => (
            ModIntegration::Agent,
            None,
            dynamic_fields
                .clone()
                .ok_or_else(|| mod_error("mod_not_found", "the external mod was not connected"))?,
        ),
    };
    let mut serialized = Vec::with_capacity(changes.len());
    for (id, value) in &changes {
        let field = fields
            .iter()
            .find(|field| field.id() == id)
            .ok_or_else(|| mod_error("mod_config_invalid", "an unknown setting was provided"))?;
        if field.is_locked() {
            return Err(mod_error(
                "mod_schema_conflict",
                "a dynamic Agent field contradicted the catalog schema",
            ));
        }
        serialized.push((field, field.serialize_value(value)?));
    }
    if agent::is_connected(state, app_id).await && integration == ModIntegration::Agent {
        if agent::set_config(state, app_id, &mod_id, &changes).await? {
            state
                .game_mods
                .lock()
                .await
                .restart_required
                .insert((app_id, mod_id.clone()));
        }
    } else {
        let guid = guid.ok_or_else(|| {
            mod_error(
                "mod_agent_required",
                "an external mod requires a unique live Agent connection",
            )
        })?;
        ensure_game_stopped(&target.executable).map_err(|_| {
            mod_error(
                "mod_agent_required",
                "the game is running without a connected GameTweaks agent",
            )
        })?;
        update_config_file(&config_path(&target, &guid), &serialized)?;
    }
    get_support(app, state, app_id).await
}

pub async fn prepare_bepinex_uninstall(
    state: &AppState,
    app_id: u32,
) -> AppResult<BepInExUninstallPlan> {
    if !cfg!(windows) {
        return Err(mod_error(
            "bepinex_uninstall_unsupported",
            "BepInEx uninstall is only available on Windows",
        ));
    }
    let game = resolve_game(app_id).await?;
    let (status, target) = analyze_windows_installation(&game.install_directory).map_err(|_| {
        mod_error(
            "bepinex_uninstall_blocked",
            "the installation could not be verified",
        )
    })?;
    if status.status != BepInExAvailability::Installed || !status.managed_by_game_tweaks {
        return Err(mod_error(
            "bepinex_not_managed",
            "only GameTweaks BepInEx installations can be removed",
        ));
    }
    ensure_game_stopped(&target.executable)?;
    let marker_path = target.game_root.join("BepInEx/.gametweaks-install.json");
    let marker = valid_install_marker(&marker_path)
        .filter(|marker| marker.app_id == app_id)
        .ok_or_else(|| mod_error("bepinex_not_managed", "the BepInEx marker was invalid"))?;
    let additional_file_count = collect_additional_files(&target, &marker.files)?.len();
    let plan_id = random_token("bepinex_uninstall_error")?;
    state.game_mods.lock().await.pending.insert(
        plan_id.clone(),
        PreparedPlan {
            action: PreparedAction::UninstallBepInEx { app_id },
            expires_at: Instant::now() + PLAN_LIFETIME,
        },
    );
    Ok(BepInExUninstallPlan {
        plan_id,
        app_id,
        version: marker.version,
        additional_file_count,
    })
}

pub async fn uninstall_bepinex(
    app: &AppHandle,
    state: &AppState,
    plan_id: String,
) -> AppResult<()> {
    let PreparedAction::UninstallBepInEx { app_id } = consume_plan(state, &plan_id).await? else {
        return Err(mod_error(
            "bepinex_uninstall_plan_expired",
            "the BepInEx uninstall plan was invalid",
        ));
    };
    mark_busy(state, app_id).await?;
    let result = uninstall_bepinex_inner(app, app_id).await;
    state.game_mods.lock().await.busy_games.remove(&app_id);
    result
}

async fn uninstall_bepinex_inner(app: &AppHandle, app_id: u32) -> AppResult<()> {
    let game = resolve_game(app_id).await?;
    let (status, target) = analyze_windows_installation(&game.install_directory)
        .map_err(|_| mod_error("bepinex_uninstall_blocked", "the installation changed"))?;
    if status.status != BepInExAvailability::Installed || !status.managed_by_game_tweaks {
        return Err(mod_error(
            "bepinex_not_managed",
            "BepInEx is no longer managed by GameTweaks",
        ));
    }
    ensure_game_stopped(&target.executable)?;
    let marker_path = target.game_root.join("BepInEx/.gametweaks-install.json");
    let marker = valid_install_marker(&marker_path)
        .filter(|marker| marker.app_id == app_id)
        .ok_or_else(|| mod_error("bepinex_not_managed", "the BepInEx marker changed"))?;
    let additional = collect_additional_files(&target, &marker.files)?;
    if !additional.is_empty() {
        backup_additional_files(app, app_id, &target, &additional)?;
    }
    let staging = TempBuilder::new()
        .prefix(".gametweaks-bepinex-uninstall-")
        .tempdir_in(&target.game_root)
        .map_err(|_| {
            mod_error(
                "bepinex_uninstall_error",
                "the BepInEx uninstall staging directory could not be created",
            )
        })?;
    let mut removals = marker.files.clone();
    removals.extend(additional);
    removals.push("BepInEx/.gametweaks-install.json".to_owned());
    removals.sort();
    removals.dedup();
    let moves = removals
        .iter()
        .map(|relative| {
            Ok((
                safe_join(&target.game_root, relative)?,
                safe_join(staging.path(), relative)?,
            ))
        })
        .collect::<AppResult<Vec<_>>>()?;
    stage_files_for_removal(&moves, "bepinex_uninstall_error")?;
    prune_empty_directories(&target.game_root.join("BepInEx"), &target.game_root);
    prune_empty_directories(&target.game_root.join("dotnet"), &target.game_root);
    Ok(())
}

fn collect_additional_files(target: &InstallTarget, owned: &[String]) -> AppResult<Vec<String>> {
    let owned: HashSet<_> = owned.iter().map(|path| path.replace('\\', "/")).collect();
    let mut additional = Vec::new();
    for top in ["BepInEx", "dotnet"] {
        let root = target.game_root.join(top);
        if !root.exists() {
            continue;
        }
        let mut pending = vec![root];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).map_err(|_| {
                mod_error(
                    "bepinex_uninstall_blocked",
                    "BepInEx files could not be inspected",
                )
            })? {
                let entry = entry.map_err(|_| {
                    mod_error(
                        "bepinex_uninstall_blocked",
                        "a BepInEx file could not be inspected",
                    )
                })?;
                let kind = entry.file_type().map_err(|_| {
                    mod_error(
                        "bepinex_uninstall_blocked",
                        "a BepInEx file type could not be verified",
                    )
                })?;
                if kind.is_symlink() {
                    return Err(mod_error(
                        "bepinex_uninstall_blocked",
                        "a BepInEx symlink blocked uninstall",
                    ));
                }
                if kind.is_dir() {
                    pending.push(entry.path());
                } else if kind.is_file() {
                    let relative = entry
                        .path()
                        .strip_prefix(&target.game_root)
                        .map_err(|_| {
                            mod_error("bepinex_uninstall_blocked", "a BepInEx path was unsafe")
                        })?
                        .to_string_lossy()
                        .replace('\\', "/");
                    if relative != "BepInEx/.gametweaks-install.json" && !owned.contains(&relative)
                    {
                        additional.push(relative);
                    }
                }
            }
        }
    }
    additional.sort();
    Ok(additional)
}

fn backup_additional_files(
    app: &AppHandle,
    app_id: u32,
    target: &InstallTarget,
    files: &[String],
) -> AppResult<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            mod_error(
                "bepinex_uninstall_error",
                "a backup timestamp could not be created",
            )
        })?
        .as_secs();
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| {
            mod_error(
                "bepinex_uninstall_error",
                "the backup location was unavailable",
            )
        })?
        .join("backups")
        .join(app_id.to_string())
        .join(timestamp.to_string());
    fs::create_dir_all(&root).map_err(|_| {
        mod_error(
            "bepinex_uninstall_error",
            "the backup directory could not be created",
        )
    })?;
    for relative in files {
        let source = safe_join(&target.game_root, relative)?;
        let destination = safe_join(&root, relative)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|_| {
                mod_error(
                    "bepinex_uninstall_error",
                    "a backup directory could not be created",
                )
            })?;
        }
        fs::copy(&source, &destination).map_err(|_| {
            mod_error(
                "bepinex_uninstall_error",
                "a BepInEx backup could not be written",
            )
        })?;
        if sha256_file(&source)? != sha256_file(&destination)? {
            return Err(mod_error(
                "bepinex_uninstall_error",
                "a BepInEx backup failed verification",
            ));
        }
    }
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "appId": app_id,
        "createdAtUnix": timestamp,
        "files": files,
    });
    write_json_new(&root.join("backup.json"), &manifest)
}

fn resolve_install_set(
    definitions: &HashMap<String, ModDefinition>,
    requested: &[String],
) -> AppResult<Vec<String>> {
    let mut resolved = Vec::new();
    let mut visiting = HashSet::new();
    let mut done = HashSet::new();
    fn visit(
        mod_id: &str,
        definitions: &HashMap<String, ModDefinition>,
        visiting: &mut HashSet<String>,
        done: &mut HashSet<String>,
        resolved: &mut Vec<String>,
    ) -> AppResult<()> {
        if done.contains(mod_id) {
            return Ok(());
        }
        if !visiting.insert(mod_id.to_owned()) {
            return Err(mod_error(
                "catalog_invalid",
                "the mod dependency graph contained a cycle",
            ));
        }
        let definition = definitions
            .get(mod_id)
            .ok_or_else(|| mod_error("mod_not_found", "a selected mod was not found"))?;
        for dependency in &definition.dependencies {
            visit(&dependency.mod_id, definitions, visiting, done, resolved)?;
        }
        visiting.remove(mod_id);
        done.insert(mod_id.to_owned());
        resolved.push(mod_id.to_owned());
        Ok(())
    }
    for mod_id in requested {
        visit(mod_id, definitions, &mut visiting, &mut done, &mut resolved)?;
    }
    Ok(resolved)
}

fn resolve_action_set(
    app_id: u32,
    target: &InstallTarget,
    definitions: &HashMap<String, ModDefinition>,
    requested: &[String],
    resolved: &[String],
    update: bool,
) -> AppResult<(Vec<String>, HashSet<String>)> {
    let requested_set: HashSet<_> = requested.iter().map(String::as_str).collect();
    if requested_set.len() != requested.len() {
        return Err(mod_error(
            "mod_invalid_request",
            "the same mod was selected more than once",
        ));
    }
    let mut requirements = HashMap::<String, Version>::new();
    for mod_id in resolved {
        let definition = definitions
            .get(mod_id)
            .ok_or_else(|| mod_error("catalog_invalid", "a resolved mod was missing"))?;
        for dependency in &definition.dependencies {
            let minimum = Version::parse(&dependency.minimum_version)
                .map_err(|_| mod_error("catalog_invalid", "a dependency version was invalid"))?;
            requirements
                .entry(dependency.mod_id.clone())
                .and_modify(|known| {
                    if minimum > *known {
                        *known = minimum.clone();
                    }
                })
                .or_insert(minimum);
        }
    }

    let mut actions = Vec::new();
    let mut replace_existing = HashSet::new();
    for mod_id in resolved {
        let definition = definitions
            .get(mod_id)
            .ok_or_else(|| mod_error("catalog_invalid", "a resolved mod was missing"))?;
        let destination = mod_directory(target, mod_id);
        let marker = read_mod_marker(&destination);
        if destination.exists() && marker.is_none() {
            return Err(mod_error(
                "mod_collision",
                "an existing mod directory is not safely managed by GameTweaks",
            ));
        }
        if let Some(marker) = &marker {
            if marker.app_id != app_id || marker.mod_id != *mod_id || marker.guid != definition.guid
            {
                return Err(mod_error(
                    "mod_collision",
                    "an installed mod marker did not match the catalog",
                ));
            }
        }

        if requested_set.contains(mod_id.as_str()) {
            match (update, marker.is_some()) {
                (true, false) => {
                    return Err(mod_error("mod_not_managed", "the mod is not installed"))
                }
                (false, true) => {
                    return Err(mod_error(
                        "mod_already_installed",
                        "the selected mod is already installed",
                    ))
                }
                (true, true) => {
                    replace_existing.insert(mod_id.clone());
                }
                (false, false) => {}
            }
            actions.push(mod_id.clone());
            continue;
        }

        if let Some(marker) = marker {
            let installed = Version::parse(&marker.version).map_err(|_| {
                mod_error(
                    "mod_collision",
                    "an installed dependency version was invalid",
                )
            })?;
            let minimum = requirements.get(mod_id).ok_or_else(|| {
                mod_error("catalog_invalid", "a dependency requirement was missing")
            })?;
            if &installed >= minimum {
                continue;
            }
            replace_existing.insert(mod_id.clone());
        }
        actions.push(mod_id.clone());
    }
    Ok((actions, replace_existing))
}

fn validate_install_set(
    app_id: u32,
    target: &InstallTarget,
    definitions: &HashMap<String, ModDefinition>,
    mod_ids: &[String],
    required_mod_ids: &[String],
    replace_existing: &HashSet<String>,
) -> AppResult<()> {
    let selected: HashSet<_> = required_mod_ids.iter().map(String::as_str).collect();
    let installed: HashSet<_> = definitions
        .keys()
        .filter(|mod_id| read_mod_marker(&mod_directory(target, mod_id)).is_some())
        .map(String::as_str)
        .collect();
    for mod_id in required_mod_ids {
        let definition = definitions
            .get(mod_id)
            .ok_or_else(|| mod_error("mod_not_found", "a selected mod was missing"))?;
        if definition.compatibility.runtime != target.runtime
            || !definition
                .compatibility
                .architectures
                .contains(&target.architecture)
        {
            return Err(mod_error(
                "mod_incompatible",
                "a selected mod is not compatible with this game",
            ));
        }
        if definition.integration == ModIntegration::Agent
            && !agent::bundled_agent_meets(
                definition.compatibility.minimum_agent_version.as_deref(),
            )
        {
            return Err(mod_error(
                "mod_incompatible",
                "the bundled agent does not meet a selected mod's minimum version",
            ));
        }
        for conflict in &definition.conflicts {
            if selected.contains(conflict.as_str()) || installed.contains(conflict.as_str()) {
                return Err(mod_error(
                    "mod_conflict",
                    "a selected mod conflicts with another mod",
                ));
            }
        }
    }
    for mod_id in mod_ids {
        let destination = mod_directory(target, mod_id);
        match read_mod_marker(&destination) {
            Some(marker)
                if marker.app_id == app_id
                    && marker.mod_id == *mod_id
                    && definitions
                        .get(mod_id)
                        .is_some_and(|definition| marker.guid == definition.guid) =>
            {
                if !replace_existing.contains(mod_id) {
                    return Err(mod_error(
                        "mod_collision",
                        "an existing mod was not replaceable",
                    ));
                }
            }
            Some(_) => {
                return Err(mod_error(
                    "mod_collision",
                    "a mod marker did not match this game",
                ))
            }
            None if destination.exists() => {
                return Err(mod_error("mod_collision", "a mod directory already exists"));
            }
            None if replace_existing.contains(mod_id) => {
                return Err(mod_error(
                    "mod_not_managed",
                    "the mod is no longer installed",
                ))
            }
            None => {}
        }
    }
    Ok(())
}

async fn download_mod(
    app: &AppHandle,
    app_id: u32,
    definition: &ModDefinition,
    directory: &Path,
) -> AppResult<PathBuf> {
    let url = Url::parse(&format!(
        "{RELEASE_ROOT}/{}/{}",
        catalog_release_tag(definition),
        definition.release.asset
    ))
    .map_err(|_| mod_error("mod_download_invalid", "the mod release URL was invalid"))?;
    let client = catalog_client()?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| {
            mod_error(
                "mod_network_error",
                "the mod package could not be downloaded",
            )
        })?
        .error_for_status()
        .map_err(|_| mod_error("mod_network_error", "the mod package download failed"))?;
    if !trusted_catalog_url(response.url())
        || response
            .content_length()
            .is_some_and(|length| length > MOD_ARCHIVE_MAX_BYTES)
    {
        return Err(mod_error(
            "mod_integrity_error",
            "the mod package source was not trusted",
        ));
    }
    let temporary = NamedTempFile::new_in(directory).map_err(|_| {
        mod_error(
            "mod_install_error",
            "a temporary mod archive could not be created",
        )
    })?;
    let path = temporary.into_temp_path().keep().map_err(|_| {
        mod_error(
            "mod_install_error",
            "a temporary mod archive could not be retained",
        )
    })?;
    let mut output = tokio::fs::File::create(&path)
        .await
        .map_err(|_| mod_error("mod_install_error", "the mod archive could not be opened"))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let total = response.content_length();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| mod_error("mod_network_error", "the mod download was interrupted"))?
    {
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > MOD_ARCHIVE_MAX_BYTES {
            return Err(mod_error(
                "mod_integrity_error",
                "the mod archive exceeded its size limit",
            ));
        }
        output
            .write_all(&chunk)
            .await
            .map_err(|_| mod_error("mod_install_error", "the mod archive could not be written"))?;
        hasher.update(&chunk);
        emit_mod_progress(
            app,
            app_id,
            &definition.mod_id,
            ModProgressStage::Downloading,
            downloaded,
            total,
        );
    }
    output.sync_all().await.map_err(|_| {
        mod_error(
            "mod_install_error",
            "the mod archive could not be finalized",
        )
    })?;
    let digest = format!("{:x}", hasher.finalize());
    if !digest.eq_ignore_ascii_case(&definition.release.sha256) {
        return Err(mod_error(
            "mod_integrity_error",
            "the mod package digest did not match",
        ));
    }
    Ok(path)
}

fn extract_mod_archive(archive_path: &Path, destination: &Path) -> AppResult<Vec<String>> {
    let file = File::open(archive_path)
        .map_err(|_| mod_error("mod_integrity_error", "the mod archive could not be opened"))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|_| mod_error("mod_integrity_error", "the mod archive was invalid"))?;
    if archive.is_empty() || archive.len() > MOD_ARCHIVE_MAX_ENTRIES {
        return Err(mod_error(
            "mod_integrity_error",
            "the mod archive had an invalid entry count",
        ));
    }
    let mut total = 0_u64;
    let mut paths = HashSet::new();
    let mut files = Vec::new();
    let mut has_dll = false;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| mod_error("mod_integrity_error", "a mod archive entry was invalid"))?;
        if entry.encrypted()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(mod_error(
                "mod_integrity_error",
                "the mod archive contained an unsafe entry",
            ));
        }
        let path = entry.enclosed_name().ok_or_else(|| {
            mod_error(
                "mod_integrity_error",
                "the mod archive contained an unsafe path",
            )
        })?;
        validate_relative_path(&path)?;
        let normalized = path.to_string_lossy().replace('\\', "/");
        if !paths.insert(normalized.clone()) {
            return Err(mod_error(
                "mod_integrity_error",
                "the mod archive contained duplicate paths",
            ));
        }
        total = total
            .checked_add(entry.size())
            .filter(|total| *total <= MOD_ARCHIVE_MAX_UNCOMPRESSED_BYTES)
            .ok_or_else(|| {
                mod_error(
                    "mod_integrity_error",
                    "the mod archive exceeded its extracted limit",
                )
            })?;
        if !entry.is_dir() {
            has_dll |= normalized.to_ascii_lowercase().ends_with(".dll");
            files.push(normalized);
        }
    }
    if !has_dll {
        return Err(mod_error(
            "mod_integrity_error",
            "the mod archive contained no plugin DLL",
        ));
    }
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| {
            mod_error(
                "mod_integrity_error",
                "a mod archive entry could not be read",
            )
        })?;
        let path = entry
            .enclosed_name()
            .ok_or_else(|| mod_error("mod_integrity_error", "a mod archive path was unsafe"))?;
        let output = destination.join(path);
        if entry.is_dir() {
            fs::create_dir_all(output).map_err(|_| {
                mod_error("mod_install_error", "a mod directory could not be created")
            })?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|_| {
                mod_error("mod_install_error", "a mod directory could not be created")
            })?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .map_err(|_| mod_error("mod_install_error", "a mod file could not be created"))?;
        let expected = entry.size();
        let copied = std::io::copy(&mut entry.take(expected.saturating_add(1)), &mut file)
            .map_err(|_| mod_error("mod_install_error", "a mod file could not be extracted"))?;
        if copied != expected {
            return Err(mod_error(
                "mod_integrity_error",
                "a mod file had an invalid size",
            ));
        }
    }
    Ok(files)
}

fn read_config_values(
    target: &InstallTarget,
    definition: &ModDefinition,
) -> AppResult<HashMap<String, Value>> {
    let path = config_path(target, &definition.guid);
    let text = if path.exists() {
        let metadata = fs::metadata(&path).map_err(|_| {
            mod_error(
                "mod_config_error",
                "the mod configuration could not be inspected",
            )
        })?;
        if metadata.len() > CONFIG_MAX_BYTES {
            return Err(mod_error(
                "mod_config_error",
                "the mod configuration was too large",
            ));
        }
        fs::read_to_string(path).map_err(|_| {
            mod_error(
                "mod_config_error",
                "the mod configuration could not be read",
            )
        })?
    } else {
        String::new()
    };
    let parsed = parse_config(&text);
    let mut values = HashMap::new();
    for field in &definition.config {
        let (section, key) = field.section_key();
        let value = parsed
            .get(&(section.to_ascii_lowercase(), key.to_ascii_lowercase()))
            .and_then(|raw| parse_field_value(field, raw))
            .unwrap_or_else(|| field.default_value());
        values.insert(field.id().to_owned(), value);
    }
    Ok(values)
}

fn parse_field_value(field: &ConfigField, raw: &str) -> Option<Value> {
    match field {
        ConfigField::Boolean { .. } => raw.trim().parse::<bool>().ok().map(Value::Bool),
        ConfigField::String { .. } | ConfigField::SingleSelect { .. } => {
            Some(Value::String(raw.trim().to_owned()))
        }
        ConfigField::Integer { .. } => raw.trim().parse::<i64>().ok().map(Value::from),
        ConfigField::Decimal { .. } => raw.trim().parse::<f64>().ok().map(Value::from),
        ConfigField::MultiSelect { .. } => Some(Value::Array(
            raw.split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| Value::String(value.to_owned()))
                .collect(),
        )),
    }
}

fn parse_config(text: &str) -> HashMap<(String, String), String> {
    let mut section = String::new();
    let mut values = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].trim().to_ascii_lowercase();
        } else if !trimmed.starts_with('#') && !trimmed.starts_with(';') {
            if let Some((key, value)) = trimmed.split_once('=') {
                values.insert(
                    (section.clone(), key.trim().to_ascii_lowercase()),
                    value.trim().to_owned(),
                );
            }
        }
    }
    values
}

fn update_config_file(path: &Path, changes: &[(&ConfigField, String)]) -> AppResult<()> {
    let existing = if path.exists() {
        let metadata = fs::metadata(path).map_err(|_| {
            mod_error(
                "mod_config_error",
                "the mod configuration could not be inspected",
            )
        })?;
        if metadata.len() > CONFIG_MAX_BYTES {
            return Err(mod_error(
                "mod_config_error",
                "the mod configuration was too large",
            ));
        }
        fs::read_to_string(path).map_err(|_| {
            mod_error(
                "mod_config_error",
                "the mod configuration could not be read",
            )
        })?
    } else {
        String::new()
    };
    let mut lines: Vec<String> = existing.lines().map(str::to_owned).collect();
    for (field, value) in changes {
        let (wanted_section, wanted_key) = field.section_key();
        let mut section = String::new();
        let mut replaced = false;
        for line in &mut lines {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                section = trimmed[1..trimmed.len() - 1].trim().to_owned();
            } else if section.eq_ignore_ascii_case(wanted_section)
                && !trimmed.starts_with('#')
                && !trimmed.starts_with(';')
            {
                if let Some((key, _)) = trimmed.split_once('=') {
                    if key.trim().eq_ignore_ascii_case(wanted_key) {
                        *line = format!("{wanted_key} = {value}");
                        replaced = true;
                        break;
                    }
                }
            }
        }
        if !replaced {
            let section_end = lines
                .iter()
                .position(|line| {
                    let trimmed = line.trim();
                    trimmed.starts_with('[')
                        && trimmed.ends_with(']')
                        && trimmed[1..trimmed.len() - 1]
                            .trim()
                            .eq_ignore_ascii_case(wanted_section)
                })
                .map(|section_start| {
                    lines[section_start + 1..]
                        .iter()
                        .position(|line| {
                            let trimmed = line.trim();
                            trimmed.starts_with('[') && trimmed.ends_with(']')
                        })
                        .map_or(lines.len(), |offset| section_start + 1 + offset)
                });
            if let Some(section_end) = section_end {
                lines.insert(section_end, format!("{wanted_key} = {value}"));
            } else {
                if !lines.is_empty() && !lines.last().is_some_and(String::is_empty) {
                    lines.push(String::new());
                }
                lines.push(format!("[{wanted_section}]"));
                lines.push(format!("{wanted_key} = {value}"));
            }
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            mod_error(
                "mod_config_error",
                "the mod config directory could not be created",
            )
        })?;
    }
    let parent = path
        .parent()
        .ok_or_else(|| mod_error("mod_config_error", "the mod config path was invalid"))?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|_| {
        mod_error(
            "mod_config_error",
            "a temporary config file could not be created",
        )
    })?;
    let mut output = lines.join("\n");
    output.push('\n');
    temporary
        .write_all(output.as_bytes())
        .and_then(|_| temporary.as_file_mut().sync_all())
        .map_err(|_| {
            mod_error(
                "mod_config_error",
                "the mod configuration could not be written",
            )
        })?;
    temporary.persist(path).map_err(|_| {
        mod_error(
            "mod_config_error",
            "the mod configuration could not be committed",
        )
    })?;
    Ok(())
}

fn catalog_client() -> AppResult<Client> {
    Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(5 * 60))
        .redirect(Policy::custom(|attempt| {
            if trusted_catalog_url(attempt.url()) && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .user_agent("GameTweaks game catalog")
        .build()
        .map_err(|_| {
            mod_error(
                "catalog_unavailable",
                "the catalog network client could not be created",
            )
        })
}

fn trusted_catalog_url(url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some(
                "raw.githubusercontent.com"
                    | "github.com"
                    | "objects.githubusercontent.com"
                    | "release-assets.githubusercontent.com"
            )
        )
}

async fn read_response_limited(mut response: reqwest::Response) -> AppResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > CATALOG_MAX_BYTES as u64)
    {
        return Err(mod_error(
            "catalog_invalid",
            "a catalog response was too large",
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| mod_error("catalog_unavailable", "a catalog response was interrupted"))?
    {
        if body.len().saturating_add(chunk.len()) > CATALOG_MAX_BYTES {
            return Err(mod_error(
                "catalog_invalid",
                "a catalog response was too large",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogCacheFile {
    schema_version: u32,
    catalog: Value,
    definitions: Vec<Value>,
}

fn persist_catalog_cache(
    app: &AppHandle,
    app_id: u32,
    catalog: &GameCatalog,
    definitions: &HashMap<String, ModDefinition>,
) -> AppResult<()> {
    let root = app
        .path()
        .app_cache_dir()
        .map_err(|_| {
            mod_error(
                "catalog_unavailable",
                "the catalog cache location was unavailable",
            )
        })?
        .join("games");
    fs::create_dir_all(&root).map_err(|_| {
        mod_error(
            "catalog_unavailable",
            "the catalog cache could not be created",
        )
    })?;
    let cache = CatalogCacheFile {
        schema_version: 1,
        catalog: serde_json::to_value(catalog)
            .map_err(|_| mod_error("catalog_invalid", "the game catalog could not be cached"))?,
        definitions: definitions
            .values()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| mod_error("catalog_invalid", "mod definitions could not be cached"))?,
    };
    let path = root.join(format!("{app_id}.json"));
    let mut temporary = NamedTempFile::new_in(&root).map_err(|_| {
        mod_error(
            "catalog_unavailable",
            "a catalog cache file could not be created",
        )
    })?;
    serde_json::to_writer(&mut temporary, &cache)
        .and_then(|_| temporary.write_all(b"\n").map_err(serde_json::Error::io))
        .map_err(|_| {
            mod_error(
                "catalog_unavailable",
                "the catalog cache could not be written",
            )
        })?;
    temporary.persist(path).map_err(|_| {
        mod_error(
            "catalog_unavailable",
            "the catalog cache could not be committed",
        )
    })?;
    Ok(())
}

fn read_catalog_cache(
    app: &AppHandle,
    app_id: u32,
) -> Option<(GameCatalog, HashMap<String, ModDefinition>)> {
    let path = app
        .path()
        .app_cache_dir()
        .ok()?
        .join("games")
        .join(format!("{app_id}.json"));
    let metadata = fs::metadata(&path).ok()?;
    if metadata.len() > CATALOG_MAX_BYTES as u64 * 4 {
        return None;
    }
    let cache: CatalogCacheFile = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if cache.schema_version != 1 {
        return None;
    }
    let catalog: GameCatalog = serde_json::from_value(cache.catalog).ok()?;
    validate_catalog(app_id, &catalog).ok()?;
    let mut definitions = HashMap::new();
    for value in cache.definitions {
        let definition: ModDefinition = serde_json::from_value(value).ok()?;
        validate_mod_definition(&definition).ok()?;
        definitions.insert(definition.mod_id.clone(), definition);
    }
    validate_relations(&definitions).ok()?;
    Some((catalog, definitions))
}

async fn consume_plan(state: &AppState, plan_id: &str) -> AppResult<PreparedAction> {
    let mut mods = state.game_mods.lock().await;
    let plan = mods
        .pending
        .remove(plan_id)
        .filter(|plan| plan.expires_at > Instant::now())
        .ok_or_else(|| mod_error("mod_plan_expired", "the action plan was missing or expired"))?;
    Ok(plan.action)
}

pub(crate) async fn mark_busy(state: &AppState, app_id: u32) -> AppResult<()> {
    if !state.game_mods.lock().await.busy_games.insert(app_id) {
        return Err(mod_error(
            "mod_busy",
            "another mod action is already running",
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) async fn clear_busy(state: &AppState, app_id: u32) {
    state.game_mods.lock().await.busy_games.remove(&app_id);
}

fn prune_plans(plans: &mut HashMap<String, PreparedPlan>) {
    plans.retain(|_, plan| plan.expires_at > Instant::now());
}

fn mod_directory(target: &InstallTarget, mod_id: &str) -> PathBuf {
    target
        .game_root
        .join("BepInEx/plugins/GameTweaks")
        .join(mod_id)
}

fn config_path(target: &InstallTarget, guid: &str) -> PathBuf {
    target
        .game_root
        .join("BepInEx/config")
        .join(format!("{guid}.cfg"))
}

fn read_mod_marker(directory: &Path) -> Option<ModInstallMarker> {
    let directory_metadata = fs::symlink_metadata(directory).ok()?;
    if !directory_metadata.is_dir() || directory_metadata.file_type().is_symlink() {
        return None;
    }
    let path = directory.join(".gametweaks-mod.json");
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return None;
    }
    let marker: ModInstallMarker = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if marker.schema_version != 1
        || !valid_identifier(&marker.mod_id)
        || !valid_guid(&marker.guid)
        || Version::parse(&marker.version).is_err()
        || !valid_sha256(&marker.sha256)
        || marker.files.is_empty()
        || marker.files.len() > MOD_ARCHIVE_MAX_ENTRIES
        || marker.files.iter().any(|path| {
            validate_relative_path(Path::new(path)).is_err()
                || !regular_file_below(directory, Path::new(path))
        })
        || marker.files.iter().collect::<HashSet<_>>().len() != marker.files.len()
        || !marker
            .files
            .iter()
            .any(|path| path.to_ascii_lowercase().ends_with(".dll"))
    {
        return None;
    }
    Some(marker)
}

fn regular_file_below(root: &Path, relative: &Path) -> bool {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            return false;
        }
    }
    fs::symlink_metadata(current)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

fn stage_files_for_removal(moves: &[(PathBuf, PathBuf)], code: &'static str) -> AppResult<()> {
    let mut staged = Vec::new();
    for (source, destination) in moves {
        let metadata = match fs::symlink_metadata(source) {
            Ok(metadata) => metadata,
            Err(_) => {
                rollback_staged_files(&staged);
                return Err(mod_error(
                    code,
                    "a managed file disappeared before it could be removed",
                ));
            }
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            rollback_staged_files(&staged);
            return Err(mod_error(
                code,
                "a managed file was no longer a regular file",
            ));
        }
        if let Some(parent) = destination.parent() {
            if fs::create_dir_all(parent).is_err() {
                rollback_staged_files(&staged);
                return Err(mod_error(
                    code,
                    "an uninstall staging directory could not be created",
                ));
            }
        }
        if fs::rename(source, destination).is_err() {
            rollback_staged_files(&staged);
            return Err(mod_error(
                code,
                "a managed file could not be staged for removal",
            ));
        }
        staged.push((source.clone(), destination.clone()));
    }
    Ok(())
}

fn rollback_staged_files(staged: &[(PathBuf, PathBuf)]) {
    for (source, destination) in staged.iter().rev() {
        if let Some(parent) = source.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::rename(destination, source);
    }
}

fn prune_empty_directories(path: &Path, stop: &Path) {
    if !path.exists() || path == stop {
        return;
    }
    if path.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            let children: Vec<_> = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect();
            for child in children {
                if child.is_dir() {
                    prune_empty_directories(&child, stop);
                }
            }
        }
        let _ = fs::remove_dir(path);
    }
}

fn safe_join(root: &Path, relative: &str) -> AppResult<PathBuf> {
    let path = Path::new(relative);
    validate_relative_path(path)?;
    Ok(root.join(path))
}

fn validate_relative_path(path: &Path) -> AppResult<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(mod_error("mod_integrity_error", "a mod path was unsafe"));
    }
    Ok(())
}

fn write_json_new(path: &Path, value: &impl Serialize) -> AppResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            mod_error(
                "mod_install_error",
                "a GameTweaks marker could not be created",
            )
        })?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|_| {
        mod_error(
            "mod_install_error",
            "a GameTweaks marker could not be written",
        )
    })?;
    file.write_all(b"\n")
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            mod_error(
                "mod_install_error",
                "a GameTweaks marker could not be finalized",
            )
        })
}

fn emit_mod_progress(
    app: &AppHandle,
    app_id: u32,
    mod_id: &str,
    stage: ModProgressStage,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    let percentage = total_bytes
        .filter(|total| *total > 0)
        .map(|total| ((downloaded_bytes.saturating_mul(100)) / total).min(100));
    let _ = app.emit(
        MOD_PROGRESS_EVENT,
        ModProgress {
            app_id,
            mod_id: mod_id.to_owned(),
            stage,
            downloaded_bytes,
            total_bytes,
            percentage,
        },
    );
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_guid(value: &str) -> bool {
    valid_identifier(value) && value.contains('.')
}

fn valid_release_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 180
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
}

fn valid_repository(value: &str) -> bool {
    let Some((owner, repository)) = value.split_once('/') else {
        return false;
    };
    !repository.contains('/')
        && [owner, repository].into_iter().all(|segment| {
            !segment.is_empty()
                && segment.len() <= 100
                && segment
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
}

fn catalog_release_tag(definition: &ModDefinition) -> String {
    format!("mod-{}-v{}", definition.mod_id, definition.version)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_localized_text(value: &LocalizedText) -> bool {
    let valid = |text: &str| !text.trim().is_empty() && text.len() <= 512 && !text.contains('\0');
    valid(&value.en) && value.de.as_deref().map_or(true, valid)
}

fn valid_config_name(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 128
        && !value
            .chars()
            .any(|character| matches!(character, '\r' | '\n' | '[' | ']' | '='))
}

fn sha256_hex(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}

fn sha256_file(path: &Path) -> AppResult<String> {
    let mut file = File::open(path).map_err(|_| {
        mod_error(
            "bepinex_uninstall_error",
            "a backup file could not be opened",
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| mod_error("bepinex_uninstall_error", "a backup file could not be read"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn random_token(code: &'static str) -> AppResult<String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| mod_error(code, "a secure action plan could not be created"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn mod_error(code: &'static str, message: impl Into<String>) -> ErrorResponse {
    ErrorResponse::from(AppError::GameMods {
        code,
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn text(value: &str) -> LocalizedText {
        LocalizedText {
            en: value.to_owned(),
            de: None,
        }
    }

    fn definition(mod_id: &str) -> ModDefinition {
        ModDefinition {
            schema_version: 1,
            mod_id: mod_id.to_owned(),
            guid: format!("dev.gametweaks.{mod_id}"),
            version: "1.0.0".to_owned(),
            official: false,
            name: text("Test mod"),
            description: text("Test description"),
            integration: ModIntegration::ConfigFile,
            compatibility: ModCompatibility {
                runtime: BepInExRuntime::Mono,
                architectures: vec![BepInExArchitecture::X64],
                minimum_agent_version: None,
            },
            release: ModRelease {
                repository: "gametweaks/test-mod".to_owned(),
                tag: "test-1.0.0".to_owned(),
                asset: "test.zip".to_owned(),
                sha256: "a".repeat(64),
            },
            dependencies: Vec::new(),
            conflicts: Vec::new(),
            config: Vec::new(),
        }
    }

    fn target(root: &Path) -> InstallTarget {
        InstallTarget {
            game_root: root.to_path_buf(),
            executable: root.join("game.exe"),
            runtime: BepInExRuntime::Mono,
            architecture: BepInExArchitecture::X64,
        }
    }

    fn install_test_mod(target: &InstallTarget, definition: &ModDefinition, version: &str) {
        let directory = mod_directory(target, &definition.mod_id);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("plugin.dll"), b"plugin").unwrap();
        write_json_new(
            &directory.join(".gametweaks-mod.json"),
            &ModInstallMarker {
                schema_version: 1,
                app_id: 10,
                mod_id: definition.mod_id.clone(),
                guid: definition.guid.clone(),
                version: version.to_owned(),
                sha256: "a".repeat(64),
                files: vec!["plugin.dll".to_owned()],
            },
        )
        .unwrap();
    }

    #[test]
    fn resolves_dependencies_before_requested_mod() {
        let base = definition("base");
        let mut feature = definition("feature");
        feature.dependencies.push(ModDependency {
            mod_id: "base".to_owned(),
            minimum_version: "1.0.0".to_owned(),
        });
        let definitions = HashMap::from([
            (base.mod_id.clone(), base),
            (feature.mod_id.clone(), feature),
        ]);
        assert_eq!(
            resolve_install_set(&definitions, &["feature".to_owned()]).unwrap(),
            vec!["base", "feature"]
        );
    }

    #[test]
    fn preserves_the_reviewed_official_status() {
        let mut definition = definition("official.mod");
        definition.official = true;

        let described = describe_mod(None, 10, &definition).unwrap();

        assert!(described.official);
        assert!(!described.external);
    }

    #[test]
    fn derives_the_immutable_catalog_release_tag() {
        let definition = definition("author.mod");

        assert_eq!(catalog_release_tag(&definition), "mod-author.mod-v1.0.0");
        assert!(validate_mod_definition(&definition).is_ok());
    }

    #[test]
    fn rejects_invalid_upstream_repositories() {
        let mut definition = definition("author.mod");
        definition.release.repository = "https://example.com/mod".to_owned();

        assert!(validate_mod_definition(&definition).is_err());
    }

    #[tokio::test]
    async fn install_plans_are_single_use() {
        let state = AppState::default();
        state.game_mods.lock().await.pending.insert(
            "single-use".to_owned(),
            PreparedPlan {
                action: PreparedAction::UninstallBepInEx { app_id: 10 },
                expires_at: Instant::now() + Duration::from_secs(60),
            },
        );

        assert!(consume_plan(&state, "single-use").await.is_ok());
        assert!(consume_plan(&state, "single-use").await.is_err());
    }

    #[tokio::test]
    async fn expired_install_plans_are_rejected() {
        let state = AppState::default();
        state.game_mods.lock().await.pending.insert(
            "expired".to_owned(),
            PreparedPlan {
                action: PreparedAction::UninstallBepInEx { app_id: 10 },
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );

        assert!(consume_plan(&state, "expired").await.is_err());
    }

    #[test]
    fn rejects_dependency_cycles() {
        let mut one = definition("one");
        let mut two = definition("two");
        one.dependencies.push(ModDependency {
            mod_id: "two".to_owned(),
            minimum_version: "1.0.0".to_owned(),
        });
        two.dependencies.push(ModDependency {
            mod_id: "one".to_owned(),
            minimum_version: "1.0.0".to_owned(),
        });
        let definitions = HashMap::from([(one.mod_id.clone(), one), (two.mod_id.clone(), two)]);
        assert!(resolve_install_set(&definitions, &["one".to_owned()]).is_err());
    }

    #[test]
    fn skips_satisfied_dependencies_in_install_plan() {
        let directory = tempfile::tempdir().unwrap();
        let target = target(directory.path());
        let base = definition("base");
        install_test_mod(&target, &base, "1.0.0");
        let mut feature = definition("feature");
        feature.dependencies.push(ModDependency {
            mod_id: "base".to_owned(),
            minimum_version: "1.0.0".to_owned(),
        });
        let definitions = HashMap::from([
            (base.mod_id.clone(), base),
            (feature.mod_id.clone(), feature),
        ]);
        let requested = vec!["feature".to_owned()];
        let resolved = resolve_install_set(&definitions, &requested).unwrap();
        let (actions, replacements) =
            resolve_action_set(10, &target, &definitions, &requested, &resolved, false).unwrap();
        assert_eq!(actions, vec!["feature"]);
        assert!(replacements.is_empty());
    }

    #[test]
    fn update_plan_installs_a_missing_dependency() {
        let directory = tempfile::tempdir().unwrap();
        let target = target(directory.path());
        let base = definition("base");
        let mut feature = definition("feature");
        feature.dependencies.push(ModDependency {
            mod_id: "base".to_owned(),
            minimum_version: "1.0.0".to_owned(),
        });
        install_test_mod(&target, &feature, "0.9.0");
        let definitions = HashMap::from([
            (base.mod_id.clone(), base),
            (feature.mod_id.clone(), feature),
        ]);
        let requested = vec!["feature".to_owned()];
        let resolved = resolve_install_set(&definitions, &requested).unwrap();
        let (actions, replacements) =
            resolve_action_set(10, &target, &definitions, &requested, &resolved, true).unwrap();
        assert_eq!(actions, vec!["base", "feature"]);
        assert_eq!(replacements, HashSet::from(["feature".to_owned()]));
    }

    #[test]
    fn validates_all_config_control_values() {
        let field = ConfigField::Integer {
            id: "count".to_owned(),
            section: "General".to_owned(),
            key: "Count".to_owned(),
            label: text("Count"),
            description: None,
            locked: false,
            default: 2,
            min: 0,
            max: 10,
            step: 2,
            apply_mode: ConfigApplyMode::NextLaunch,
        };
        assert_eq!(field.serialize_value(&Value::from(8)).unwrap(), "8");
        assert!(field.serialize_value(&Value::from(9)).is_err());
    }

    #[test]
    fn config_writer_preserves_unknown_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plugin.cfg");
        fs::write(
            &path,
            "# comment\n[General]\nEnabled = false\nUnknown = keep\n",
        )
        .unwrap();
        let field = ConfigField::Boolean {
            id: "enabled".to_owned(),
            section: "General".to_owned(),
            key: "Enabled".to_owned(),
            label: text("Enabled"),
            description: None,
            locked: false,
            default: false,
            apply_mode: ConfigApplyMode::Live,
            display: BooleanDisplay::Switch,
        };
        update_config_file(&path, &[(&field, "true".to_owned())]).unwrap();
        let updated = fs::read_to_string(path).unwrap();
        assert!(updated.contains("# comment"));
        assert!(updated.contains("Enabled = true"));
        assert!(updated.contains("Unknown = keep"));
    }

    #[test]
    fn config_writer_reuses_an_existing_section_for_multiple_new_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plugin.cfg");
        fs::write(&path, "[General]\nUnknown = keep\n[Other]\nValue = keep\n").unwrap();
        let first = ConfigField::Boolean {
            id: "first".to_owned(),
            section: "General".to_owned(),
            key: "First".to_owned(),
            label: text("First"),
            description: None,
            locked: false,
            default: false,
            apply_mode: ConfigApplyMode::Live,
            display: BooleanDisplay::Switch,
        };
        let second = ConfigField::Boolean {
            id: "second".to_owned(),
            section: "General".to_owned(),
            key: "Second".to_owned(),
            label: text("Second"),
            description: None,
            locked: false,
            default: false,
            apply_mode: ConfigApplyMode::Live,
            display: BooleanDisplay::Switch,
        };

        update_config_file(
            &path,
            &[(&first, "true".to_owned()), (&second, "false".to_owned())],
        )
        .unwrap();

        let updated = fs::read_to_string(path).unwrap();
        assert_eq!(updated.matches("[General]").count(), 1);
        assert!(updated.contains("First = true"));
        assert!(updated.contains("Second = false"));
        assert!(updated.find("Second = false").unwrap() < updated.find("[Other]").unwrap());
    }

    #[test]
    fn rejects_zip_slip_and_requires_a_dll() {
        let directory = tempfile::tempdir().unwrap();
        let archive_path = directory.path().join("bad.zip");
        let file = File::create(&archive_path).unwrap();
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("../escape.dll", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"dll").unwrap();
        writer.finish().unwrap();
        let output = directory.path().join("output");
        fs::create_dir(&output).unwrap();
        assert!(extract_mod_archive(&archive_path, &output).is_err());
    }

    #[test]
    fn incomplete_managed_mod_is_not_reported_as_installed() {
        let directory = tempfile::tempdir().unwrap();
        let mod_directory = directory.path().join("example.mod");
        fs::create_dir(&mod_directory).unwrap();
        fs::write(mod_directory.join("plugin.dll"), b"plugin").unwrap();
        let marker = ModInstallMarker {
            schema_version: 1,
            app_id: 10,
            mod_id: "example.mod".to_owned(),
            guid: "dev.example.mod".to_owned(),
            version: "1.0.0".to_owned(),
            sha256: "a".repeat(64),
            files: vec!["plugin.dll".to_owned()],
        };
        write_json_new(&mod_directory.join(".gametweaks-mod.json"), &marker).unwrap();
        assert!(read_mod_marker(&mod_directory).is_some());

        fs::remove_file(mod_directory.join("plugin.dll")).unwrap();
        assert!(read_mod_marker(&mod_directory).is_none());
    }

    #[test]
    fn uninstall_staging_rolls_back_after_a_late_failure() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("plugin.dll");
        fs::write(&source, b"plugin").unwrap();
        let missing = directory.path().join("missing.dll");
        let staging = directory.path().join("staging");
        let moves = vec![
            (source.clone(), staging.join("plugin.dll")),
            (missing, staging.join("missing.dll")),
        ];

        assert!(stage_files_for_removal(&moves, "test_error").is_err());
        assert_eq!(fs::read(source).unwrap(), b"plugin");
    }

    #[test]
    fn rejects_duplicate_selection_options() {
        let field = ConfigField::SingleSelect {
            id: "mode".to_owned(),
            section: "General".to_owned(),
            key: "Mode".to_owned(),
            label: text("Mode"),
            description: None,
            locked: false,
            default: "same".to_owned(),
            options: vec![
                SelectOption {
                    value: "same".to_owned(),
                    label: text("First"),
                },
                SelectOption {
                    value: "same".to_owned(),
                    label: text("Second"),
                },
            ],
            apply_mode: ConfigApplyMode::Live,
            display: SelectionDisplay::Dropdown,
        };
        assert!(!field.valid_for_agent());
    }
}
