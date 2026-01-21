use anyhow::Context;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct UpdateIdentity {
    pub slug: String,
    pub os: String,
    pub arch: String,
    pub firmware_channel: String,
    pub firmware_latest_url: Option<String>,
}

impl UpdateIdentity {
    pub fn load_or_default() -> Self {
        if let Ok(Some(manifest)) = AppManifest::load_any() {
            return manifest.into_identity();
        }
        Self::default_for_current_platform()
    }

    pub fn default_for_current_platform() -> Self {
        Self {
            slug: "pdes-fw-x-updater".to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            firmware_channel: "stable".to_string(),
            firmware_latest_url: None,
        }
    }

    pub fn package_latest_url(&self) -> String {
        format!(
            "{}/apps/{}/package/latest?os={}&arch={}",
            api_base_url(),
            self.slug,
            self.os,
            self.arch
        )
    }

    pub fn firmware_latest_url(&self) -> String {
        if let Some(url) = &self.firmware_latest_url {
            if url.starts_with("http://") || url.starts_with("https://") {
                return url.to_string();
            }
            if url.starts_with('/') {
                return format!("{}{}", api_base_url(), url);
            }
            return format!("{}/{}", api_base_url(), url);
        }
        format!(
            "{}/apps/{}/firmware/latest?channel={}",
            api_base_url(),
            self.slug,
            self.firmware_channel
        )
    }
}

#[derive(Debug, Deserialize)]
struct AppManifest {
    #[serde(default)]
    app_slug: String,
    #[serde(default)]
    firmware: FirmwareSection,
    #[serde(default)]
    platform: PlatformSection,
}

#[derive(Debug, Default, Deserialize)]
struct FirmwareSection {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    latest_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct PlatformSection {
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    arch: Option<String>,
}

impl AppManifest {
    fn load_any() -> anyhow::Result<Option<Self>> {
        for path in candidate_manifest_paths()? {
            if path.exists() {
                return Ok(Some(Self::load_from_path(&path)?));
            }
        }
        Ok(None)
    }

    fn load_from_path(path: &Path) -> anyhow::Result<Self> {
        let data = fs::read_to_string(path)
            .with_context(|| format!("Unable to read manifest {}", path.display()))?;
        let manifest = serde_json::from_str::<Self>(&data)
            .with_context(|| format!("Unable to parse manifest {}", path.display()))?;
        Ok(manifest)
    }

    fn into_identity(self) -> UpdateIdentity {
        let runtime_os = std::env::consts::OS.to_string();
        let runtime_arch = std::env::consts::ARCH.to_string();

        UpdateIdentity {
            slug: if self.app_slug.is_empty() {
                UpdateIdentity::default_for_current_platform().slug
            } else {
                self.app_slug
            },
            os: self
                .platform
                .os
                .filter(|o| o == &runtime_os)
                .unwrap_or(runtime_os),
            arch: self
                .platform
                .arch
                .filter(|a| a == &runtime_arch)
                .unwrap_or(runtime_arch),
            firmware_channel: self
                .firmware
                .channel
                .unwrap_or_else(|| "stable".to_string()),
            firmware_latest_url: self.firmware.latest_url,
        }
    }
}

fn candidate_manifest_paths() -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();

    if let Ok(forced) = std::env::var("PDES_MANIFEST_PATH") {
        paths.push(PathBuf::from(forced));
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        paths.push(dir.join("manifest.json"));
        paths.push(dir.join("assets").join("manifest.json"));
    }

    // Debian package assets live under /opt
    paths.push(PathBuf::from("/opt/pdes-win-fw-updater/manifest.json"));
    paths.push(PathBuf::from(
        "/opt/pdes-win-fw-updater/assets/manifest.json",
    ));

    // Dev convenience
    paths.push(PathBuf::from("manifest.json"));
    paths.push(PathBuf::from("assets/manifest.json"));

    Ok(paths)
}

fn api_base_url() -> &'static str {
    "https://cdn.palodurosystems.io/api"
}
