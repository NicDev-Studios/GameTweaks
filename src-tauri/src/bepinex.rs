use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;
use reqwest::{Client, Response, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tempfile::{Builder as TempBuilder, NamedTempFile};
use tokio::io::AsyncWriteExt;
use zip::ZipArchive;

use crate::core::error::{AppError, AppResult, ErrorResponse};
use crate::core::state::AppState;
use crate::steam::{discover_installed_games, SteamGame};

const GITHUB_RELEASES_URL: &str =
    "https://api.github.com/repos/BepInEx/BepInEx/releases?per_page=50";
const BEPINEX_BUILDS_URL: &str = "https://builds.bepinex.dev/projects/bepinex_be";
const INSTALL_PROGRESS_EVENT: &str = "gametweaks-bepinex-install-progress";
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);
const METADATA_MAX_BYTES: usize = 8 * 1024 * 1024;
const ARCHIVE_MAX_BYTES: u64 = 256 * 1024 * 1024;
const ARCHIVE_MAX_UNCOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;
const ARCHIVE_MAX_ENTRIES: usize = 10_000;
const INSTALL_SCAN_MAX_ENTRIES: usize = 10_000;
const INSTALL_SCAN_MAX_DEPTH: usize = 2;
const MARKER_MAX_BYTES: usize = 256 * 1024;
const LOG_MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BepInExRuntime {
    Mono,
    Il2Cpp,
}

impl BepInExRuntime {
    fn label(self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Il2Cpp => "il2cpp",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BepInExArchitecture {
    X86,
    X64,
}

impl BepInExArchitecture {
    fn asset_label(self) -> &'static str {
        match self {
            Self::X86 => "x86",
            Self::X64 => "x64",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BepInExAvailability {
    Installable,
    Installed,
    Unsupported,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BepInExReason {
    WindowsOnly,
    NotUnity,
    AmbiguousExecutable,
    AmbiguousRuntime,
    UnsupportedArchitecture,
    InspectionFailed,
    UnsafeSymlink,
    AntiCheatDetected,
    ExistingFiles,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BepInExGameStatus {
    pub status: BepInExAvailability,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<BepInExRuntime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<BepInExArchitecture>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<BepInExReason>,
    pub managed_by_game_tweaks: bool,
}

impl BepInExGameStatus {
    fn unsupported(reason: BepInExReason) -> Self {
        Self {
            status: BepInExAvailability::Unsupported,
            runtime: None,
            architecture: None,
            installed_version: None,
            reason: Some(reason),
            managed_by_game_tweaks: false,
        }
    }

    fn blocked(
        reason: BepInExReason,
        runtime: Option<BepInExRuntime>,
        architecture: Option<BepInExArchitecture>,
    ) -> Self {
        Self {
            status: BepInExAvailability::Blocked,
            runtime,
            architecture,
            installed_version: None,
            reason: Some(reason),
            managed_by_game_tweaks: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InstallTarget {
    pub(crate) game_root: PathBuf,
    pub(crate) executable: PathBuf,
    pub(crate) runtime: BepInExRuntime,
    pub(crate) architecture: BepInExArchitecture,
}

#[derive(Clone, Debug)]
struct PackageInfo {
    version: String,
    asset_name: String,
    download_url: Url,
    expected_digest: Option<String>,
    source: PackageSource,
}

#[derive(Clone, Copy, Debug)]
enum PackageSource {
    GitHubStable,
    BepInBuilds,
}

impl PackageSource {
    fn label(self) -> &'static str {
        match self {
            Self::GitHubStable => "github-stable",
            Self::BepInBuilds => "bepinbuilds",
        }
    }
}

#[derive(Clone, Debug)]
struct PreparedInstall {
    app_id: u32,
    steam_install_directory: PathBuf,
    target: InstallTarget,
    executable_size: u64,
    executable_modified: Option<std::time::SystemTime>,
    package: PackageInfo,
    expires_at: Instant,
}

#[derive(Default)]
pub struct BepInExInstallState {
    pending: HashMap<String, PreparedInstall>,
    installing: HashSet<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BepInExInstallPlan {
    pub plan_id: String,
    pub app_id: u32,
    pub version: String,
    pub runtime: BepInExRuntime,
    pub architecture: BepInExArchitecture,
    pub release_channel: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BepInExInstallResult {
    pub app_id: u32,
    pub version: String,
    pub runtime: BepInExRuntime,
    pub architecture: BepInExArchitecture,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallProgress {
    app_id: u32,
    stage: InstallStage,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percentage: Option<u64>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum InstallStage {
    Downloading,
    Verifying,
    Installing,
    Completed,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    draft: bool,
    prerelease: bool,
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstallMarker {
    pub(crate) schema_version: u32,
    pub(crate) app_id: u32,
    pub(crate) version: String,
    pub(crate) runtime: String,
    pub(crate) architecture: String,
    pub(crate) source: String,
    pub(crate) asset_name: String,
    pub(crate) sha256: String,
    pub(crate) files: Vec<String>,
}

pub fn analyze_installation(install_directory: &Path) -> BepInExGameStatus {
    #[cfg(windows)]
    {
        match analyze_windows_installation(install_directory) {
            Ok((status, _)) => status,
            Err(status) => status,
        }
    }

    #[cfg(not(windows))]
    {
        let _ = install_directory;
        BepInExGameStatus::unsupported(BepInExReason::WindowsOnly)
    }
}

pub(crate) fn analyze_windows_installation(
    install_directory: &Path,
) -> Result<(BepInExGameStatus, InstallTarget), BepInExGameStatus> {
    let install_directory = fs::canonicalize(install_directory)
        .map_err(|_| BepInExGameStatus::blocked(BepInExReason::InspectionFailed, None, None))?;
    if fs::symlink_metadata(&install_directory)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(true)
    {
        return Err(BepInExGameStatus::blocked(
            BepInExReason::UnsafeSymlink,
            None,
            None,
        ));
    }

    let candidates = find_unity_candidates(&install_directory)?;
    if candidates.is_empty() {
        return Err(BepInExGameStatus::unsupported(BepInExReason::NotUnity));
    }
    if candidates.len() != 1 {
        return Err(BepInExGameStatus::blocked(
            BepInExReason::AmbiguousExecutable,
            None,
            None,
        ));
    }

    let (game_root, executable, runtime) = candidates.into_iter().next().expect("one candidate");
    let architecture = read_pe_architecture(&executable)
        .map_err(|reason| BepInExGameStatus::blocked(reason, Some(runtime), None))?;
    let target = InstallTarget {
        game_root,
        executable,
        runtime,
        architecture,
    };

    if let Some(installed_version) = installed_bepinex_version(&target.game_root)? {
        let managed_by_game_tweaks =
            valid_install_marker(&target.game_root.join("BepInEx/.gametweaks-install.json"))
                .is_some();
        return Ok((
            BepInExGameStatus {
                status: BepInExAvailability::Installed,
                runtime: Some(runtime),
                architecture: Some(architecture),
                installed_version,
                reason: None,
                managed_by_game_tweaks,
            },
            target,
        ));
    }

    if contains_anti_cheat(&target.game_root)? {
        return Err(BepInExGameStatus::blocked(
            BepInExReason::AntiCheatDetected,
            Some(runtime),
            Some(architecture),
        ));
    }

    Ok((
        BepInExGameStatus {
            status: BepInExAvailability::Installable,
            runtime: Some(runtime),
            architecture: Some(architecture),
            installed_version: None,
            reason: None,
            managed_by_game_tweaks: false,
        },
        target,
    ))
}

type UnityCandidate = (PathBuf, PathBuf, BepInExRuntime);

fn find_unity_candidates(
    install_directory: &Path,
) -> Result<Vec<UnityCandidate>, BepInExGameStatus> {
    let mut directories = vec![(install_directory.to_path_buf(), 0_usize)];
    let mut data_directories = Vec::new();
    let mut inspected_entries = 0_usize;

    while let Some((directory, depth)) = directories.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|_| BepInExGameStatus::blocked(BepInExReason::InspectionFailed, None, None))?;
        for entry in entries {
            let entry = entry.map_err(|_| {
                BepInExGameStatus::blocked(BepInExReason::InspectionFailed, None, None)
            })?;
            inspected_entries += 1;
            if inspected_entries > INSTALL_SCAN_MAX_ENTRIES {
                return Err(BepInExGameStatus::blocked(
                    BepInExReason::InspectionFailed,
                    None,
                    None,
                ));
            }
            let file_type = entry.file_type().map_err(|_| {
                BepInExGameStatus::blocked(BepInExReason::InspectionFailed, None, None)
            })?;
            if file_type.is_symlink() {
                return Err(BepInExGameStatus::blocked(
                    BepInExReason::UnsafeSymlink,
                    None,
                    None,
                ));
            }
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if strip_suffix_ignore_ascii_case(&name, "_Data").is_some() {
                data_directories.push(entry.path());
            } else if depth < INSTALL_SCAN_MAX_DEPTH {
                directories.push((entry.path(), depth + 1));
            }
        }
    }

    let mut candidates = Vec::new();
    let mut saw_runtime_conflict = false;
    for data_directory in data_directories {
        let Some(parent) = data_directory.parent() else {
            continue;
        };
        let Some(data_name) = data_directory.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(base_name) = strip_suffix_ignore_ascii_case(data_name, "_Data") else {
            continue;
        };
        let executable_name = format!("{base_name}.exe");
        let Some(executable) = find_child_ignore_ascii_case(parent, &executable_name)? else {
            continue;
        };
        if !fs::metadata(&executable)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            continue;
        }

        let has_managed = find_child_ignore_ascii_case(&data_directory, "Managed")?
            .and_then(|path| fs::metadata(path).ok())
            .is_some_and(|metadata| metadata.is_dir());
        let has_game_assembly = find_child_ignore_ascii_case(parent, "GameAssembly.dll")?
            .and_then(|path| fs::metadata(path).ok())
            .is_some_and(|metadata| metadata.is_file());
        let has_metadata = find_case_insensitive_path(
            &data_directory,
            &["il2cpp_data", "Metadata", "global-metadata.dat"],
        )?
        .and_then(|path| fs::metadata(path).ok())
        .is_some_and(|metadata| metadata.is_file());

        let runtime = match (has_managed, has_game_assembly && has_metadata) {
            (true, false) => Some(BepInExRuntime::Mono),
            (false, true) => Some(BepInExRuntime::Il2Cpp),
            (true, true) => {
                saw_runtime_conflict = true;
                None
            }
            (false, false) => None,
        };
        if let Some(runtime) = runtime {
            candidates.push((parent.to_path_buf(), executable, runtime));
        }
    }

    if saw_runtime_conflict {
        return Err(BepInExGameStatus::blocked(
            BepInExReason::AmbiguousRuntime,
            None,
            None,
        ));
    }
    Ok(candidates)
}

fn strip_suffix_ignore_ascii_case<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    let start = value.len().checked_sub(suffix.len())?;
    value[start..]
        .eq_ignore_ascii_case(suffix)
        .then_some(&value[..start])
}

fn find_child_ignore_ascii_case(
    directory: &Path,
    name: &str,
) -> Result<Option<PathBuf>, BepInExGameStatus> {
    for entry in fs::read_dir(directory)
        .map_err(|_| BepInExGameStatus::blocked(BepInExReason::InspectionFailed, None, None))?
    {
        let entry = entry
            .map_err(|_| BepInExGameStatus::blocked(BepInExReason::InspectionFailed, None, None))?;
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            if entry
                .file_type()
                .map(|kind| kind.is_symlink())
                .unwrap_or(true)
            {
                return Err(BepInExGameStatus::blocked(
                    BepInExReason::UnsafeSymlink,
                    None,
                    None,
                ));
            }
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

fn find_case_insensitive_path(
    root: &Path,
    components: &[&str],
) -> Result<Option<PathBuf>, BepInExGameStatus> {
    let mut current = root.to_path_buf();
    for component in components {
        let Some(next) = find_child_ignore_ascii_case(&current, component)? else {
            return Ok(None);
        };
        current = next;
    }
    Ok(Some(current))
}

fn read_pe_architecture(executable: &Path) -> Result<BepInExArchitecture, BepInExReason> {
    let mut file = File::open(executable).map_err(|_| BepInExReason::InspectionFailed)?;
    let mut dos_header = [0_u8; 64];
    file.read_exact(&mut dos_header)
        .map_err(|_| BepInExReason::UnsupportedArchitecture)?;
    if &dos_header[..2] != b"MZ" {
        return Err(BepInExReason::UnsupportedArchitecture);
    }
    let pe_offset = u32::from_le_bytes(
        dos_header[0x3c..0x40]
            .try_into()
            .map_err(|_| BepInExReason::UnsupportedArchitecture)?,
    ) as u64;
    if pe_offset > 16 * 1024 * 1024 {
        return Err(BepInExReason::UnsupportedArchitecture);
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(pe_offset))
        .map_err(|_| BepInExReason::UnsupportedArchitecture)?;
    let mut pe_header = [0_u8; 6];
    file.read_exact(&mut pe_header)
        .map_err(|_| BepInExReason::UnsupportedArchitecture)?;
    if &pe_header[..4] != b"PE\0\0" {
        return Err(BepInExReason::UnsupportedArchitecture);
    }
    match u16::from_le_bytes([pe_header[4], pe_header[5]]) {
        0x014c => Ok(BepInExArchitecture::X86),
        0x8664 => Ok(BepInExArchitecture::X64),
        _ => Err(BepInExReason::UnsupportedArchitecture),
    }
}

fn installed_bepinex_version(
    game_root: &Path,
) -> Result<Option<Option<String>>, BepInExGameStatus> {
    let bepinex = game_root.join("BepInEx");
    let doorstop = game_root.join("doorstop_config.ini");
    let winhttp = game_root.join("winhttp.dll");
    let v5_core = bepinex.join("core").join("BepInEx.dll");
    let v6_core = bepinex.join("core").join("BepInEx.Core.dll");
    let has_core = v5_core.is_file() || v6_core.is_file();
    let complete = bepinex.is_dir() && doorstop.is_file() && winhttp.is_file() && has_core;
    let any = bepinex.exists() || doorstop.exists() || winhttp.exists();

    if !any {
        return Ok(None);
    }
    if !complete {
        return Err(BepInExGameStatus::blocked(
            BepInExReason::ExistingFiles,
            None,
            None,
        ));
    }

    let version = read_marker_version(&bepinex.join(".gametweaks-install.json"))
        .or_else(|| read_log_version(&bepinex.join("LogOutput.log")))
        .or_else(|| read_log_version(&bepinex.join("LogOutput.txt")));
    Ok(Some(version))
}

fn read_marker_version(path: &Path) -> Option<String> {
    let marker = valid_install_marker(path)?;
    (!marker.version.trim().is_empty()).then_some(marker.version)
}

pub(crate) fn valid_install_marker(path: &Path) -> Option<InstallMarker> {
    let raw = read_file_limited(path, MARKER_MAX_BYTES)?;
    let marker: InstallMarker = serde_json::from_slice(&raw).ok()?;
    if marker.schema_version != 1
        || marker.version.trim().is_empty()
        || marker.files.is_empty()
        || marker.files.len() > ARCHIVE_MAX_ENTRIES
        || marker.files.iter().any(|file| {
            let path = Path::new(file);
            path.is_absolute()
                || path
                    .components()
                    .any(|component| !matches!(component, Component::Normal(_)))
        })
    {
        return None;
    }
    Some(marker)
}

fn read_log_version(path: &Path) -> Option<String> {
    let raw = read_file_limited(path, LOG_MAX_BYTES)?;
    let text = String::from_utf8_lossy(&raw);
    for line in text.lines().take(100) {
        let Some((_, after)) = line.split_once("BepInEx ") else {
            continue;
        };
        let Some(version) = after.split_whitespace().next() else {
            continue;
        };
        let version = version.trim_matches(|character: char| {
            !character.is_ascii_alphanumeric()
                && character != '.'
                && character != '-'
                && character != '+'
        });
        if version.chars().any(|character| character.is_ascii_digit()) {
            return Some(version.to_owned());
        }
    }
    None
}

fn read_file_limited(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let mut raw = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut raw).ok()?;
    Some(raw)
}

fn contains_anti_cheat(game_root: &Path) -> Result<bool, BepInExGameStatus> {
    let mut directories = vec![(game_root.to_path_buf(), 0_usize)];
    let mut inspected = 0_usize;
    while let Some((directory, depth)) = directories.pop() {
        for entry in fs::read_dir(directory)
            .map_err(|_| BepInExGameStatus::blocked(BepInExReason::InspectionFailed, None, None))?
        {
            let entry = entry.map_err(|_| {
                BepInExGameStatus::blocked(BepInExReason::InspectionFailed, None, None)
            })?;
            inspected += 1;
            if inspected > INSTALL_SCAN_MAX_ENTRIES {
                return Err(BepInExGameStatus::blocked(
                    BepInExReason::InspectionFailed,
                    None,
                    None,
                ));
            }
            let file_type = entry.file_type().map_err(|_| {
                BepInExGameStatus::blocked(BepInExReason::InspectionFailed, None, None)
            })?;
            if file_type.is_symlink() {
                return Err(BepInExGameStatus::blocked(
                    BepInExReason::UnsafeSymlink,
                    None,
                    None,
                ));
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if is_anti_cheat_name(&name) {
                return Ok(true);
            }
            if file_type.is_dir() && depth < INSTALL_SCAN_MAX_DEPTH {
                directories.push((entry.path(), depth + 1));
            }
        }
    }
    Ok(false)
}

fn is_anti_cheat_name(name: &str) -> bool {
    name == "easyanticheat"
        || name == "easyanticheat_eos"
        || name == "battleye"
        || name == "start_protected_game.exe"
        || (name.starts_with("easyanticheat") && (name.ends_with(".exe") || name.ends_with(".dll")))
        || (name.starts_with("beservice") && name.ends_with(".exe"))
        || (name.starts_with("beclient") && name.ends_with(".dll"))
}

pub async fn prepare_install(
    _app: &AppHandle,
    state: &AppState,
    app_id: u32,
) -> AppResult<BepInExInstallPlan> {
    if !cfg!(windows) {
        return Err(bepinex_error(
            "bepinex_unsupported",
            "automatic BepInEx installation is only available on Windows",
        ));
    }

    let game = resolve_game(app_id).await?;
    let (status, target) =
        analyze_windows_installation(&game.install_directory).map_err(status_error)?;
    if status.status != BepInExAvailability::Installable {
        return Err(status_error(status));
    }
    ensure_game_stopped(&target.executable)?;
    TempBuilder::new()
        .prefix(".gametweaks-write-check-")
        .tempdir_in(&target.game_root)
        .map_err(|_| {
            bepinex_error(
                "bepinex_install_error",
                "the game directory is not writable",
            )
        })?;

    let package = resolve_package(target.runtime, target.architecture).await?;
    let metadata = fs::metadata(&target.executable).map_err(|_| {
        bepinex_error(
            "bepinex_blocked",
            "the game executable could not be verified",
        )
    })?;
    let plan_id = random_plan_id()?;
    let prepared = PreparedInstall {
        app_id,
        steam_install_directory: game.install_directory,
        target: target.clone(),
        executable_size: metadata.len(),
        executable_modified: metadata.modified().ok(),
        package: package.clone(),
        expires_at: Instant::now() + PLAN_LIFETIME,
    };

    let mut install_state = state.bepinex.lock().await;
    install_state
        .pending
        .retain(|_, plan| plan.expires_at > Instant::now());
    if install_state.installing.contains(&app_id) {
        return Err(bepinex_error(
            "bepinex_busy",
            "BepInEx is already being installed for this game",
        ));
    }
    install_state
        .pending
        .retain(|_, plan| plan.app_id != app_id);
    install_state.pending.insert(plan_id.clone(), prepared);

    Ok(BepInExInstallPlan {
        plan_id,
        app_id,
        version: package.version,
        runtime: target.runtime,
        architecture: target.architecture,
        release_channel: match target.runtime {
            BepInExRuntime::Mono => "stable",
            BepInExRuntime::Il2Cpp => "bleedingEdge",
        },
    })
}

pub async fn install(
    app: &AppHandle,
    state: &AppState,
    plan_id: String,
) -> AppResult<BepInExInstallResult> {
    let prepared = {
        let mut install_state = state.bepinex.lock().await;
        let prepared = install_state.pending.remove(&plan_id).ok_or_else(|| {
            bepinex_error(
                "bepinex_plan_expired",
                "the BepInEx installation plan is missing or expired",
            )
        })?;
        if prepared.expires_at <= Instant::now() {
            return Err(bepinex_error(
                "bepinex_plan_expired",
                "the BepInEx installation plan has expired",
            ));
        }
        if !install_state.installing.insert(prepared.app_id) {
            return Err(bepinex_error(
                "bepinex_busy",
                "BepInEx is already being installed for this game",
            ));
        }
        prepared
    };

    let result = install_prepared(app, &prepared).await;
    state
        .bepinex
        .lock()
        .await
        .installing
        .remove(&prepared.app_id);
    result
}

async fn install_prepared(
    app: &AppHandle,
    prepared: &PreparedInstall,
) -> AppResult<BepInExInstallResult> {
    let game = resolve_game(prepared.app_id).await?;
    let current_steam_directory = fs::canonicalize(&game.install_directory).map_err(|_| {
        bepinex_error(
            "bepinex_blocked",
            "the Steam installation changed after confirmation",
        )
    })?;
    let prepared_steam_directory =
        fs::canonicalize(&prepared.steam_install_directory).map_err(|_| {
            bepinex_error(
                "bepinex_blocked",
                "the Steam installation changed after confirmation",
            )
        })?;
    if current_steam_directory != prepared_steam_directory {
        return Err(bepinex_error(
            "bepinex_blocked",
            "the Steam installation changed after confirmation",
        ));
    }

    let (status, current_target) =
        analyze_windows_installation(&game.install_directory).map_err(status_error)?;
    if status.status != BepInExAvailability::Installable
        || current_target.game_root != prepared.target.game_root
        || current_target.executable != prepared.target.executable
        || current_target.runtime != prepared.target.runtime
        || current_target.architecture != prepared.target.architecture
    {
        return Err(bepinex_error(
            "bepinex_blocked",
            "the game installation changed after confirmation",
        ));
    }
    let executable_metadata = fs::metadata(&current_target.executable).map_err(|_| {
        bepinex_error(
            "bepinex_blocked",
            "the game executable changed after confirmation",
        )
    })?;
    if executable_metadata.len() != prepared.executable_size
        || executable_metadata.modified().ok() != prepared.executable_modified
    {
        return Err(bepinex_error(
            "bepinex_blocked",
            "the game executable changed after confirmation",
        ));
    }
    ensure_game_stopped(&current_target.executable)?;

    let (archive_path, archive_guard, sha256) = download_package(
        app,
        prepared.app_id,
        &current_target.game_root,
        &prepared.package,
    )
    .await?;
    emit_progress(app, prepared.app_id, InstallStage::Verifying, 0, None);
    ensure_game_stopped(&current_target.executable)?;
    let target = current_target.clone();
    let package = prepared.package.clone();
    emit_progress(app, prepared.app_id, InstallStage::Installing, 0, None);
    let app_id = prepared.app_id;
    tauri::async_runtime::spawn_blocking(move || {
        install_archive(&archive_path, &target, &package, &sha256, app_id)
    })
    .await
    .map_err(|_| {
        bepinex_error(
            "bepinex_install_error",
            "the BepInEx installation task failed",
        )
    })??;
    drop(archive_guard);

    emit_progress(app, prepared.app_id, InstallStage::Completed, 0, None);
    Ok(BepInExInstallResult {
        app_id: prepared.app_id,
        version: prepared.package.version.clone(),
        runtime: prepared.target.runtime,
        architecture: prepared.target.architecture,
    })
}

pub(crate) async fn resolve_game(app_id: u32) -> AppResult<SteamGame> {
    let games = tauri::async_runtime::spawn_blocking(|| discover_installed_games(None))
        .await
        .map_err(|_| bepinex_error("bepinex_blocked", "Steam games could not be resolved"))?
        .map_err(|_| bepinex_error("bepinex_blocked", "Steam games could not be resolved"))?;
    games
        .into_iter()
        .find(|game| game.app_id == app_id)
        .ok_or_else(|| {
            bepinex_error(
                "bepinex_blocked",
                "the selected Steam game is no longer installed",
            )
        })
}

async fn resolve_package(
    runtime: BepInExRuntime,
    architecture: BepInExArchitecture,
) -> AppResult<PackageInfo> {
    let client = bepinex_client()?;
    match runtime {
        BepInExRuntime::Mono => resolve_mono_package(&client, architecture).await,
        BepInExRuntime::Il2Cpp => resolve_il2cpp_package(&client, architecture).await,
    }
}

async fn resolve_mono_package(
    client: &Client,
    architecture: BepInExArchitecture,
) -> AppResult<PackageInfo> {
    let releases = client
        .get(GITHUB_RELEASES_URL)
        .send()
        .await
        .map_err(network_error)?
        .error_for_status()
        .map_err(network_error)?
        .json::<Vec<GitHubRelease>>()
        .await
        .map_err(|_| {
            bepinex_error(
                "bepinex_network_error",
                "the BepInEx release response was invalid",
            )
        })?;

    select_mono_package(releases, architecture)
}

fn select_mono_package(
    releases: Vec<GitHubRelease>,
    architecture: BepInExArchitecture,
) -> AppResult<PackageInfo> {
    let release = releases
        .into_iter()
        .filter(|release| {
            !release.draft && !release.prerelease && release.tag_name.starts_with("v5.4.")
        })
        .max_by_key(|release| parse_v5_version(&release.tag_name))
        .ok_or_else(|| {
            bepinex_error(
                "bepinex_network_error",
                "no stable BepInEx 5 release was found",
            )
        })?;
    let version = release
        .tag_name
        .strip_prefix('v')
        .filter(|_| parse_v5_version(&release.tag_name).is_some())
        .ok_or_else(|| {
            bepinex_error(
                "bepinex_integrity_error",
                "the BepInEx release version was invalid",
            )
        })?
        .to_owned();
    let expected_name = format!("BepInEx_win_{}_{}.zip", architecture.asset_label(), version);
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == expected_name)
        .ok_or_else(|| {
            bepinex_error(
                "bepinex_integrity_error",
                "the stable BepInEx release has no matching Windows package",
            )
        })?;
    let digest = asset
        .digest
        .and_then(|digest| digest.strip_prefix("sha256:").map(str::to_owned))
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| {
            bepinex_error(
                "bepinex_integrity_error",
                "the stable BepInEx package has no valid SHA-256 digest",
            )
        })?;
    let download_url = Url::parse(&asset.browser_download_url).map_err(|_| {
        bepinex_error(
            "bepinex_integrity_error",
            "the stable BepInEx download URL was invalid",
        )
    })?;
    if download_url.scheme() != "https" || download_url.host_str() != Some("github.com") {
        return Err(bepinex_error(
            "bepinex_integrity_error",
            "the stable BepInEx download host was not trusted",
        ));
    }

    Ok(PackageInfo {
        version,
        asset_name: expected_name,
        download_url,
        expected_digest: Some(digest.to_ascii_lowercase()),
        source: PackageSource::GitHubStable,
    })
}

fn parse_v5_version(tag: &str) -> Option<(u64, u64, u64, u64)> {
    let mut parts = tag.strip_prefix('v')?.split('.');
    let version = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    (parts.next().is_none() && version.0 == 5 && version.1 == 4).then_some(version)
}

async fn resolve_il2cpp_package(
    client: &Client,
    architecture: BepInExArchitecture,
) -> AppResult<PackageInfo> {
    let response = client
        .get(BEPINEX_BUILDS_URL)
        .send()
        .await
        .map_err(network_error)?
        .error_for_status()
        .map_err(network_error)?;
    let html = String::from_utf8(read_response_limited(response, METADATA_MAX_BYTES).await?)
        .map_err(|_| {
            bepinex_error(
                "bepinex_network_error",
                "the BepInBuilds response was invalid",
            )
        })?;

    select_il2cpp_package(&html, architecture)
}

fn select_il2cpp_package(html: &str, architecture: BepInExArchitecture) -> AppResult<PackageInfo> {
    let prefix = format!("BepInEx-Unity.IL2CPP-win-{}-", architecture.asset_label());

    for href in href_values(html) {
        let href = decode_minimal_html_entities(href);
        let Ok(url) = Url::parse(BEPINEX_BUILDS_URL).and_then(|base| base.join(&href)) else {
            continue;
        };
        if url.scheme() != "https" || url.host_str() != Some("builds.bepinex.dev") {
            continue;
        }
        let segments: Vec<_> = url.path_segments().into_iter().flatten().collect();
        if segments.len() != 4
            || segments[0] != "projects"
            || segments[1] != "bepinex_be"
            || !segments[2].bytes().all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let asset_name = segments[3].replace("%2B", "+").replace("%2b", "+");
        if !asset_name.starts_with(&prefix)
            || !asset_name.ends_with(".zip")
            || !valid_bleeding_edge_name(&asset_name, segments[2])
        {
            continue;
        }
        let version = asset_name
            .strip_prefix(&prefix)
            .and_then(|name| name.strip_suffix(".zip"))
            .ok_or_else(|| {
                bepinex_error(
                    "bepinex_integrity_error",
                    "the BepInBuilds package name was invalid",
                )
            })?
            .to_owned();
        return Ok(PackageInfo {
            version,
            asset_name,
            download_url: url,
            expected_digest: None,
            source: PackageSource::BepInBuilds,
        });
    }

    Err(bepinex_error(
        "bepinex_network_error",
        "no matching BepInEx IL2CPP package was found",
    ))
}

fn href_values(html: &str) -> impl Iterator<Item = &str> {
    html.split("href=\"")
        .skip(1)
        .filter_map(|tail| tail.split_once('"').map(|(href, _)| href))
}

fn decode_minimal_html_entities(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#43;", "+")
        .replace("&#x2B;", "+")
        .replace("&#x2b;", "+")
}

fn valid_bleeding_edge_name(asset_name: &str, path_build: &str) -> bool {
    let Some((version, suffix)) = asset_name.rsplit_once("-be.") else {
        return false;
    };
    if !version.rsplit('-').next().is_some_and(|version| {
        let parts: Vec<_> = version.split('.').collect();
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    }) {
        return false;
    }
    let Some(suffix) = suffix.strip_suffix(".zip") else {
        return false;
    };
    let Some((build, commit)) = suffix.split_once('+') else {
        return false;
    };
    build == path_build && commit.len() >= 7 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn read_response_limited(mut response: Response, limit: usize) -> AppResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(bepinex_error(
            "bepinex_network_error",
            "the BepInEx metadata response was too large",
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(bepinex_error(
                "bepinex_network_error",
                "the BepInEx metadata response was too large",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn bepinex_client() -> AppResult<Client> {
    Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(5 * 60))
        .redirect(Policy::custom(|attempt| {
            if trusted_response_url(attempt.url()) && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .user_agent("GameTweaks BepInEx installer")
        .build()
        .map_err(|_| {
            bepinex_error(
                "bepinex_network_error",
                "the BepInEx network client could not be created",
            )
        })
}

fn trusted_response_url(url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "objects.githubusercontent.com"
                    | "release-assets.githubusercontent.com"
                    | "builds.bepinex.dev"
            )
        )
}

async fn download_package(
    app: &AppHandle,
    app_id: u32,
    game_root: &Path,
    package: &PackageInfo,
) -> AppResult<(PathBuf, tempfile::TempPath, String)> {
    let client = bepinex_client()?;
    let mut response = client
        .get(package.download_url.clone())
        .send()
        .await
        .map_err(network_error)?
        .error_for_status()
        .map_err(network_error)?;
    if !trusted_response_url(response.url()) {
        return Err(bepinex_error(
            "bepinex_integrity_error",
            "the BepInEx download redirected to an untrusted host",
        ));
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or_default();
    if !matches!(
        content_type,
        "application/zip" | "application/x-zip-compressed" | "application/octet-stream"
    ) {
        return Err(bepinex_error(
            "bepinex_integrity_error",
            "the BepInEx download was not a ZIP archive",
        ));
    }
    let total_bytes = response.content_length();
    if total_bytes.is_some_and(|total| total > ARCHIVE_MAX_BYTES) {
        return Err(bepinex_error(
            "bepinex_integrity_error",
            "the BepInEx archive was too large",
        ));
    }

    let temporary = NamedTempFile::new_in(game_root).map_err(|_| {
        bepinex_error(
            "bepinex_install_error",
            "a temporary archive could not be created in the game directory",
        )
    })?;
    let temporary = temporary.into_temp_path();
    let archive_path = temporary.to_path_buf();
    let mut output = tokio::fs::File::create(&archive_path).await.map_err(|_| {
        bepinex_error(
            "bepinex_install_error",
            "the temporary BepInEx archive could not be opened",
        )
    })?;
    let mut downloaded_bytes = 0_u64;
    let mut hasher = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        downloaded_bytes = downloaded_bytes.saturating_add(chunk.len() as u64);
        if downloaded_bytes > ARCHIVE_MAX_BYTES {
            return Err(bepinex_error(
                "bepinex_integrity_error",
                "the BepInEx archive exceeded the download limit",
            ));
        }
        output.write_all(&chunk).await.map_err(|_| {
            bepinex_error(
                "bepinex_install_error",
                "the BepInEx archive could not be written",
            )
        })?;
        hasher.update(&chunk);
        emit_progress(
            app,
            app_id,
            InstallStage::Downloading,
            downloaded_bytes,
            total_bytes,
        );
    }
    output.sync_all().await.map_err(|_| {
        bepinex_error(
            "bepinex_install_error",
            "the BepInEx archive could not be finalized",
        )
    })?;
    drop(output);

    let sha256 = format!("{:x}", hasher.finalize());
    if package
        .expected_digest
        .as_ref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&sha256))
    {
        return Err(bepinex_error(
            "bepinex_integrity_error",
            "the BepInEx archive failed SHA-256 verification",
        ));
    }
    Ok((archive_path, temporary, sha256))
}

fn install_archive(
    archive_path: &Path,
    target: &InstallTarget,
    package: &PackageInfo,
    sha256: &str,
    app_id: u32,
) -> AppResult<()> {
    let staging = TempBuilder::new()
        .prefix(".gametweaks-bepinex-")
        .tempdir_in(&target.game_root)
        .map_err(|_| {
            bepinex_error(
                "bepinex_install_error",
                "a BepInEx staging directory could not be created",
            )
        })?;
    let content = staging.path().join("content");
    fs::create_dir(&content).map_err(|_| {
        bepinex_error(
            "bepinex_install_error",
            "the BepInEx staging directory could not be prepared",
        )
    })?;
    let files = extract_validated_archive(archive_path, &content, target.runtime)?;

    let mut top_level = Vec::new();
    for entry in fs::read_dir(&content).map_err(|_| {
        bepinex_error(
            "bepinex_install_error",
            "the extracted BepInEx package could not be inspected",
        )
    })? {
        let entry = entry.map_err(|_| {
            bepinex_error(
                "bepinex_install_error",
                "the extracted BepInEx package could not be inspected",
            )
        })?;
        let destination = target.game_root.join(entry.file_name());
        if destination.exists() {
            return Err(bepinex_error(
                "bepinex_blocked",
                "an existing game file conflicts with the BepInEx package",
            ));
        }
        top_level.push((entry.path(), destination));
    }
    top_level.sort_by(|left, right| left.1.cmp(&right.1));

    let mut created = Vec::new();
    for (source, destination) in top_level {
        if let Err(error) = fs::rename(&source, &destination) {
            rollback_created(&created);
            tracing::warn!(%error, "failed to commit a BepInEx package entry");
            return Err(bepinex_error(
                "bepinex_install_error",
                "BepInEx could not be committed to the game directory",
            ));
        }
        created.push(destination);
    }

    let marker = InstallMarker {
        schema_version: 1,
        app_id,
        version: package.version.clone(),
        runtime: target.runtime.label().to_owned(),
        architecture: target.architecture.asset_label().to_owned(),
        source: package.source.label().to_owned(),
        asset_name: package.asset_name.clone(),
        sha256: sha256.to_owned(),
        files,
    };
    let marker_path = target
        .game_root
        .join("BepInEx")
        .join(".gametweaks-install.json");
    let marker_result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker_path)
        .and_then(|mut file| {
            serde_json::to_writer_pretty(&mut file, &marker).map_err(std::io::Error::other)?;
            file.write_all(b"\n")?;
            file.sync_all()
        });
    if let Err(error) = marker_result {
        rollback_created(&created);
        tracing::warn!(%error, "failed to write the GameTweaks BepInEx marker");
        return Err(bepinex_error(
            "bepinex_install_error",
            "the BepInEx installation marker could not be written",
        ));
    }
    Ok(())
}

fn extract_validated_archive(
    archive_path: &Path,
    destination: &Path,
    runtime: BepInExRuntime,
) -> AppResult<Vec<String>> {
    let file = File::open(archive_path).map_err(|_| {
        bepinex_error(
            "bepinex_integrity_error",
            "the downloaded BepInEx archive could not be opened",
        )
    })?;
    let mut archive = ZipArchive::new(file).map_err(|_| {
        bepinex_error(
            "bepinex_integrity_error",
            "the downloaded BepInEx archive was invalid",
        )
    })?;
    if archive.is_empty() || archive.len() > ARCHIVE_MAX_ENTRIES {
        return Err(bepinex_error(
            "bepinex_integrity_error",
            "the BepInEx archive contained an invalid number of entries",
        ));
    }

    let mut total_size = 0_u64;
    let mut files = Vec::new();
    let mut paths = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| {
            bepinex_error(
                "bepinex_integrity_error",
                "a BepInEx archive entry was invalid",
            )
        })?;
        if entry.encrypted() {
            return Err(bepinex_error(
                "bepinex_integrity_error",
                "encrypted BepInEx archives are not supported",
            ));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(bepinex_error(
                "bepinex_integrity_error",
                "the BepInEx archive contained a symbolic link",
            ));
        }
        let path = entry.enclosed_name().ok_or_else(|| {
            bepinex_error(
                "bepinex_integrity_error",
                "the BepInEx archive contained an unsafe path",
            )
        })?;
        validate_archive_path(&path)?;
        total_size = total_size.checked_add(entry.size()).ok_or_else(|| {
            bepinex_error(
                "bepinex_integrity_error",
                "the BepInEx archive size was invalid",
            )
        })?;
        if total_size > ARCHIVE_MAX_UNCOMPRESSED_BYTES {
            return Err(bepinex_error(
                "bepinex_integrity_error",
                "the extracted BepInEx package would be too large",
            ));
        }
        let path_string = path.to_string_lossy().replace('\\', "/");
        if !paths.insert(path_string.clone()) {
            return Err(bepinex_error(
                "bepinex_integrity_error",
                "the BepInEx archive contained a duplicate path",
            ));
        }
        if !entry.is_dir() {
            files.push(path_string);
        }
    }

    let required = match runtime {
        BepInExRuntime::Mono => vec![
            "BepInEx/core/BepInEx.dll",
            "doorstop_config.ini",
            "winhttp.dll",
        ],
        BepInExRuntime::Il2Cpp => vec![
            "BepInEx/core/BepInEx.Core.dll",
            "BepInEx/core/BepInEx.Unity.IL2CPP.dll",
            "doorstop_config.ini",
            "winhttp.dll",
        ],
    };
    if required.iter().any(|required| !paths.contains(*required)) {
        return Err(bepinex_error(
            "bepinex_integrity_error",
            "the BepInEx archive did not match the detected Unity runtime",
        ));
    }

    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| {
            bepinex_error(
                "bepinex_integrity_error",
                "a BepInEx archive entry could not be reopened",
            )
        })?;
        let path = entry.enclosed_name().ok_or_else(|| {
            bepinex_error(
                "bepinex_integrity_error",
                "the BepInEx archive contained an unsafe path",
            )
        })?;
        let output = destination.join(path);
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|_| {
                bepinex_error(
                    "bepinex_install_error",
                    "a BepInEx directory could not be extracted",
                )
            })?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|_| {
                bepinex_error(
                    "bepinex_install_error",
                    "a BepInEx directory could not be extracted",
                )
            })?;
        }
        let mut output_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|_| {
                bepinex_error(
                    "bepinex_install_error",
                    "a BepInEx file could not be extracted",
                )
            })?;
        let expected_size = entry.size();
        let mut limited_entry = entry.take(expected_size.saturating_add(1));
        let copied = std::io::copy(&mut limited_entry, &mut output_file).map_err(|_| {
            bepinex_error(
                "bepinex_install_error",
                "a BepInEx file could not be extracted",
            )
        })?;
        if copied != expected_size {
            return Err(bepinex_error(
                "bepinex_integrity_error",
                "a BepInEx archive entry had an invalid extracted size",
            ));
        }
        output_file.sync_all().map_err(|_| {
            bepinex_error(
                "bepinex_install_error",
                "an extracted BepInEx file could not be finalized",
            )
        })?;
    }
    Ok(files)
}

fn validate_archive_path(path: &Path) -> AppResult<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(bepinex_error(
            "bepinex_integrity_error",
            "the BepInEx archive contained an unsafe path",
        ));
    }
    let top_level = path
        .components()
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .ok_or_else(|| {
            bepinex_error(
                "bepinex_integrity_error",
                "the BepInEx archive contained an invalid path",
            )
        })?;
    if !matches!(
        top_level,
        "BepInEx"
            | "dotnet"
            | ".doorstop_version"
            | "doorstop_config.ini"
            | "winhttp.dll"
            | "changelog.txt"
            | "README.md"
            | "LICENSE"
    ) {
        return Err(bepinex_error(
            "bepinex_integrity_error",
            "the BepInEx archive contained an unexpected top-level entry",
        ));
    }
    Ok(())
}

fn rollback_created(created: &[PathBuf]) {
    for path in created.iter().rev() {
        let result = if path.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
        if let Err(error) = result {
            tracing::error!(%error, "failed to roll back a newly created BepInEx entry");
        }
    }
}

fn emit_progress(
    app: &AppHandle,
    app_id: u32,
    stage: InstallStage,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    let percentage = total_bytes
        .filter(|total| *total > 0)
        .map(|total| ((downloaded_bytes.saturating_mul(100)) / total).min(100));
    let _ = app.emit(
        INSTALL_PROGRESS_EVENT,
        InstallProgress {
            app_id,
            stage,
            downloaded_bytes,
            total_bytes,
            percentage,
        },
    );
}

fn status_error(status: BepInExGameStatus) -> ErrorResponse {
    let (code, message) = match status.status {
        BepInExAvailability::Installed => (
            "bepinex_already_installed",
            "BepInEx is already installed for this game",
        ),
        BepInExAvailability::Unsupported => (
            "bepinex_unsupported",
            "this game is not supported for automatic BepInEx installation",
        ),
        BepInExAvailability::Blocked => (
            "bepinex_blocked",
            "automatic BepInEx installation is blocked for this game",
        ),
        BepInExAvailability::Installable => (
            "bepinex_blocked",
            "the BepInEx installation state was inconsistent",
        ),
    };
    bepinex_error(code, message)
}

fn network_error(_error: reqwest::Error) -> ErrorResponse {
    bepinex_error(
        "bepinex_network_error",
        "the official BepInEx source could not be reached",
    )
}

fn bepinex_error(code: &'static str, message: impl Into<String>) -> ErrorResponse {
    ErrorResponse::from(AppError::BepInEx {
        code,
        message: message.into(),
    })
}

fn random_plan_id() -> AppResult<String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| {
        bepinex_error(
            "bepinex_install_error",
            "a secure BepInEx installation plan could not be created",
        )
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(windows)]
pub(crate) fn ensure_game_stopped(executable: &Path) -> AppResult<()> {
    if windows_process_matches(executable)? {
        return Err(bepinex_error(
            "bepinex_game_running",
            "close the game before installing BepInEx",
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn ensure_game_stopped(_executable: &Path) -> AppResult<()> {
    Ok(())
}

#[cfg(windows)]
fn windows_process_matches(executable: &Path) -> AppResult<bool> {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let target_name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            bepinex_error(
                "bepinex_blocked",
                "the game executable name could not be verified",
            )
        })?;
    let target_path = normalize_windows_path(executable);
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(bepinex_error(
            "bepinex_blocked",
            "running game processes could not be verified",
        ));
    }

    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    let mut matched = false;
    while has_entry {
        let name_length = entry
            .szExeFile
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(entry.szExeFile.len());
        let process_name = String::from_utf16_lossy(&entry.szExeFile[..name_length]);
        if process_name.eq_ignore_ascii_case(target_name) {
            let process =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID) };
            if process.is_null() {
                matched = true;
                break;
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
                matched = true;
                break;
            }
            let process_path = PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
            if normalize_windows_path(&process_path) == target_path {
                matched = true;
                break;
            }
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    Ok(matched)
}

#[cfg(windows)]
fn normalize_windows_path(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn write_pe(path: &Path, machine: u16) {
        let mut raw = vec![0_u8; 0x86];
        raw[..2].copy_from_slice(b"MZ");
        raw[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        raw[0x80..0x84].copy_from_slice(b"PE\0\0");
        raw[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        fs::write(path, raw).unwrap();
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).unwrap();
        let mut writer = ZipWriter::new(file);
        for (name, contents) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn detects_mono_x64_installation() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("Example_Data/Managed")).unwrap();
        write_pe(&directory.path().join("Example.exe"), 0x8664);

        let (status, target) = analyze_windows_installation(directory.path()).unwrap();
        assert_eq!(status.status, BepInExAvailability::Installable);
        assert_eq!(target.runtime, BepInExRuntime::Mono);
        assert_eq!(target.architecture, BepInExArchitecture::X64);
    }

    #[test]
    fn detects_il2cpp_x86_installation() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("Example_Data/il2cpp_data/Metadata")).unwrap();
        fs::write(
            directory
                .path()
                .join("Example_Data/il2cpp_data/Metadata/global-metadata.dat"),
            [],
        )
        .unwrap();
        fs::write(directory.path().join("GameAssembly.dll"), []).unwrap();
        write_pe(&directory.path().join("Example.exe"), 0x014c);

        let (status, target) = analyze_windows_installation(directory.path()).unwrap();
        assert_eq!(status.status, BepInExAvailability::Installable);
        assert_eq!(target.runtime, BepInExRuntime::Il2Cpp);
        assert_eq!(target.architecture, BepInExArchitecture::X86);
    }

    #[test]
    fn blocks_anti_cheat_installation() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("Example_Data/Managed")).unwrap();
        fs::create_dir(directory.path().join("EasyAntiCheat")).unwrap();
        write_pe(&directory.path().join("Example.exe"), 0x8664);

        let status = analyze_windows_installation(directory.path()).unwrap_err();
        assert_eq!(status.reason, Some(BepInExReason::AntiCheatDetected));
    }

    #[test]
    fn rejects_multiple_unity_executables() {
        let directory = tempfile::tempdir().unwrap();
        for name in ["One", "Two"] {
            fs::create_dir_all(directory.path().join(format!("{name}_Data/Managed"))).unwrap();
            write_pe(&directory.path().join(format!("{name}.exe")), 0x8664);
        }

        let status = analyze_windows_installation(directory.path()).unwrap_err();
        assert_eq!(status.reason, Some(BepInExReason::AmbiguousExecutable));
    }

    #[test]
    fn parses_bleeding_edge_asset_names() {
        assert!(valid_bleeding_edge_name(
            "BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.785+6abdba4.zip",
            "785"
        ));
        assert!(!valid_bleeding_edge_name(
            "BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.784+6abdba4.zip",
            "785"
        ));
    }

    #[test]
    fn selects_latest_stable_mono_asset_with_digest() {
        let digest = format!("sha256:{}", "a".repeat(64));
        let releases = vec![
            GitHubRelease {
                draft: false,
                prerelease: false,
                tag_name: "v5.4.22.0".into(),
                assets: Vec::new(),
            },
            GitHubRelease {
                draft: false,
                prerelease: false,
                tag_name: "v5.4.23.5".into(),
                assets: vec![GitHubAsset {
                    name: "BepInEx_win_x64_5.4.23.5.zip".into(),
                    browser_download_url: "https://github.com/BepInEx/BepInEx/releases/download/v5.4.23.5/BepInEx_win_x64_5.4.23.5.zip".into(),
                    digest: Some(digest),
                }],
            },
        ];

        let package = select_mono_package(releases, BepInExArchitecture::X64).unwrap();
        assert_eq!(package.version, "5.4.23.5");
        assert!(package.expected_digest.is_some());
    }

    #[test]
    fn rejects_stable_mono_asset_without_digest() {
        let releases = vec![GitHubRelease {
            draft: false,
            prerelease: false,
            tag_name: "v5.4.23.5".into(),
            assets: vec![GitHubAsset {
                name: "BepInEx_win_x64_5.4.23.5.zip".into(),
                browser_download_url: "https://github.com/BepInEx/BepInEx/releases/download/v5.4.23.5/BepInEx_win_x64_5.4.23.5.zip".into(),
                digest: None,
            }],
        }];

        assert!(select_mono_package(releases, BepInExArchitecture::X64).is_err());
    }

    #[test]
    fn selects_first_matching_official_il2cpp_asset() {
        let html = r#"
            <a href="https://example.com/projects/bepinex_be/999/BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.999%2Babcdef0.zip">bad</a>
            <a href="/projects/bepinex_be/785/BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.785%2B6abdba4.zip">current</a>
            <a href="/projects/bepinex_be/784/BepInEx-Unity.IL2CPP-win-x64-6.0.0-be.784%2B0523d6f.zip">older</a>
        "#;

        let package = select_il2cpp_package(html, BepInExArchitecture::X64).unwrap();
        assert_eq!(package.version, "6.0.0-be.785+6abdba4");
        assert_eq!(package.download_url.host_str(), Some("builds.bepinex.dev"));
    }

    #[test]
    fn validates_and_extracts_a_mono_package() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("mono.zip");
        write_zip(
            &archive,
            &[
                ("BepInEx/core/BepInEx.dll", b"core"),
                ("doorstop_config.ini", b"config"),
                ("winhttp.dll", b"doorstop"),
            ],
        );
        let output = directory.path().join("output");
        fs::create_dir(&output).unwrap();

        let files = extract_validated_archive(&archive, &output, BepInExRuntime::Mono).unwrap();
        assert!(files.contains(&"BepInEx/core/BepInEx.dll".into()));
        assert_eq!(
            fs::read(output.join("BepInEx/core/BepInEx.dll")).unwrap(),
            b"core"
        );
    }

    #[test]
    fn rejects_a_package_for_the_wrong_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("mono.zip");
        write_zip(
            &archive,
            &[
                ("BepInEx/core/BepInEx.dll", b"core"),
                ("doorstop_config.ini", b"config"),
                ("winhttp.dll", b"doorstop"),
            ],
        );
        let output = directory.path().join("output");
        fs::create_dir(&output).unwrap();

        assert!(extract_validated_archive(&archive, &output, BepInExRuntime::Il2Cpp).is_err());
    }

    #[test]
    fn detects_an_existing_installation_marker() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("BepInEx/core")).unwrap();
        fs::write(directory.path().join("BepInEx/core/BepInEx.dll"), []).unwrap();
        fs::write(directory.path().join("doorstop_config.ini"), []).unwrap();
        fs::write(directory.path().join("winhttp.dll"), []).unwrap();
        let marker = InstallMarker {
            schema_version: 1,
            app_id: 10,
            version: "5.4.23.5".into(),
            runtime: "mono".into(),
            architecture: "x64".into(),
            source: "github-stable".into(),
            asset_name: "package.zip".into(),
            sha256: "a".repeat(64),
            files: vec![
                "BepInEx/core/BepInEx.dll".into(),
                "doorstop_config.ini".into(),
                "winhttp.dll".into(),
            ],
        };
        fs::write(
            directory.path().join("BepInEx/.gametweaks-install.json"),
            serde_json::to_vec(&marker).unwrap(),
        )
        .unwrap();

        assert_eq!(
            installed_bepinex_version(directory.path()).unwrap(),
            Some(Some("5.4.23.5".into()))
        );
    }

    #[test]
    fn rejects_archive_traversal_paths() {
        assert!(validate_archive_path(Path::new("../winhttp.dll")).is_err());
        assert!(validate_archive_path(Path::new("game.exe")).is_err());
        assert!(validate_archive_path(Path::new("BepInEx/core/BepInEx.dll")).is_ok());
    }
}
