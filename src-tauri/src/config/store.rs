use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};
use tempfile::NamedTempFile;

use crate::config::model::AppConfig;
use crate::core::error::{AppError, AppResult, ErrorResponse};

const CONFIG_FILE_NAME: &str = "config.json";
const CONFIG_BACKUP_FILE_NAME: &str = "config.json.bak";

fn config_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    let config_dir = app.path().app_config_dir().map_err(|error| {
        AppError::Config(format!("failed to resolve config directory: {error}"))
    })?;

    Ok(config_dir.join(CONFIG_FILE_NAME))
}

pub fn load_config(app: &AppHandle) -> AppResult<AppConfig> {
    let path = config_path(app).map_err(ErrorResponse::from)?;
    let backup_path = path.with_file_name(CONFIG_BACKUP_FILE_NAME);

    if !path.exists() {
        if backup_path.exists() {
            tracing::warn!(path = %backup_path.display(), "primary config is missing; loading backup");
            return read_config(&backup_path);
        }

        return Ok(AppConfig::default());
    }

    match read_config(&path) {
        Ok(config) => Ok(config),
        Err(primary_error) if backup_path.exists() => match read_config(&backup_path) {
            Ok(config) => {
                tracing::warn!(
                    primary_error = %primary_error.message,
                    backup = %backup_path.display(),
                    "primary config is invalid; loading backup"
                );
                Ok(config)
            }
            Err(_) => Err(primary_error),
        },
        Err(error) => Err(error),
    }
}

pub fn save_config(app: &AppHandle, config: &AppConfig) -> AppResult<()> {
    let path = config_path(app).map_err(ErrorResponse::from)?;
    let raw = serde_json::to_vec_pretty(config)
        .map_err(|error| AppError::Config(format!("failed to serialize config: {error}")))?;

    if path.exists() {
        if let Ok(previous_raw) = fs::read(&path) {
            if serde_json::from_slice::<AppConfig>(&previous_raw).is_ok() {
                atomic_write(&path.with_file_name(CONFIG_BACKUP_FILE_NAME), &previous_raw)?;
            }
        }
    }

    atomic_write(&path, &raw)
}

fn read_config(path: &Path) -> AppResult<AppConfig> {
    let raw = fs::read(path).map_err(|error| {
        AppError::Config(format!(
            "failed to read config at {}: {error}",
            path.display()
        ))
    })?;

    serde_json::from_slice(&raw).map_err(|error| {
        ErrorResponse::from(AppError::Config(format!(
            "failed to parse config at {}: {error}",
            path.display()
        )))
    })
}

fn atomic_write(path: &Path, raw: &[u8]) -> AppResult<()> {
    let parent = path.parent().ok_or_else(|| {
        ErrorResponse::from(AppError::Config(format!(
            "config path has no parent: {}",
            path.display()
        )))
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        AppError::Config(format!(
            "failed to create config directory at {}: {error}",
            parent.display()
        ))
    })?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(|error| {
        AppError::Config(format!(
            "failed to create a temporary config in {}: {error}",
            parent.display()
        ))
    })?;
    temporary.write_all(raw).map_err(|error| {
        AppError::Config(format!(
            "failed to write a temporary config in {}: {error}",
            parent.display()
        ))
    })?;
    temporary.as_file_mut().sync_all().map_err(|error| {
        AppError::Config(format!(
            "failed to sync a temporary config in {}: {error}",
            parent.display()
        ))
    })?;

    temporary.persist(path).map_err(|error| {
        AppError::Config(format!(
            "failed to commit config at {}: {error}",
            path.display()
        ))
    })?;

    #[cfg(unix)]
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_the_complete_file() {
        let root = std::env::temp_dir().join(format!(
            "gametweaks-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("config.json");

        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"second");
        fs::remove_dir_all(root).unwrap();
    }
}
