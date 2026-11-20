use serde::Deserialize;
use std::time::Duration;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

pub(super) fn latest(repository: &str) -> Option<String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?
        .block_on(async {
            // Fetch release metadata only. The browser handles any manual download.
            let bytes = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .user_agent(concat!("Pulseboard/", env!("CARGO_PKG_VERSION")))
                .build()
                .ok()?
                .get(format!(
                    "https://api.github.com/repos/{repository}/releases/latest"
                ))
                .header(reqwest::header::ACCEPT, "application/vnd.github+json")
                .send()
                .await
                .ok()?
                .error_for_status()
                .ok()?
                .bytes()
                .await
                .ok()?;
            let release: Release = serde_json::from_slice(&bytes).ok()?;
            is_newer(env!("CARGO_PKG_VERSION"), &release.tag_name).then_some(release.tag_name)
        })
}

fn is_newer(current: &str, tag: &str) -> bool {
    fn version(value: &str) -> Option<[u64; 3]> {
        let mut parts = value.split('.');
        let mut numbers = [0; 3];
        for number in &mut numbers {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            *number = part.parse().ok()?;
        }
        parts.next().is_none().then_some(numbers)
    }

    tag.strip_prefix('v')
        .and_then(version)
        .zip(version(current))
        .is_some_and(|(release, installed)| release > installed)
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn only_offers_a_newer_stable_release() {
        assert!(is_newer("0.2.2", "v0.2.10"));
        for tag in ["v0.2.2", "v0.2.1", "v0.3.0-beta.1", "v0.3", "latest"] {
            assert!(!is_newer("0.2.2", tag), "{tag}");
        }
    }
}
