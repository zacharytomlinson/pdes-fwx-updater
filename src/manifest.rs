use anyhow::Context;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

pub fn load_manifest_app_version() -> Option<String> {
    let paths = candidate_manifest_paths().ok()?;
    for path in paths {
        if path.file_name().and_then(|s| s.to_str()) != Some("manifest.json") {
            continue;
        }
        if !path.exists() {
            continue;
        }
        let data = fs::read_to_string(&path).ok()?;
        let v = serde_json::from_str::<serde_json::Value>(&data).ok()?;
        let version = find_first_string(
            &v,
            &[
                "app_version",
                "version",
                "pkg_version",
                "build_version",
                "tag",
            ],
        )?;
        let trimmed = version.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    None
}

fn find_first_string(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    match v {
        serde_json::Value::Object(map) => {
            for k in keys {
                if let Some(val) = map.get(*k).and_then(|x| x.as_str()) {
                    return Some(val.to_string());
                }
            }
            for val in map.values() {
                if let Some(found) = find_first_string(val, keys) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(|x| find_first_string(x, keys)),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct UpdateIdentity {
    pub slug: String,
    pub os: String,
    pub arch: String,
    pub api_base_url: String,
    pub bootstrap_pkg_url: Option<String>,
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
            slug: "pdes-fwx-updater".to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            api_base_url: default_api_base_url(),
            bootstrap_pkg_url: None,
            firmware_channel: "stable".to_string(),
            firmware_latest_url: None,
        }
    }

    pub fn package_latest_url(&self) -> String {
        let target = format!("{}:{}", self.os, self.arch);
        let base = self.api_base_url.trim_end_matches('/');
        format!(
            "{}/apps/{}/package/latest?os={}&arch={}&target={}",
            base, self.slug, self.os, self.arch, target
        )
    }

    pub fn firmware_latest_url(&self) -> String {
        if let Some(url) = &self.firmware_latest_url {
            return join_api_url(&self.api_base_url, url);
        }
        let target = format!("{}:{}", self.os, self.arch);
        let base = self.api_base_url.trim_end_matches('/');
        format!(
            "{}/apps/{}/firmware/latest?channel={}&os={}&arch={}&target={}",
            base, self.slug, self.firmware_channel, self.os, self.arch, target
        )
    }
}

fn join_api_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return base.to_string();
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }

    let owned = (!trimmed.starts_with('/')).then(|| format!("/{trimmed}"));
    let path = owned.as_deref().unwrap_or(trimmed);

    let path = if base.ends_with("/api") && (path == "/api" || path.starts_with("/api/")) {
        &path["/api".len()..]
    } else {
        path
    };

    if path.is_empty() {
        base.to_string()
    } else {
        format!("{base}{path}")
    }
}

#[derive(Debug, Deserialize)]
struct AppManifest {
    #[serde(default)]
    app_slug: String,
    #[serde(default)]
    api_base_url: Option<String>,
    #[serde(default)]
    bootstrap_pkg_url: Option<String>,
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

        let derived_api_base = self
            .bootstrap_pkg_url
            .as_deref()
            .and_then(derive_api_base_url_from_bootstrap_pkg_url);

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
            api_base_url: self
                .api_base_url
                .filter(|s| !s.trim().is_empty())
                .or(derived_api_base)
                .unwrap_or_else(default_api_base_url),
            bootstrap_pkg_url: self.bootstrap_pkg_url,
            firmware_channel: self
                .firmware
                .channel
                .unwrap_or_else(|| "stable".to_string()),
            firmware_latest_url: self.firmware.latest_url,
        }
    }
}

fn derive_api_base_url_from_bootstrap_pkg_url(url: &str) -> Option<String> {
    // Expected: https://YOUR_HOST/bootstrap/<token>/pkg
    let (prefix, _) = url.split_once("/bootstrap/")?;
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return None;
    }
    Some(format!("{prefix}/api"))
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

    // Debian package assets live under /opt (prefer current path, keep legacy for older installs).
    for root in ["/opt/pdes-fwx-updater", "/opt/pdes-win-fw-updater"] {
        paths.push(PathBuf::from(format!("{root}/manifest.json")));
        paths.push(PathBuf::from(format!("{root}/assets/manifest.json")));
        paths.push(PathBuf::from(format!("{root}/config.json")));
        paths.push(PathBuf::from(format!("{root}/updates/config.json")));
    }

    if let Ok(path) = crate::app_dirs::updates_manifest_path() {
        paths.push(path);
    }
    if let Ok(path) = crate::app_dirs::updates_config_path() {
        paths.push(path);
    }

    // Dev convenience
    paths.push(PathBuf::from("manifest.json"));
    paths.push(PathBuf::from("assets/manifest.json"));
    paths.push(PathBuf::from("config.json"));

    Ok(paths)
}

fn default_api_base_url() -> String {
    std::env::var("PDES_API_BASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://apps.palodurosystems.io/api".to_string())
}

#[cfg(test)]
mod tests {
    use super::join_api_url;

    #[test]
    fn join_api_url_dedupes_api_prefix() {
        let base = "https://apps.palodurosystems.io/api";
        assert_eq!(
            join_api_url(
                base,
                "/api/apps/pdes-fwx-updater/firmware/latest?channel=stable"
            ),
            "https://apps.palodurosystems.io/api/apps/pdes-fwx-updater/firmware/latest?channel=stable"
        );
        assert_eq!(
            join_api_url(
                base,
                "api/apps/pdes-fwx-updater/firmware/latest?channel=stable"
            ),
            "https://apps.palodurosystems.io/api/apps/pdes-fwx-updater/firmware/latest?channel=stable"
        );
    }

    #[test]
    fn join_api_url_preserves_non_api_paths() {
        let base = "https://apps.palodurosystems.io/api/";
        assert_eq!(
            join_api_url(base, "/apps/pdes-fwx-updater/package/latest"),
            "https://apps.palodurosystems.io/api/apps/pdes-fwx-updater/package/latest"
        );
    }
}
