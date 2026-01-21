use anyhow::Context;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub update_token: Option<String>,
    #[serde(default)]
    pub update_url: Option<String>,
}

impl AppSettings {
    pub fn load_or_default() -> Self {
        match Self::load() {
            Ok(settings) => settings,
            Err(e) => {
                println!("Unable to load settings: {e:#}");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = settings_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Unable to create settings directory: {}", parent.display())
            })?;
        }
        let json = serde_json::to_vec_pretty(self).context("Unable to serialize settings")?;
        write_atomic(&path, &json)?;
        Ok(())
    }

    fn load() -> anyhow::Result<Self> {
        let path = settings_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read(&path).with_context(|| format!("Unable to read {}", path.display()))?;
        let settings =
            serde_json::from_slice::<Self>(&data).context("Unable to parse settings.json")?;
        Ok(settings)
    }
}

fn settings_path() -> anyhow::Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "pdes", "pdes-fw-updater")
        .context("Unable to resolve per-user settings directory")?;
    Ok(dirs.config_dir().join("settings.json"))
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let Some(parent) = dest.parent() else {
        anyhow::bail!("Invalid settings path: {}", dest.display());
    };
    let tmp = parent.join("settings.json.tmp");
    fs::write(&tmp, bytes).with_context(|| format!("Unable to write {}", tmp.display()))?;
    fs::rename(&tmp, dest)
        .with_context(|| format!("Unable to move settings into place: {}", dest.display()))?;
    Ok(())
}
