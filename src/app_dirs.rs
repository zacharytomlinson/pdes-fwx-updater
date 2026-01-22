use anyhow::Context;
use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;

const QUALIFIER: &str = "com";
const ORGANIZATION: &str = "pdes";
const APPLICATION: &str = "pdes-fw-updater";

fn project_dirs() -> anyhow::Result<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .context("Unable to resolve per-user app directories")
}

pub fn settings_path() -> anyhow::Result<PathBuf> {
    Ok(project_dirs()?.config_dir().join("settings.json"))
}

/// Persistent per-user storage for package management + firmware artifacts.
///
/// Windows: `%LOCALAPPDATA%\\pdes\\pdes-fw-updater`
/// Linux: `$XDG_DATA_HOME/pdes/pdes-fw-updater` (or `~/.local/share/...`)
pub fn data_root_dir() -> anyhow::Result<PathBuf> {
    Ok(project_dirs()?.data_local_dir().to_path_buf())
}

pub fn updates_dir() -> anyhow::Result<PathBuf> {
    Ok(data_root_dir()?.join("updates"))
}

pub fn updates_staging_dir() -> anyhow::Result<PathBuf> {
    Ok(updates_dir()?.join("staging"))
}

pub fn updates_config_path() -> anyhow::Result<PathBuf> {
    Ok(updates_dir()?.join("config.json"))
}

pub fn updates_manifest_path() -> anyhow::Result<PathBuf> {
    Ok(updates_dir()?.join("manifest.json"))
}

pub fn firmware_dir() -> anyhow::Result<PathBuf> {
    Ok(data_root_dir()?.join("firmware"))
}

pub fn ensure_dirs() -> anyhow::Result<()> {
    let data_root = data_root_dir()?;
    fs::create_dir_all(&data_root)
        .with_context(|| format!("Unable to create {}", data_root.display()))?;

    let updates = updates_dir()?;
    fs::create_dir_all(&updates)
        .with_context(|| format!("Unable to create {}", updates.display()))?;

    let staging = updates_staging_dir()?;
    fs::create_dir_all(&staging)
        .with_context(|| format!("Unable to create {}", staging.display()))?;

    let firmware = firmware_dir()?;
    fs::create_dir_all(&firmware)
        .with_context(|| format!("Unable to create {}", firmware.display()))?;

    Ok(())
}
