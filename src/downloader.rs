use anyhow::Context;
use directories::UserDirs;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn download_to_user_downloads_verified(
    url: &str,
    token: &str,
    expected_sha256: &str,
) -> anyhow::Result<PathBuf> {
    let downloads_dir = user_downloads_dir()?;
    fs::create_dir_all(&downloads_dir).with_context(|| {
        format!(
            "Unable to create downloads directory: {}",
            downloads_dir.display()
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

    let dest_path = unique_dest_path(&downloads_dir.join(filename));

    let mut reader = response.into_reader();
    let mut file = File::create(&dest_path)
        .with_context(|| format!("Unable to create {}", dest_path.display()))?;

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
        anyhow::bail!(
            "SHA256 mismatch: expected {expected}, got {got} (file saved at {})",
            dest_path.display()
        );
    }

    Ok(dest_path)
}

pub fn user_downloads_dir() -> anyhow::Result<PathBuf> {
    let dirs = UserDirs::new().context("Unable to determine user directories")?;
    if let Some(dir) = dirs.download_dir() {
        return Ok(dir.to_path_buf());
    }
    let home = dirs.home_dir().to_path_buf().join("Downloads");
    Ok(home)
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
