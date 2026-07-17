use std::collections::HashMap;
#[cfg(windows)]
use std::fs::File;
use std::fs::{self, OpenOptions};
#[cfg(any(windows, test))]
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

#[cfg(any(windows, test))]
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(any(windows, test))]
use sha2::Sha256;
#[cfg(windows)]
use tauri::Emitter;
use tauri::{AppHandle, Manager};
#[cfg(windows)]
use tempfile::Builder as TempBuilder;
use tokio::sync::oneshot;

#[cfg(windows)]
use crate::bepinex::{
    analyze_windows_installation, ensure_game_stopped, ensure_no_anti_cheat, resolve_game,
    BepInExAvailability,
};
use crate::bepinex::{BepInExRuntime, InstallTarget};
use crate::config::model::developer_mode_enabled;
use crate::core::error::{AppError, AppResult, ErrorResponse};
use crate::core::state::AppState;
use crate::game_mods::{ConfigField, GameMod, GameModStatus, LocalizedText, ModIntegration};
#[cfg(windows)]
use crate::steam::discover_installed_games;

const PROTOCOL_VERSION: u32 = 1;
const AGENT_VERSION: &str = "0.1.0";
#[cfg(windows)]
const PIPE_NAME: &str = r"\\.\pipe\GameTweaks.Agent.v1";
#[cfg(any(windows, test))]
const MAX_FRAME_BYTES: usize = 1024 * 1024;
#[cfg(windows)]
const AGENT_STATE_EVENT: &str = "gametweaks-agent-state";
#[cfg(windows)]
const AGENT_CONFIG_EVENT: &str = "gametweaks-agent-config-changed";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Incompatible,
    Ambiguous,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentMarker {
    schema_version: u32,
    app_id: u32,
    version: String,
    runtime: BepInExRuntime,
    secret: String,
    files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentModSnapshot {
    mod_id: String,
    version: String,
    name: LocalizedText,
    description: LocalizedText,
    #[serde(default)]
    fields: Vec<ConfigField>,
    #[serde(default)]
    values: HashMap<String, Value>,
}

#[cfg(any(windows, test))]
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
enum IncomingFrame {
    Hello {
        protocol_version: u32,
        app_id: u32,
        process_id: u32,
        instance_id: String,
        runtime: BepInExRuntime,
        agent_version: String,
        proof: String,
    },
    Snapshot {
        protocol_version: u32,
        app_id: u32,
        instance_id: String,
        mods: Vec<AgentModSnapshot>,
    },
    ConfigChanged {
        protocol_version: u32,
        app_id: u32,
        instance_id: String,
        mod_id: String,
        values: HashMap<String, Value>,
    },
    ConfigResult {
        protocol_version: u32,
        request_id: String,
        accepted: bool,
        error_code: Option<String>,
        #[serde(default)]
        restart_required: bool,
    },
}

#[derive(Debug, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
enum OutgoingFrame {
    #[cfg(windows)]
    Challenge {
        protocol_version: u32,
        challenge: String,
    },
    #[cfg(windows)]
    HelloAck {
        protocol_version: u32,
        accepted: bool,
        error_code: Option<&'static str>,
    },
    SetConfig {
        protocol_version: u32,
        request_id: String,
        mod_id: String,
        values: HashMap<String, Value>,
    },
}

struct OutboundMessage {
    #[cfg_attr(not(windows), allow(dead_code))]
    frame: OutgoingFrame,
}

#[derive(Clone)]
struct AgentConnection {
    #[cfg_attr(not(windows), allow(dead_code))]
    instance_id: String,
    sender: mpsc::Sender<OutboundMessage>,
    mods: Vec<AgentModSnapshot>,
}

#[derive(Default)]
pub struct AgentState {
    connections: HashMap<u32, Vec<AgentConnection>>,
    statuses: HashMap<u32, AgentConnectionStatus>,
    pending: HashMap<String, oneshot::Sender<Result<bool, String>>>,
}

pub fn start_server(app: AppHandle) {
    #[cfg(windows)]
    if let Err(error) = std::thread::Builder::new()
        .name("gametweaks-agent-pipe".to_owned())
        .spawn(move || windows_pipe::run(app))
    {
        tracing::error!(%error, "failed to start the GameTweaks agent pipe server");
    }

    #[cfg(not(windows))]
    let _ = app;
}

pub async fn connection_status(state: &AppState, app_id: u32) -> AgentConnectionStatus {
    let agent = state.agent.lock().await;
    match agent.connections.get(&app_id).map(Vec::len) {
        Some(1) => AgentConnectionStatus::Connected,
        Some(count) if count > 1 => AgentConnectionStatus::Ambiguous,
        _ => agent.statuses.get(&app_id).copied().unwrap_or_default(),
    }
}

pub async fn is_connected(state: &AppState, app_id: u32) -> bool {
    connection_status(state, app_id).await == AgentConnectionStatus::Connected
}

pub async fn external_mods(state: &AppState, app_id: u32) -> Vec<GameMod> {
    let agent = state.agent.lock().await;
    let Some(connections) = agent.connections.get(&app_id) else {
        return Vec::new();
    };
    if connections.len() != 1 {
        return Vec::new();
    }
    connections[0]
        .mods
        .iter()
        .map(snapshot_to_external_mod)
        .collect()
}

pub async fn overlay_runtime_mods(state: &AppState, app_id: u32, mods: &mut [GameMod]) {
    let agent = state.agent.lock().await;
    let Some(connection) = agent
        .connections
        .get(&app_id)
        .filter(|connections| connections.len() == 1)
        .and_then(|connections| connections.first())
    else {
        return;
    };
    for game_mod in mods {
        let Some(snapshot) = connection
            .mods
            .iter()
            .find(|snapshot| snapshot.mod_id == game_mod.mod_id)
        else {
            continue;
        };
        for field in &snapshot.fields {
            let Some(known) = game_mod
                .config
                .iter_mut()
                .find(|known| known.id_for_agent() == field.id_for_agent())
            else {
                if field.valid_for_agent() {
                    game_mod.config.push(field.clone());
                }
                continue;
            };
            if !same_field_contract(known, field) {
                known.set_locked(true);
            }
        }
        for (id, value) in &snapshot.values {
            if !game_mod
                .config
                .iter()
                .any(|field| field.id_for_agent() == id && field.is_locked())
            {
                game_mod.values.insert(id.clone(), value.clone());
            }
        }
    }
}

fn same_field_contract(left: &ConfigField, right: &ConfigField) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.set_locked(false);
    right.set_locked(false);
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

pub async fn runtime_fields(
    state: &AppState,
    app_id: u32,
    mod_id: &str,
) -> Option<Vec<ConfigField>> {
    let agent = state.agent.lock().await;
    agent
        .connections
        .get(&app_id)
        .filter(|connections| connections.len() == 1)
        .and_then(|connections| connections.first())
        .and_then(|connection| {
            connection
                .mods
                .iter()
                .find(|snapshot| snapshot.mod_id == mod_id)
        })
        .map(|snapshot| snapshot.fields.clone())
}

pub(crate) fn merge_runtime_fields(fields: &mut Vec<ConfigField>, dynamic: &[ConfigField]) {
    for field in dynamic.iter().filter(|field| field.valid_for_agent()) {
        if let Some(known) = fields
            .iter_mut()
            .find(|known| known.id_for_agent() == field.id_for_agent())
        {
            if !same_field_contract(known, field) {
                known.set_locked(true);
            }
        } else {
            fields.push(field.clone());
        }
    }
}

fn snapshot_to_external_mod(snapshot: &AgentModSnapshot) -> GameMod {
    GameMod {
        mod_id: snapshot.mod_id.clone(),
        guid: snapshot.mod_id.clone(),
        version: snapshot.version.clone(),
        installed_version: Some(snapshot.version.clone()),
        official: false,
        external: true,
        name: snapshot.name.clone(),
        description: snapshot.description.clone(),
        integration: ModIntegration::Agent,
        status: GameModStatus::External,
        dependencies: Vec::new(),
        conflicts: Vec::new(),
        config: snapshot
            .fields
            .iter()
            .filter(|field| field.valid_for_agent())
            .cloned()
            .collect(),
        values: snapshot.values.clone(),
        restart_required: false,
    }
}

pub async fn set_config(
    state: &AppState,
    app_id: u32,
    mod_id: &str,
    values: &HashMap<String, Value>,
) -> AppResult<bool> {
    let (sender, receiver, request_id) = {
        let mut agent = state.agent.lock().await;
        let connections = agent
            .connections
            .get(&app_id)
            .filter(|connections| connections.len() == 1)
            .ok_or_else(|| {
                agent_error(
                    "agent_not_connected",
                    "the game agent is not uniquely connected",
                )
            })?;
        let sender = connections[0].sender.clone();
        let request_id = random_hex(16)?;
        let (result_sender, receiver) = oneshot::channel();
        agent.pending.insert(request_id.clone(), result_sender);
        (sender, receiver, request_id)
    };
    if sender
        .send(OutboundMessage {
            frame: OutgoingFrame::SetConfig {
                protocol_version: PROTOCOL_VERSION,
                request_id: request_id.clone(),
                mod_id: mod_id.to_owned(),
                values: values.clone(),
            },
        })
        .is_err()
    {
        state.agent.lock().await.pending.remove(&request_id);
        return Err(agent_error(
            "agent_disconnected",
            "the game agent disconnected",
        ));
    }
    match tokio::time::timeout(Duration::from_secs(5), receiver).await {
        Ok(Ok(Ok(restart_required))) => Ok(restart_required),
        Ok(Ok(Err(_))) => Err(agent_error(
            "agent_config_rejected",
            "the mod rejected a configuration change",
        )),
        _ => {
            state.agent.lock().await.pending.remove(&request_id);
            Err(agent_error(
                "agent_timeout",
                "the game agent did not confirm the configuration change",
            ))
        }
    }
}

pub(crate) fn agent_is_current(target: &InstallTarget, app_id: u32) -> bool {
    valid_agent_install(target)
        .is_some_and(|marker| marker.app_id == app_id && marker.version == AGENT_VERSION)
}

pub(crate) fn agent_is_installed(target: &InstallTarget, app_id: u32) -> bool {
    valid_agent_install(target).is_some_and(|marker| marker.app_id == app_id)
}

pub(crate) fn bundled_agent_meets(minimum: Option<&str>) -> bool {
    minimum.is_none_or(|minimum| {
        semver::Version::parse(AGENT_VERSION).ok() >= semver::Version::parse(minimum).ok()
    })
}

pub async fn install_development_agent(
    app: &AppHandle,
    state: &AppState,
    app_id: u32,
) -> AppResult<()> {
    let developer_mode = {
        let config = state.config.read().await;
        developer_mode_enabled(&config)
    };
    if !developer_mode {
        return Err(agent_error(
            "developer_mode_required",
            "developer mode is required to install the development agent",
        ));
    }

    #[cfg(not(windows))]
    {
        let _ = (app, app_id);
        Err(agent_error(
            "agent_unsupported",
            "the GameTweaks agent is currently available on Windows only",
        ))
    }

    #[cfg(windows)]
    {
        crate::game_mods::mark_busy(state, app_id).await?;
        let result = install_development_agent_inner(app, app_id).await;
        crate::game_mods::clear_busy(state, app_id).await;
        result
    }
}

#[cfg(windows)]
async fn install_development_agent_inner(app: &AppHandle, app_id: u32) -> AppResult<()> {
    let game = resolve_game(app_id).await?;
    let (status, target) = analyze_windows_installation(&game.install_directory).map_err(|_| {
        agent_error(
            "agent_install_blocked",
            "the game installation is not compatible with the development agent",
        )
    })?;
    if status.status != BepInExAvailability::Installed {
        return Err(agent_error(
            "mod_requires_bepinex",
            "BepInEx must be installed before the development agent",
        ));
    }
    ensure_no_anti_cheat(&target.game_root)?;
    ensure_game_stopped(&target.executable)?;

    let staging = TempBuilder::new()
        .prefix(".gametweaks-agent-")
        .tempdir_in(&target.game_root)
        .map_err(|_| {
            agent_error(
                "agent_install_error",
                "a development agent staging directory could not be created",
            )
        })?;
    stage_bundled_agent(app, &target, staging.path())?;
    ensure_game_stopped(&target.executable)?;

    let rollback_root = staging.path().join("rollback");
    fs::create_dir(&rollback_root).map_err(|_| {
        agent_error(
            "agent_install_error",
            "the development agent rollback directory could not be created",
        )
    })?;
    let Some((_, previous)) = commit_staged_agent(&target, staging.path(), app_id, &rollback_root)?
    else {
        return Err(agent_error(
            "agent_install_error",
            "the development agent was not staged",
        ));
    };
    if let Some(previous) = previous {
        if let Err(error) = fs::remove_dir_all(previous) {
            tracing::warn!(%error, "failed to clean up the previous development agent");
        }
    }
    Ok(())
}

pub(crate) fn stage_bundled_agent(
    app: &AppHandle,
    target: &InstallTarget,
    staging_root: &Path,
) -> AppResult<()> {
    let destination = staging_root.join("agent-content");
    if destination.exists() {
        return Ok(());
    }
    let runtime_name = match target.runtime {
        BepInExRuntime::Mono => "mono",
        BepInExRuntime::Il2Cpp => "il2cpp",
    };
    let resource_root = app
        .path()
        .resource_dir()
        .map_err(|_| {
            agent_error(
                "agent_unavailable",
                "the bundled agent resource directory was unavailable",
            )
        })?
        .join("agent")
        .join(runtime_name);
    let development_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../agent/artifacts")
        .join(runtime_name);
    let source = if resource_root.is_dir() {
        resource_root
    } else {
        development_root
    };
    if !source.is_dir() {
        return Err(agent_error(
            "agent_unavailable",
            "the runtime-specific GameTweaks agent has not been built",
        ));
    }
    copy_regular_tree(&source, &destination)?;
    let files = collect_relative_files(&destination)?;
    if !files
        .iter()
        .any(|file| file.to_ascii_lowercase().ends_with(".dll"))
    {
        return Err(agent_error(
            "agent_unavailable",
            "the bundled agent contained no DLL",
        ));
    }
    let marker = AgentMarker {
        schema_version: 1,
        app_id: 0,
        version: AGENT_VERSION.to_owned(),
        runtime: target.runtime,
        secret: random_hex(32)?,
        files,
    };
    write_marker(&destination.join(".gametweaks-agent.json"), &marker)
}

pub(crate) fn commit_staged_agent(
    target: &InstallTarget,
    staging_root: &Path,
    app_id: u32,
    rollback_root: &Path,
) -> AppResult<Option<(PathBuf, Option<PathBuf>)>> {
    let staged = staging_root.join("agent-content");
    if !staged.exists() {
        return Ok(None);
    }
    let marker_path = staged.join(".gametweaks-agent.json");
    let mut marker = read_agent_marker(&marker_path)
        .ok_or_else(|| agent_error("agent_unavailable", "the staged agent marker was invalid"))?;
    marker.app_id = app_id;
    fs::remove_file(&marker_path).map_err(|_| {
        agent_error(
            "agent_install_error",
            "the staged agent marker could not be updated",
        )
    })?;
    write_marker(&marker_path, &marker)?;
    let destination = agent_directory(target);
    let old = if destination.exists() {
        let installed = valid_agent_install(target).ok_or_else(|| {
            agent_error(
                "agent_collision",
                "an existing agent directory is not managed by GameTweaks",
            )
        })?;
        if installed.app_id != app_id {
            return Err(agent_error(
                "agent_collision",
                "the existing agent belongs to another game",
            ));
        }
        let old = rollback_root.join("GameTweaks.Agent");
        fs::rename(&destination, &old).map_err(|_| {
            agent_error(
                "agent_install_error",
                "the previous agent version could not be staged",
            )
        })?;
        Some(old)
    } else {
        None
    };
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|_| {
            agent_error(
                "agent_install_error",
                "the agent plugin directory could not be created",
            )
        })?;
    }
    if fs::rename(staged, &destination).is_err() {
        if let Some(old) = &old {
            let _ = fs::rename(old, &destination);
        }
        return Err(agent_error(
            "agent_install_error",
            "the agent could not be installed",
        ));
    }
    Ok(Some((destination, old)))
}

fn agent_directory(target: &InstallTarget) -> PathBuf {
    target.game_root.join("BepInEx/plugins/GameTweaks.Agent")
}

fn read_agent_marker(path: &Path) -> Option<AgentMarker> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return None;
    }
    let marker: AgentMarker = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if marker.schema_version != 1
        || marker.secret.len() != 64
        || !marker.secret.bytes().all(|byte| byte.is_ascii_hexdigit())
        || marker.files.is_empty()
    {
        return None;
    }
    Some(marker)
}

fn valid_agent_install(target: &InstallTarget) -> Option<AgentMarker> {
    let directory = agent_directory(target);
    let metadata = fs::symlink_metadata(&directory).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    let marker = read_agent_marker(&directory.join(".gametweaks-agent.json"))?;
    for relative in &marker.files {
        let relative = Path::new(relative);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return None;
        }
        let mut current = directory.clone();
        for component in relative.components() {
            current.push(component.as_os_str());
            let metadata = fs::symlink_metadata(&current).ok()?;
            if metadata.file_type().is_symlink() {
                return None;
            }
        }
        if !fs::symlink_metadata(current).ok()?.is_file() {
            return None;
        }
    }
    Some(marker)
}

fn write_marker(path: &Path, marker: &AgentMarker) -> AppResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            agent_error(
                "agent_install_error",
                "the agent marker could not be created",
            )
        })?;
    serde_json::to_writer_pretty(&mut file, marker).map_err(|_| {
        agent_error(
            "agent_install_error",
            "the agent marker could not be written",
        )
    })?;
    file.write_all(b"\n").map_err(|_| {
        agent_error(
            "agent_install_error",
            "the agent marker could not be finalized",
        )
    })
}

fn copy_regular_tree(source: &Path, destination: &Path) -> AppResult<()> {
    fs::create_dir(destination).map_err(|_| {
        agent_error(
            "agent_install_error",
            "the agent staging directory could not be created",
        )
    })?;
    let mut pending = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((from, to)) = pending.pop() {
        for entry in fs::read_dir(from)
            .map_err(|_| agent_error("agent_unavailable", "the bundled agent could not be read"))?
        {
            let entry = entry.map_err(|_| {
                agent_error(
                    "agent_unavailable",
                    "a bundled agent file could not be read",
                )
            })?;
            let kind = entry.file_type().map_err(|_| {
                agent_error("agent_unavailable", "a bundled agent file type was invalid")
            })?;
            if kind.is_symlink() {
                return Err(agent_error(
                    "agent_unavailable",
                    "the bundled agent contained a symlink",
                ));
            }
            let target = to.join(entry.file_name());
            if kind.is_dir() {
                fs::create_dir(&target).map_err(|_| {
                    agent_error(
                        "agent_install_error",
                        "an agent directory could not be created",
                    )
                })?;
                pending.push((entry.path(), target));
            } else if kind.is_file() {
                fs::copy(entry.path(), target).map_err(|_| {
                    agent_error("agent_install_error", "an agent file could not be copied")
                })?;
            }
        }
    }
    Ok(())
}

fn collect_relative_files(root: &Path) -> AppResult<Vec<String>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(|_| {
            agent_error(
                "agent_unavailable",
                "the agent staging directory could not be read",
            )
        })? {
            let entry = entry
                .map_err(|_| agent_error("agent_unavailable", "an agent file could not be read"))?;
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                pending.push(entry.path());
            } else {
                files.push(
                    entry
                        .path()
                        .strip_prefix(root)
                        .map_err(|_| {
                            agent_error("agent_unavailable", "an agent file path was unsafe")
                        })?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    files.sort();
    Ok(files)
}

fn random_hex(length: usize) -> AppResult<String> {
    let mut bytes = vec![0_u8; length];
    getrandom::getrandom(&mut bytes).map_err(|_| {
        agent_error(
            "agent_security_error",
            "secure agent randomness was unavailable",
        )
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(any(windows, test))]
fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

#[cfg(any(windows, test))]
fn proof(
    secret: &str,
    challenge: &str,
    app_id: u32,
    process_id: u32,
    instance_id: &str,
) -> Option<String> {
    let secret = decode_hex(secret)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&secret).ok()?;
    mac.update(challenge.as_bytes());
    mac.update(b"|");
    mac.update(app_id.to_string().as_bytes());
    mac.update(b"|");
    mac.update(process_id.to_string().as_bytes());
    mac.update(b"|");
    mac.update(instance_id.as_bytes());
    Some(
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn agent_error(code: &'static str, message: impl Into<String>) -> ErrorResponse {
    ErrorResponse::from(AppError::GameMods {
        code,
        message: message.into(),
    })
}

#[cfg(any(windows, test))]
fn write_frame(writer: &mut impl Write, frame: &OutgoingFrame) -> std::io::Result<()> {
    let raw = serde_json::to_vec(frame).map_err(std::io::Error::other)?;
    if raw.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "agent frame too large",
        ));
    }
    writer.write_all(&(raw.len() as u32).to_le_bytes())?;
    writer.write_all(&raw)?;
    writer.flush()
}

#[cfg(any(windows, test))]
fn read_frame(reader: &mut impl Read) -> std::io::Result<IncomingFrame> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid agent frame length",
        ));
    }
    let mut raw = vec![0_u8; length];
    reader.read_exact(&mut raw)?;
    serde_json::from_slice(&raw).map_err(std::io::Error::other)
}

#[cfg(any(windows, test))]
fn valid_snapshot(snapshot: &AgentModSnapshot) -> bool {
    !snapshot.mod_id.is_empty()
        && snapshot.mod_id.len() <= 96
        && snapshot
            .mod_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !snapshot.version.is_empty()
        && snapshot.version.len() <= 64
        && snapshot.fields.len() <= 512
        && snapshot.values.len() <= 512
        && snapshot.fields.iter().all(ConfigField::valid_for_agent)
}

#[cfg(windows)]
fn handle_incoming(app: &AppHandle, app_id: u32, instance_id: &str, frame: IncomingFrame) -> bool {
    let state = app.state::<AppState>();
    match frame {
        IncomingFrame::Snapshot {
            protocol_version,
            app_id: frame_app_id,
            instance_id: frame_instance,
            mods,
        } if protocol_version == PROTOCOL_VERSION
            && frame_app_id == app_id
            && frame_instance == instance_id
            && mods.len() <= 256
            && mods.iter().all(valid_snapshot) =>
        {
            let mut agent = state.agent.blocking_lock();
            if let Some(connection) = agent.connections.get_mut(&app_id).and_then(|connections| {
                connections
                    .iter_mut()
                    .find(|connection| connection.instance_id == instance_id)
            }) {
                connection.mods = mods;
            }
            drop(agent);
            let _ = app.emit(
                AGENT_STATE_EVENT,
                serde_json::json!({ "appId": app_id, "status": "connected" }),
            );
            true
        }
        IncomingFrame::ConfigChanged {
            protocol_version,
            app_id: frame_app_id,
            instance_id: frame_instance,
            mod_id,
            values,
        } if protocol_version == PROTOCOL_VERSION
            && frame_app_id == app_id
            && frame_instance == instance_id
            && values.len() <= 128 =>
        {
            let mut agent = state.agent.blocking_lock();
            if let Some(snapshot) = agent
                .connections
                .get_mut(&app_id)
                .and_then(|connections| {
                    connections
                        .iter_mut()
                        .find(|connection| connection.instance_id == instance_id)
                })
                .and_then(|connection| {
                    connection
                        .mods
                        .iter_mut()
                        .find(|snapshot| snapshot.mod_id == mod_id)
                })
            {
                for (id, value) in &values {
                    snapshot.values.insert(id.clone(), value.clone());
                }
            }
            drop(agent);
            let _ = app.emit(
                AGENT_CONFIG_EVENT,
                serde_json::json!({ "appId": app_id, "modId": mod_id, "values": values }),
            );
            true
        }
        IncomingFrame::ConfigResult {
            protocol_version,
            request_id,
            accepted,
            error_code,
            restart_required,
        } if protocol_version == PROTOCOL_VERSION => {
            if let Some(pending) = state.agent.blocking_lock().pending.remove(&request_id) {
                let _ = pending.send(if accepted {
                    Ok(restart_required)
                } else {
                    Err(error_code.unwrap_or_else(|| "rejected".to_owned()))
                });
            }
            true
        }
        IncomingFrame::Hello { .. } => false,
        _ => false,
    }
}

#[cfg(windows)]
fn resolve_agent_target(app_id: u32) -> Option<(InstallTarget, AgentMarker)> {
    let game = discover_installed_games(None)
        .ok()?
        .into_iter()
        .find(|game| game.app_id == app_id)?;
    let (_, target) = analyze_windows_installation(&game.install_directory).ok()?;
    let marker = read_agent_marker(&agent_directory(&target).join(".gametweaks-agent.json"))?;
    (marker.app_id == app_id).then_some((target, marker))
}

#[cfg(windows)]
mod windows_pipe {
    use std::mem::size_of;
    use std::os::windows::io::FromRawHandle;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Foundation::{
        GetLastError, LocalFree, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    use super::*;

    pub(super) fn run(app: AppHandle) {
        loop {
            match create_pipe() {
                Ok((pipe, client_process_id)) => {
                    let connection_app = app.clone();
                    let _ = std::thread::Builder::new()
                        .name("gametweaks-agent-client".to_owned())
                        .spawn(move || handle_client(connection_app, pipe, client_process_id));
                }
                Err(error) => {
                    tracing::warn!(%error, "the GameTweaks agent pipe could not accept a connection");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }

    fn create_pipe() -> std::io::Result<(File, u32)> {
        let name: Vec<u16> = PIPE_NAME.encode_utf16().chain(Some(0)).collect();
        let sddl: Vec<u16> = "D:P(A;;GA;;;SY)(A;;GA;;;OW)"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if converted == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                16,
                MAX_FRAME_BYTES as u32,
                MAX_FRAME_BYTES as u32,
                5_000,
                &attributes,
            )
        };
        unsafe { LocalFree(descriptor) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error());
        }
        let connected = unsafe { ConnectNamedPipe(handle, null_mut()) };
        if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            unsafe { CloseHandle(handle) };
            return Err(std::io::Error::last_os_error());
        }
        let mut client_process_id = 0_u32;
        if unsafe { GetNamedPipeClientProcessId(handle, &mut client_process_id) } == 0 {
            unsafe { CloseHandle(handle) };
            return Err(std::io::Error::last_os_error());
        }
        let file = unsafe { File::from_raw_handle(handle) };
        Ok((file, client_process_id))
    }

    fn handle_client(app: AppHandle, mut pipe: File, client_process_id: u32) {
        let challenge = match random_hex(32) {
            Ok(challenge) => challenge,
            Err(_) => return,
        };
        if write_frame(
            &mut pipe,
            &OutgoingFrame::Challenge {
                protocol_version: PROTOCOL_VERSION,
                challenge: challenge.clone(),
            },
        )
        .is_err()
        {
            return;
        }
        let Ok(IncomingFrame::Hello {
            protocol_version,
            app_id,
            process_id,
            instance_id,
            runtime,
            agent_version,
            proof: received_proof,
        }) = read_frame(&mut pipe)
        else {
            return;
        };
        let resolved = resolve_agent_target(app_id);
        if resolved.is_some() {
            update_status(&app, app_id, AgentConnectionStatus::Connecting);
        }
        let valid = protocol_version == PROTOCOL_VERSION
            && process_id == client_process_id
            && !instance_id.is_empty()
            && instance_id.len() <= 128
            && !agent_version.is_empty()
            && resolved.as_ref().is_some_and(|(target, marker)| {
                runtime == target.runtime
                    && agent_version == marker.version
                    && process_matches(process_id, &target.executable)
                    && proof(&marker.secret, &challenge, app_id, process_id, &instance_id)
                        .is_some_and(|expected| expected.eq_ignore_ascii_case(&received_proof))
            });
        if write_frame(
            &mut pipe,
            &OutgoingFrame::HelloAck {
                protocol_version: PROTOCOL_VERSION,
                accepted: valid,
                error_code: (!valid).then_some("authentication_failed"),
            },
        )
        .is_err()
            || !valid
        {
            if resolved.is_some() {
                update_status(&app, app_id, AgentConnectionStatus::Incompatible);
            }
            return;
        }

        let Ok(mut writer) = pipe.try_clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel::<OutboundMessage>();
        std::thread::spawn(move || {
            while let Ok(message) = receiver.recv() {
                if write_frame(&mut writer, &message.frame).is_err() {
                    break;
                }
            }
        });
        let status = {
            let state = app.state::<AppState>();
            state
                .game_mods
                .blocking_lock()
                .restart_required
                .retain(|(connected_app_id, _)| *connected_app_id != app_id);
            let mut agent = state.agent.blocking_lock();
            let connections = agent.connections.entry(app_id).or_default();
            connections.push(AgentConnection {
                instance_id: instance_id.clone(),
                sender,
                mods: Vec::new(),
            });
            let status = if connections.len() == 1 {
                AgentConnectionStatus::Connected
            } else {
                AgentConnectionStatus::Ambiguous
            };
            agent.statuses.insert(app_id, status);
            status
        };
        let _ = app.emit(
            AGENT_STATE_EVENT,
            serde_json::json!({ "appId": app_id, "status": status }),
        );
        while let Ok(frame) = read_frame(&mut pipe) {
            if !handle_incoming(&app, app_id, &instance_id, frame) {
                break;
            }
        }
        let state = app.state::<AppState>();
        let mut agent = state.agent.blocking_lock();
        let status = if let Some(connections) = agent.connections.get_mut(&app_id) {
            connections.retain(|connection| connection.instance_id != instance_id);
            if connections.is_empty() {
                agent.connections.remove(&app_id);
                AgentConnectionStatus::Disconnected
            } else if connections.len() == 1 {
                AgentConnectionStatus::Connected
            } else {
                AgentConnectionStatus::Ambiguous
            }
        } else {
            AgentConnectionStatus::Disconnected
        };
        agent.statuses.insert(app_id, status);
        drop(agent);
        let _ = app.emit(
            AGENT_STATE_EVENT,
            serde_json::json!({ "appId": app_id, "status": status }),
        );
    }

    fn update_status(app: &AppHandle, app_id: u32, status: AgentConnectionStatus) {
        app.state::<AppState>()
            .agent
            .blocking_lock()
            .statuses
            .insert(app_id, status);
        let _ = app.emit(
            AGENT_STATE_EVENT,
            serde_json::json!({ "appId": app_id, "status": status }),
        );
    }

    fn process_matches(process_id: u32, executable: &Path) -> bool {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if process.is_null() {
            return false;
        }
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        let queried = unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &mut length,
            )
        };
        unsafe { CloseHandle(process) };
        if queried == 0 {
            return false;
        }
        let process_path = fs::canonicalize(PathBuf::from(String::from_utf16_lossy(
            &buffer[..length as usize],
        )))
        .ok();
        let executable = fs::canonicalize(executable).ok();
        process_path == executable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_installed_and_current_agents() {
        let root = tempfile::tempdir().unwrap();
        let target = InstallTarget {
            game_root: root.path().to_path_buf(),
            executable: root.path().join("game.exe"),
            runtime: BepInExRuntime::Mono,
            architecture: crate::bepinex::BepInExArchitecture::X64,
        };
        let directory = agent_directory(&target);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("agent.dll"), b"agent").unwrap();
        write_marker(
            &directory.join(".gametweaks-agent.json"),
            &AgentMarker {
                schema_version: 1,
                app_id: 2709570,
                version: "0.0.9".to_owned(),
                runtime: BepInExRuntime::Mono,
                secret: "11".repeat(32),
                files: vec!["agent.dll".to_owned()],
            },
        )
        .unwrap();

        assert!(agent_is_installed(&target, 2709570));
        assert!(!agent_is_current(&target, 2709570));
        assert!(!agent_is_installed(&target, 10));
    }

    #[test]
    fn challenge_proof_is_stable_and_context_bound() {
        let secret = "11".repeat(32);
        let first = proof(&secret, "challenge", 10, 20, "instance").unwrap();
        assert_eq!(
            first,
            proof(&secret, "challenge", 10, 20, "instance").unwrap()
        );
        assert_ne!(
            first,
            proof(&secret, "challenge", 11, 20, "instance").unwrap()
        );
    }

    #[test]
    fn frame_reader_rejects_oversized_messages() {
        let mut raw = ((MAX_FRAME_BYTES as u32) + 1).to_le_bytes().to_vec();
        raw.extend_from_slice(b"{}");
        assert!(read_frame(&mut raw.as_slice()).is_err());
    }

    #[test]
    fn protocol_frames_round_trip() {
        let frame = OutgoingFrame::SetConfig {
            protocol_version: 1,
            request_id: "request".to_owned(),
            mod_id: "example.mod".to_owned(),
            values: HashMap::from([("enabled".to_owned(), Value::Bool(true))]),
        };
        let mut raw = Vec::new();
        write_frame(&mut raw, &frame).unwrap();
        assert!(raw.len() > 4);

        let payload: Value = serde_json::from_slice(&raw[4..]).unwrap();
        assert_eq!(payload["type"], "setConfig");
        assert_eq!(payload["protocolVersion"], 1);
        assert_eq!(payload["requestId"], "request");
        assert_eq!(payload["modId"], "example.mod");
        assert!(payload.get("protocol_version").is_none());
        assert!(payload.get("request_id").is_none());
        assert!(payload.get("mod_id").is_none());
    }

    #[test]
    fn rejects_invalid_dynamic_mod_snapshots() {
        let snapshot = AgentModSnapshot {
            mod_id: "../unsafe".to_owned(),
            version: "1.0.0".to_owned(),
            name: LocalizedText {
                en: "Unsafe".to_owned(),
                de: None,
            },
            description: LocalizedText {
                en: "Unsafe".to_owned(),
                de: None,
            },
            fields: Vec::new(),
            values: HashMap::new(),
        };
        assert!(!valid_snapshot(&snapshot));
    }

    #[test]
    fn agent_only_mods_are_reported_as_external() {
        let snapshot = AgentModSnapshot {
            mod_id: "external.mod".to_owned(),
            version: "1.2.3".to_owned(),
            name: LocalizedText {
                en: "External".to_owned(),
                de: None,
            },
            description: LocalizedText {
                en: "External mod".to_owned(),
                de: None,
            },
            fields: Vec::new(),
            values: HashMap::new(),
        };

        let game_mod = snapshot_to_external_mod(&snapshot);

        assert!(game_mod.external);
        assert!(!game_mod.official);
        assert_eq!(game_mod.status, GameModStatus::External);
    }

    #[test]
    fn contradictory_dynamic_fields_lock_the_catalog_field() {
        let field = |default| ConfigField::Boolean {
            id: "enabled".to_owned(),
            section: "General".to_owned(),
            key: "Enabled".to_owned(),
            label: LocalizedText {
                en: "Enabled".to_owned(),
                de: None,
            },
            description: None,
            locked: false,
            default,
            apply_mode: crate::game_mods::ConfigApplyMode::Live,
            display: crate::game_mods::BooleanDisplay::Switch,
        };
        let mut catalog = vec![field(false)];
        merge_runtime_fields(&mut catalog, &[field(true)]);
        assert!(catalog[0].is_locked());
    }
}
