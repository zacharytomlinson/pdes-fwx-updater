use anyhow::Context;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct BootstrapExchangeResult {
    pub device_token: String,
    pub app_slug: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
}

pub fn exchange_pkg_url(bootstrap_pkg_url: &str) -> anyhow::Result<BootstrapExchangeResult> {
    let exchange_url = exchange_url_from_pkg_url(bootstrap_pkg_url)?;
    let response = match ureq::post(&exchange_url)
        .timeout(Duration::from_secs(10))
        .set("Accept", "application/json")
        .call()
    {
        Ok(resp) => resp,
        Err(ureq::Error::Status(code, _)) => {
            anyhow::bail!("Bootstrap exchange failed (HTTP {code})")
        }
        Err(ureq::Error::Transport(_)) => {
            anyhow::bail!("Bootstrap exchange failed (network error)")
        }
    };

    let body = response
        .into_string()
        .context("Unable to read bootstrap exchange response")?;

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
        let device_token = find_first_string(&v, &["device_token", "token", "access_token"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .context("Bootstrap exchange response missing device_token")?;

        let app_slug = find_first_string(&v, &["app_slug", "slug"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let os = find_first_string(&v, &["os", "platform_os"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let arch = find_first_string(&v, &["arch", "platform_arch"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        return Ok(BootstrapExchangeResult {
            device_token,
            app_slug,
            os,
            arch,
        });
    }

    let token = body.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("Bootstrap exchange returned an empty token");
    }
    Ok(BootstrapExchangeResult {
        device_token: token,
        app_slug: None,
        os: None,
        arch: None,
    })
}

fn exchange_url_from_pkg_url(bootstrap_pkg_url: &str) -> anyhow::Result<String> {
    // Expected: https://YOUR_HOST/bootstrap/<bootstrap_token>/pkg
    let url = bootstrap_pkg_url.trim();
    if url.is_empty() {
        anyhow::bail!("Bootstrap package URL is empty");
    }

    let (prefix, rest) = url
        .split_once("/bootstrap/")
        .context("Bootstrap URL missing /bootstrap/")?;
    let mut segments = rest.split('/');
    let token = segments
        .next()
        .filter(|s| !s.is_empty())
        .context("Bootstrap URL missing token segment")?;
    let tail = segments.next().unwrap_or_default();
    if tail != "pkg" {
        anyhow::bail!("Bootstrap URL must end with /bootstrap/<token>/pkg");
    }

    let name = device_name();
    let name = url_encode_component(&name);
    Ok(format!("{prefix}/bootstrap/{token}/exchange?name={name}"))
}

fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or_else(|| "pdes-device".to_string())
}

fn url_encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let ch = b as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::{exchange_url_from_pkg_url, url_encode_component};

    #[test]
    fn encodes_query_component() {
        assert_eq!(url_encode_component("a b"), "a%20b");
        assert_eq!(url_encode_component("x/y"), "x%2Fy");
    }

    #[test]
    fn builds_exchange_url() {
        let url = exchange_url_from_pkg_url("https://h/bootstrap/abc/pkg").unwrap();
        assert!(url.starts_with("https://h/bootstrap/abc/exchange?name="));
    }
}
