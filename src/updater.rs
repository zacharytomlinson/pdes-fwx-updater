use anyhow::Context;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct UpdateResult {
    pub app: Option<ArtifactUpdate>,
    pub firmware: Option<ArtifactUpdate>,
}

#[derive(Debug, Clone)]
pub struct ArtifactUpdate {
    pub latest_version: String,
    pub download_url: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub enum UpdateCheckState {
    NotConfigured,
    Checking,
    UpToDate,
    UpdateAvailable(UpdateResult),
    Error(String),
}

#[derive(Debug, Deserialize)]
struct ArtifactPayload {
    #[serde(default, alias = "latest_version", alias = "version")]
    version: Option<String>,
    #[serde(default, alias = "download_url", alias = "url")]
    url: Option<String>,
    #[serde(default, alias = "pkg_sha256", alias = "sha256")]
    sha256: Option<String>,
}

pub fn check_for_updates(
    package_latest_url: &str,
    firmware_latest_url: &str,
    token: &str,
) -> anyhow::Result<UpdateResult> {
    let app = fetch_artifact(package_latest_url, token)?;
    let firmware = fetch_artifact(firmware_latest_url, token)?;
    Ok(UpdateResult { app, firmware })
}

fn fetch_artifact(url: &str, token: &str) -> anyhow::Result<Option<ArtifactUpdate>> {
    let response = match ureq::get(url)
        .timeout(Duration::from_secs(10))
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .call()
    {
        Ok(resp) => resp,
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            let snippet = body_snippet(&body);
            if snippet.is_empty() {
                anyhow::bail!("Update check HTTP {code} from {url}");
            }
            anyhow::bail!("Update check HTTP {code} from {url}: {snippet}");
        }
        Err(ureq::Error::Transport(t)) => {
            anyhow::bail!("Update check transport error for {url}: {t}");
        }
    };

    let body = response
        .into_string()
        .with_context(|| format!("Unable to read update response from {url}"))?;

    if let Ok(payload) = serde_json::from_str::<ArtifactPayload>(&body) {
        return Ok(payload.into_update());
    }

    let v: serde_json::Value = serde_json::from_str(&body)
        .with_context(|| format!("Invalid JSON response from {url}: {}", body_snippet(&body)))?;
    Ok(extract_update_from_value(&v))
}

fn body_snippet(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut snippet: String = trimmed
        .replace(['\r', '\n'], " ")
        .chars()
        .take(240)
        .collect();
    if trimmed.chars().count() > 240 {
        snippet.push('…');
    }
    snippet
}

impl ArtifactPayload {
    fn into_update(self) -> Option<ArtifactUpdate> {
        Some(ArtifactUpdate {
            latest_version: self.version?,
            download_url: self.url.and_then(normalize_download_url),
            sha256: self.sha256.and_then(normalize_sha256),
        })
    }
}

fn extract_update_from_value(v: &serde_json::Value) -> Option<ArtifactUpdate> {
    let version = find_first_string(v, &["latest_version", "version", "tag", "app_version"])?;
    let url = find_first_string(v, &["download_url", "url", "package_url", "installer_url"])
        .and_then(normalize_download_url);
    let sha256 = find_first_string(v, &["pkg_sha256", "sha256"]).and_then(normalize_sha256);
    Some(ArtifactUpdate {
        latest_version: version,
        download_url: url,
        sha256,
    })
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

fn normalize_download_url(url: String) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(url);
    }
    let origin = cdn_origin();
    if url.starts_with('/') {
        return Some(format!("{origin}{url}"));
    }
    Some(format!("{origin}/{url}"))
}

fn cdn_origin() -> String {
    std::env::var("PDES_CDN_ORIGIN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://apps.palodurosystems.io".to_string())
}

fn normalize_sha256(sha: String) -> Option<String> {
    let normalized = sha.trim().to_ascii_lowercase();
    if normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(normalized)
    } else {
        None
    }
}

pub fn is_newer_version(current: &str, latest: &str) -> bool {
    compare_versions(current, latest)
        .map(|ord| ord.is_lt())
        .unwrap_or(false)
}

fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let parse = |s: &str| {
        s.split(|c: char| !c.is_ascii_digit() && c != '.')
            .find(|p| p.contains('.'))
            .unwrap_or(s)
            .split('.')
            .map(|p| p.parse::<u64>().ok())
            .collect::<Vec<_>>()
    };

    let a_parts = parse(a);
    let b_parts = parse(b);
    if a_parts.iter().any(|p| p.is_none()) || b_parts.iter().any(|p| p.is_none()) {
        return None;
    }

    let max_len = a_parts.len().max(b_parts.len());
    for idx in 0..max_len {
        let a_num = a_parts.get(idx).copied().flatten().unwrap_or(0);
        let b_num = b_parts.get(idx).copied().flatten().unwrap_or(0);
        match a_num.cmp(&b_num) {
            std::cmp::Ordering::Equal => continue,
            other => return Some(other),
        }
    }
    Some(std::cmp::Ordering::Equal)
}

#[cfg(test)]
mod tests {
    use super::is_newer_version;

    #[test]
    fn compares_versions_by_parts() {
        assert!(is_newer_version("0.1.2", "0.1.3"));
        assert!(is_newer_version("0.1.2", "0.2.0"));
        assert!(is_newer_version("0.1.2", "1.0.0"));
        assert!(!is_newer_version("0.1.2", "0.1.2"));
        assert!(!is_newer_version("0.2.0", "0.1.9"));
    }

    #[test]
    fn tolerates_prefixed_versions() {
        assert!(is_newer_version("v0.1.2", "v0.1.3"));
        assert!(is_newer_version("0.1.2", "release-0.1.3"));
    }
}
