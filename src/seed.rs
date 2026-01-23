use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};

pub fn ensure_seed_config() -> anyhow::Result<Option<PathBuf>> {
    let dest = crate::app_dirs::updates_config_path()?;
    if dest.exists() {
        return Ok(None);
    }

    if let Some(cfg) = config_from_env()? {
        write_json(&dest, &cfg)?;
        tighten_perms(&dest)?;
        return Ok(Some(dest));
    }

    let candidates = seed_file_candidates()?;
    if let Some(seed) = candidates.into_iter().find(|p| p.exists()) {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Unable to create seed config directory: {}",
                    parent.display()
                )
            })?;
        }
        fs::copy(&seed, &dest).with_context(|| {
            format!(
                "Unable to copy seed config from {} to {}",
                seed.display(),
                dest.display()
            )
        })?;
        tighten_perms(&dest)?;

        if is_next_to_exe(&seed).unwrap_or(false) {
            let _ = fs::remove_file(&seed);
        }

        return Ok(Some(dest));
    }

    Ok(None)
}

pub fn persist_bootstrap_exchange(
    bootstrap_pkg_url: &str,
    exchange: &crate::bootstrap::BootstrapExchangeResult,
) -> anyhow::Result<PathBuf> {
    let dest = crate::app_dirs::updates_config_path()?;
    let mut obj = match fs::read_to_string(&dest)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    {
        Some(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };

    // Do not persist bootstrap secrets long-term.
    obj.remove("bootstrap_pkg_url");

    if let Some(api_base) = derive_api_base_url_from_bootstrap_pkg_url(bootstrap_pkg_url) {
        obj.insert("api_base_url".to_string(), api_base.into());
    }

    if let Some(slug) = exchange
        .app_slug
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        obj.insert("app_slug".to_string(), slug.trim().into());
    }

    let runtime_os = std::env::consts::OS;
    let runtime_arch = std::env::consts::ARCH;
    let platform_os = exchange
        .os
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s == runtime_os)
        .unwrap_or(runtime_os);
    let platform_arch = exchange
        .arch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s == runtime_arch)
        .unwrap_or(runtime_arch);
    obj.insert(
        "platform".to_string(),
        serde_json::json!({ "os": platform_os, "arch": platform_arch }),
    );

    let channel = obj
        .get("firmware")
        .and_then(|v| v.get("channel"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("stable");
    obj.insert(
        "firmware".to_string(),
        serde_json::json!({ "channel": channel }),
    );

    write_json(&dest, &serde_json::Value::Object(obj))?;
    tighten_perms(&dest)?;
    Ok(dest)
}

fn config_from_env() -> anyhow::Result<Option<serde_json::Value>> {
    let Ok(bootstrap_pkg_url) = std::env::var("PDES_PKG_URL") else {
        return Ok(None);
    };
    let bootstrap_pkg_url = bootstrap_pkg_url.trim().to_string();
    if bootstrap_pkg_url.is_empty() {
        return Ok(None);
    }

    let mut obj = serde_json::Map::new();
    obj.insert("bootstrap_pkg_url".to_string(), bootstrap_pkg_url.into());

    if let Ok(v) = std::env::var("PDES_API_BASE_URL")
        && !v.trim().is_empty()
    {
        obj.insert("api_base_url".to_string(), v.trim().into());
    }
    if let Ok(v) = std::env::var("PDES_APP_SLUG")
        && !v.trim().is_empty()
    {
        obj.insert("app_slug".to_string(), v.trim().into());
    }
    if let Ok(v) = std::env::var("PDES_FIRMWARE_CHANNEL")
        && !v.trim().is_empty()
    {
        obj.insert(
            "firmware".to_string(),
            serde_json::json!({ "channel": v.trim() }),
        );
    }

    Ok(Some(serde_json::Value::Object(obj)))
}

fn derive_api_base_url_from_bootstrap_pkg_url(url: &str) -> Option<String> {
    let (prefix, _) = url.split_once("/bootstrap/")?;
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return None;
    }
    Some(format!("{prefix}/api"))
}

fn seed_file_candidates() -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();

    if let Ok(path) = std::env::var("PDES_SEED_CONFIG_PATH") {
        out.push(PathBuf::from(path));
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        out.push(dir.join("seed-config.json"));
        out.push(dir.join("pdes-updater-seed.json"));
        out.push(dir.join("config.seed.json"));
    }

    // Prefer Linux-named /opt root, but keep legacy path for older installs.
    out.push(PathBuf::from("/opt/pdes-fw-updater/seed-config.json"));
    out.push(PathBuf::from("/opt/pdes-fw-updater/config.seed.json"));
    out.push(PathBuf::from("/opt/pdes-win-fw-updater/seed-config.json"));
    out.push(PathBuf::from("/opt/pdes-win-fw-updater/config.seed.json"));

    Ok(out)
}

fn write_json(dest: &Path, v: &serde_json::Value) -> anyhow::Result<()> {
    let Some(parent) = dest.parent() else {
        anyhow::bail!("Invalid seed config path: {}", dest.display());
    };
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "Unable to create seed config directory: {}",
            parent.display()
        )
    })?;

    let tmp = parent.join("config.json.tmp");
    let bytes = serde_json::to_vec_pretty(v).context("Unable to serialize seed config")?;
    fs::write(&tmp, bytes).with_context(|| format!("Unable to write {}", tmp.display()))?;
    fs::rename(&tmp, dest)
        .with_context(|| format!("Unable to move seed config into place: {}", dest.display()))?;
    Ok(())
}

fn tighten_perms(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Unable to set 0600 on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn is_next_to_exe(path: &Path) -> anyhow::Result<bool> {
    let exe = std::env::current_exe().context("Unable to resolve current executable")?;
    let Some(exe_dir) = exe.parent() else {
        return Ok(false);
    };
    Ok(path.parent() == Some(exe_dir))
}
