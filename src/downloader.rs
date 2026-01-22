use anyhow::Context;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn download_app_pkg_to_staging_verified(
    url: &str,
    token: &str,
    expected_sha256: &str,
) -> anyhow::Result<PathBuf> {
    let dest = crate::app_dirs::updates_staging_dir()?.join("app.pkg");
    download_to_path_verified(&dest, url, token, expected_sha256)?;
    Ok(dest)
}

pub fn download_to_app_firmware_verified(
    url: &str,
    token: &str,
    expected_sha256: &str,
) -> anyhow::Result<PathBuf> {
    let firmware_dir = crate::app_dirs::firmware_dir()?;
    download_to_dir_verified(&firmware_dir, url, token, expected_sha256)
}

pub fn download_to_path_verified(
    dest_path: &Path,
    url: &str,
    token: &str,
    expected_sha256: &str,
) -> anyhow::Result<()> {
    let Some(parent) = dest_path.parent() else {
        anyhow::bail!("Invalid destination path: {}", dest_path.display());
    };
    fs::create_dir_all(parent).with_context(|| format!("Unable to create {}", parent.display()))?;

    let response = ureq::get(url)
        .timeout(Duration::from_secs(30))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .context("Download request failed")?;

    download_response_to_path_verified(response, dest_path, expected_sha256)?;
    Ok(())
}

pub fn download_to_dir_verified(
    dest_dir: &Path,
    url: &str,
    token: &str,
    expected_sha256: &str,
) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(dest_dir).with_context(|| {
        format!(
            "Unable to create destination directory: {}",
            dest_dir.display()
        )
    })?;

    let response = ureq::get(url)
        .timeout(Duration::from_secs(30))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .context("Download request failed")?;

    let filename = filename_from_response(&response).unwrap_or_else(|| {
        let base = url.split('?').next().unwrap_or(url);
        base.split('/')
            .next_back()
            .filter(|s| !s.is_empty())
            .unwrap_or("download.bin")
            .to_string()
    });

    let dest_path = unique_dest_path(&dest_dir.join(filename));
    download_response_to_path_verified(response, &dest_path, expected_sha256)?;
    Ok(dest_path)
}

fn download_response_to_path_verified(
    response: ureq::Response,
    dest_path: &Path,
    expected_sha256: &str,
) -> anyhow::Result<()> {
    let Some(parent) = dest_path.parent() else {
        anyhow::bail!("Invalid destination path: {}", dest_path.display());
    };

    let filename = dest_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download.bin");
    let tmp_path = parent.join(format!("{filename}.tmp"));

    let mut reader = response.into_reader();
    let mut file = File::create(&tmp_path)
        .with_context(|| format!("Unable to create {}", tmp_path.display()))?;

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).context("Download read failed")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .context("Download write failed")?;
    }

    let got = format!("{:x}", hasher.finalize());
    let expected = expected_sha256.trim().to_ascii_lowercase();
    if got != expected {
        let _ = fs::remove_file(&tmp_path);
        anyhow::bail!("SHA256 mismatch: expected {expected}, got {got}");
    }

    // Atomic swap where possible; Windows rename() won't overwrite.
    let _ = fs::remove_file(dest_path);
    fs::rename(&tmp_path, dest_path)
        .with_context(|| format!("Unable to move into place: {}", dest_path.display()))?;

    Ok(())
}

fn filename_from_response(resp: &ureq::Response) -> Option<String> {
    let header = resp.header("content-disposition")?;
    parse_content_disposition_filename(header)
}

fn parse_content_disposition_filename(header: &str) -> Option<String> {
    // Very small parser for: attachment; filename="foo.ext"
    let lower = header.to_ascii_lowercase();
    let idx = lower.find("filename=")?;
    let value = header.get(idx + "filename=".len()..)?.trim();
    let value = value.trim_matches(';').trim();
    let value = value.trim_matches('"').trim_matches('\'').trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn unique_dest_path(initial: &Path) -> PathBuf {
    if !initial.exists() {
        return initial.to_path_buf();
    }
    let parent = initial.parent().unwrap_or_else(|| Path::new("."));
    let stem = initial
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let ext = initial.extension().and_then(|s| s.to_str()).unwrap_or("");

    for idx in 1..=1000 {
        let name = if ext.is_empty() {
            format!("{stem} ({idx})")
        } else {
            format!("{stem} ({idx}).{ext}")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    initial.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::parse_content_disposition_filename;

    #[test]
    fn parses_content_disposition_filename() {
        assert_eq!(
            parse_content_disposition_filename("attachment; filename=\"foo.zip\"").as_deref(),
            Some("foo.zip")
        );
        assert_eq!(
            parse_content_disposition_filename("inline; filename=bar.msi").as_deref(),
            Some("bar.msi")
        );
        assert_eq!(parse_content_disposition_filename("attachment"), None);
    }
}
