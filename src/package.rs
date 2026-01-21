use anyhow::Context;
use serde::Deserialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::Builder;

use crate::downloader::user_downloads_dir;

#[derive(Debug, Clone)]
pub struct InstallerPlan {
    pub installer_path: PathBuf,
}

pub fn extract_pkg_and_find_installer(pkg_path: &Path) -> anyhow::Result<InstallerPlan> {
    let downloads = user_downloads_dir()?;
    let base = downloads.join("PDES").join("updates");
    fs::create_dir_all(&base).with_context(|| format!("Unable to create {}", base.display()))?;

    let extract_root = Builder::new()
        .prefix("pkg-")
        .tempdir_in(&base)
        .context("Unable to create extraction directory")?
        .keep();

    extract_archive(pkg_path, &extract_root)?;
    let payload_root = find_payload_root(&extract_root)?;

    let manifest_path = payload_root.join("manifest.json");
    let manifest = PackageManifest::load(&manifest_path)?;

    let installer_rel = PathBuf::from(&manifest.installer.path);
    let installer_path = payload_root.join(&installer_rel);
    if !installer_path.exists() {
        anyhow::bail!(
            "Installer listed in manifest does not exist: {}",
            installer_path.display()
        );
    }

    Ok(InstallerPlan { installer_path })
}

pub fn launch_installer(installer_path: &Path) -> anyhow::Result<()> {
    launch_installer_impl(installer_path)
}

#[cfg(target_os = "windows")]
fn launch_installer_impl(installer_path: &Path) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = Command::new("msiexec.exe");
    cmd.args(["/i", &installer_path.display().to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().context("Unable to launch MSI installer")?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn launch_installer_impl(installer_path: &Path) -> anyhow::Result<()> {
    // Prefer opening with a GUI handler (software center), falling back to pkexec+dpkg.
    if spawn_if_available("xdg-open", &[installer_path.to_string_lossy().as_ref()]).is_ok() {
        return Ok(());
    }
    if spawn_if_available(
        "pkexec",
        &["dpkg", "-i", installer_path.to_string_lossy().as_ref()],
    )
    .is_ok()
    {
        return Ok(());
    }
    anyhow::bail!(
        "Unable to launch installer automatically. Please install: {}",
        installer_path.display()
    );
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn launch_installer_impl(installer_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!(
        "Automatic installer launch is not implemented for this OS. Installer is at: {}",
        installer_path.display()
    );
}

fn extract_archive(pkg_path: &Path, dest: &Path) -> anyhow::Result<()> {
    let magic = read_magic(pkg_path)?;
    if magic.starts_with(&[0x1f, 0x8b]) {
        // gzip (likely tar.gz)
        run_extract_tar_gz(pkg_path, dest)?;
        return Ok(());
    }
    if magic.starts_with(b"PK") {
        // zip
        if run_extract_unzip(pkg_path, dest).is_ok() {
            return Ok(());
        }
        if run_extract_tar_auto(pkg_path, dest).is_ok() {
            return Ok(());
        }
        anyhow::bail!("Unable to extract zip archive (unzip/tar not available)");
    }

    if run_extract_tar_auto(pkg_path, dest).is_ok() {
        return Ok(());
    }
    if run_extract_unzip(pkg_path, dest).is_ok() {
        return Ok(());
    }
    anyhow::bail!("Unable to extract archive: {}", pkg_path.display());
}

fn read_magic(path: &Path) -> anyhow::Result<Vec<u8>> {
    let mut f =
        fs::File::open(path).with_context(|| format!("Unable to open {}", path.display()))?;
    let mut buf = vec![0u8; 4];
    let n = f.read(&mut buf).context("Unable to read file header")?;
    buf.truncate(n);
    Ok(buf)
}

fn run_extract_tar_gz(pkg_path: &Path, dest: &Path) -> anyhow::Result<()> {
    run_cmd(
        "tar",
        &[
            "-xzf",
            pkg_path.to_string_lossy().as_ref(),
            "-C",
            dest.to_string_lossy().as_ref(),
        ],
    )
}

fn run_extract_tar_auto(pkg_path: &Path, dest: &Path) -> anyhow::Result<()> {
    run_cmd(
        "tar",
        &[
            "-xf",
            pkg_path.to_string_lossy().as_ref(),
            "-C",
            dest.to_string_lossy().as_ref(),
        ],
    )
}

fn run_extract_unzip(pkg_path: &Path, dest: &Path) -> anyhow::Result<()> {
    run_cmd(
        "unzip",
        &[
            "-q",
            "-o",
            pkg_path.to_string_lossy().as_ref(),
            "-d",
            dest.to_string_lossy().as_ref(),
        ],
    )
}

fn run_cmd(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let status = cmd
        .status()
        .with_context(|| format!("Unable to run {program}"))?;
    if !status.success() {
        anyhow::bail!("{program} failed with status: {status}");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn spawn_if_available(program: &str, args: &[&str]) -> anyhow::Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.spawn()
        .with_context(|| format!("Unable to spawn {program}"))?;
    Ok(())
}

fn find_payload_root(extract_root: &Path) -> anyhow::Result<PathBuf> {
    if extract_root.join("manifest.json").exists() {
        return Ok(extract_root.to_path_buf());
    }
    let mut dirs = Vec::new();
    for entry in fs::read_dir(extract_root)
        .with_context(|| format!("Unable to list {}", extract_root.display()))?
    {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            dirs.push(entry.path());
        }
    }
    if dirs.len() == 1 && dirs[0].join("manifest.json").exists() {
        return Ok(dirs.remove(0));
    }
    anyhow::bail!(
        "Unable to locate manifest.json in extracted package at {}",
        extract_root.display()
    );
}

#[derive(Debug, Deserialize)]
struct PackageManifest {
    #[serde(default)]
    installer: InstallerSection,
}

#[derive(Debug, Default, Deserialize)]
struct InstallerSection {
    #[serde(default)]
    path: String,
}

impl PackageManifest {
    fn load(path: &Path) -> anyhow::Result<Self> {
        let data = fs::read_to_string(path)
            .with_context(|| format!("Unable to read {}", path.display()))?;
        let parsed = serde_json::from_str::<Self>(&data)
            .with_context(|| "Invalid manifest.json".to_string())?;
        if parsed.installer.path.is_empty() {
            anyhow::bail!("manifest.json missing installer.path");
        }
        Ok(parsed)
    }
}
