use anyhow::Context;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DeviceTokenStoreOutcome {
    pub path: PathBuf,
    pub perms_tightened: bool,
}

pub fn device_token_path() -> anyhow::Result<PathBuf> {
    Ok(crate::app_dirs::updates_dir()?.join("device_token.txt"))
}

pub fn load() -> anyhow::Result<Option<String>> {
    let path = device_token_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("Unable to read {}", path.display()))?;
    let token = raw.trim().to_string();
    Ok((!token.is_empty()).then_some(token))
}

pub fn store(token: &str) -> anyhow::Result<DeviceTokenStoreOutcome> {
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("Refusing to store empty device token");
    }
    let path = device_token_path()?;
    write_atomic(&path, token.as_bytes())?;
    let perms_tightened = tighten_perms(&path)?;
    Ok(DeviceTokenStoreOutcome {
        path,
        perms_tightened,
    })
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let Some(parent) = dest.parent() else {
        anyhow::bail!("Invalid device token path: {}", dest.display());
    };
    fs::create_dir_all(parent).with_context(|| format!("Unable to create {}", parent.display()))?;
    let tmp = parent.join("device_token.txt.tmp");
    fs::write(&tmp, bytes).with_context(|| format!("Unable to write {}", tmp.display()))?;
    let _ = fs::remove_file(dest);
    fs::rename(&tmp, dest)
        .with_context(|| format!("Unable to move into place: {}", dest.display()))?;
    Ok(())
}

#[cfg(unix)]
fn tighten_perms(path: &Path) -> anyhow::Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("Unable to set 0600 on {}", path.display()))?;
    Ok(true)
}

#[cfg(windows)]
fn tighten_perms(path: &Path) -> anyhow::Result<bool> {
    use std::process::Command;

    // Best-effort: LocalAppData is already per-user, but try to remove inheritance and
    // grant full control to the current user (plus SYSTEM/Administrators).
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "CURRENT_USER".to_string());
    let target = path.to_string_lossy().to_string();
    let status = Command::new("icacls")
        .args([
            &target,
            "/inheritance:r",
            "/grant:r",
            &format!("{user}:F"),
            "/grant:r",
            "SYSTEM:F",
            "/grant:r",
            "Administrators:F",
        ])
        .status();
    Ok(status.is_ok_and(|s| s.success()))
}

#[cfg(not(any(unix, windows)))]
fn tighten_perms(_path: &Path) -> anyhow::Result<bool> {
    Ok(false)
}
