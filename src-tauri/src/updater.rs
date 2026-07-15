use reqwest::redirect::Policy;
use reqwest::Url;
use serde::Deserialize;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

use crate::config::model::UpdateChannel;
use crate::core::error::{AppError, AppResult, ErrorResponse};
use crate::core::state::AppState;

const GITHUB_RELEASES_API_URL: &str =
    "https://api.github.com/repos/NicDev-Studios/GameTweaks/releases";
const DEVELOPMENT_RELEASE_TAG: &str = "DEV_RELEASE";
const UPDATER_MANIFEST_ASSET_NAME: &str = "latest.json";
const UPDATE_PROGRESS_EVENT: &str = "gametweaks-update-progress";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub version: String,
    pub date: Option<String>,
    pub body: Option<String>,
    pub channel: UpdateChannel,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percentage: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    draft: bool,
    prerelease: bool,
    tag_name: String,
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubReleaseAsset {
    name: String,
    browser_download_url: String,
}

pub async fn check_for_update(app: &AppHandle, state: &AppState) -> AppResult<Option<UpdateInfo>> {
    if !crate::version::is_release_build() {
        *state.pending_update.lock().await = None;
        return Ok(None);
    }

    let channel = state.config.read().await.update_channel;
    let endpoint = update_endpoint(channel).await?;
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(updater_error)?
        .build()
        .map_err(updater_error)?;

    let update = updater.check().await.map_err(updater_error)?;
    let Some(update) = update else {
        *state.pending_update.lock().await = None;
        return Ok(None);
    };

    let info = UpdateInfo {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        date: update.date.map(|date| date.to_string()),
        body: update.body.clone(),
        channel,
    };

    *state.pending_update.lock().await = Some(update);

    Ok(Some(info))
}

pub async fn download_and_install_update(app: &AppHandle, state: &AppState) -> AppResult<()> {
    let update =
        state.pending_update.lock().await.clone().ok_or_else(|| {
            ErrorResponse::from(AppError::Updater("no update is available".into()))
        })?;

    let mut downloaded_bytes = 0_u64;
    let app_handle = app.clone();

    update
        .download_and_install(
            |chunk_len, total_bytes| {
                downloaded_bytes = downloaded_bytes.saturating_add(chunk_len as u64);
                let percentage = total_bytes
                    .filter(|total| *total > 0)
                    .map(|total| ((downloaded_bytes * 100) / total).min(100));
                let _ = app_handle.emit(
                    UPDATE_PROGRESS_EVENT,
                    UpdateProgress {
                        downloaded_bytes,
                        total_bytes,
                        percentage,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(updater_error)?;

    *state.pending_update.lock().await = None;

    Ok(())
}

async fn update_endpoint(channel: UpdateChannel) -> AppResult<Url> {
    let releases = github_client()?
        .get(GITHUB_RELEASES_API_URL)
        .send()
        .await
        .map_err(|error| {
            ErrorResponse::from(AppError::Updater(format!(
                "failed to fetch GitHub releases: {error}"
            )))
        })?
        .error_for_status()
        .map_err(|error| {
            ErrorResponse::from(AppError::Updater(format!(
                "GitHub releases request failed: {error}"
            )))
        })?
        .json::<Vec<GitHubRelease>>()
        .await
        .map_err(|error| {
            ErrorResponse::from(AppError::Updater(format!(
                "failed to parse GitHub releases: {error}"
            )))
        })?;

    let release = releases
        .iter()
        .find(|release| {
            !release.draft
                && release.tag_name != DEVELOPMENT_RELEASE_TAG
                && match channel {
                    UpdateChannel::Stable => !release.prerelease,
                    UpdateChannel::Beta => release.prerelease,
                }
        })
        .ok_or_else(|| {
            ErrorResponse::from(AppError::Updater(format!(
                "no published {} release was found",
                update_channel_label(channel)
            )))
        })?;

    let manifest_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == UPDATER_MANIFEST_ASSET_NAME)
        .ok_or_else(|| {
            ErrorResponse::from(AppError::Updater(format!(
                "release {} does not provide {}",
                release.tag_name, UPDATER_MANIFEST_ASSET_NAME
            )))
        })?;

    Url::parse(&manifest_asset.browser_download_url).map_err(|error| {
        ErrorResponse::from(AppError::Updater(format!(
            "failed to parse updater manifest URL for release {}: {error}",
            release.tag_name
        )))
    })
}

fn github_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(Policy::limited(5))
        .user_agent("GameTweaks updater")
        .build()
        .map_err(|error| {
            ErrorResponse::from(AppError::Updater(format!(
                "failed to create GitHub client: {error}"
            )))
        })
}

fn update_channel_label(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => "stable",
        UpdateChannel::Beta => "beta",
    }
}

fn updater_error(error: tauri_plugin_updater::Error) -> ErrorResponse {
    ErrorResponse::from(AppError::Updater(error.to_string()))
}
