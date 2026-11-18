//! A read-only check for a stable server release. This never downloads or installs code.

use semver::Version;
use serde::Deserialize;
use std::time::Duration;

const RELEASE_URL: &str = "https://api.github.com/repos/getnodalia/hito/releases/latest";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn newer_release(
    body: &[u8],
    current: &str,
) -> Result<Option<Version>, Box<dyn std::error::Error>> {
    let release: Release = serde_json::from_slice(body)?;
    if release.draft || release.prerelease {
        return Err("release is not stable".into());
    }
    let tag = release
        .tag_name
        .strip_prefix('v')
        .ok_or("release tag must begin with v")?;
    let latest = Version::parse(tag)?;
    if !latest.pre.is_empty() || !latest.build.is_empty() {
        return Err("release tag is not a stable version".into());
    }
    let installed = Version::parse(current)?;
    Ok((latest > installed).then_some(latest))
}

pub async fn check() -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("hito-server/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .timeout(Duration::from_secs(15))
        .build()?;
    let mut response = client.get(RELEASE_URL).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err("release response is too large".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("release response is too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    let installed = env!("CARGO_PKG_VERSION");
    Ok(match newer_release(&body, installed)? {
        Some(version) => {
            format!("Installed: {installed}; latest stable: {version}. No update was installed.")
        }
        None => format!("Installed: {installed}; no newer stable release found."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically_and_ignores_older_releases() {
        let release = br#"{"tag_name":"v0.10.0","draft":false,"prerelease":false}"#;
        assert_eq!(
            newer_release(release, "0.9.0").ok().flatten(),
            Version::parse("0.10.0").ok()
        );
        assert!(matches!(newer_release(release, "0.10.0"), Ok(None)));
        assert!(matches!(newer_release(release, "0.11.0"), Ok(None)));
    }

    #[test]
    fn rejects_unstable_or_invalid_responses() {
        for body in [
            r#"{"tag_name":"v0.2.0","draft":true,"prerelease":false}"#,
            r#"{"tag_name":"v0.2.0-rc.1","draft":false,"prerelease":false}"#,
            r#"{"tag_name":"0.2.0","draft":false,"prerelease":false}"#,
            r#"{"tag_name":"v0.2.0","draft":false,"prerelease":true}"#,
            r#"{}"#,
        ] {
            assert!(newer_release(body.as_bytes(), "0.1.3").is_err());
        }
    }
}
