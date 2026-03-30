use crate::consts::{PLANO_GITHUB_REPO, PLANO_VERSION};

/// Get the current CLI version.
pub fn get_version() -> &'static str {
    PLANO_VERSION
}

/// Fetch the latest version from GitHub releases.
pub async fn get_latest_version() -> Option<String> {
    let url = format!("https://api.github.com/repos/{PLANO_GITHUB_REPO}/releases/latest");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;

    let resp = client
        .get(&url)
        .header("User-Agent", "plano-cli")
        .send()
        .await
        .ok()?;

    let json: serde_json::Value = resp.json().await.ok()?;
    let tag = json.get("tag_name")?.as_str()?;
    // Strip leading 'v' if present
    Some(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

/// Check if current version is outdated.
pub fn check_version_status(current: &str, latest: Option<&str>) -> VersionStatus {
    let Some(latest) = latest else {
        return VersionStatus {
            is_outdated: false,
            latest: None,
        };
    };

    let current_parts = parse_version(current);
    let latest_parts = parse_version(latest);

    VersionStatus {
        is_outdated: latest_parts > current_parts,
        latest: Some(latest.to_string()),
    }
}

pub struct VersionStatus {
    pub is_outdated: bool,
    pub latest: Option<String>,
}

fn parse_version(v: &str) -> Vec<u32> {
    v.split('.')
        .filter_map(|s| {
            // Handle pre-release suffixes like "1a1"
            let numeric: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
            numeric.parse().ok()
        })
        .collect()
}

/// Maybe check for updates and print a message.
pub async fn maybe_check_updates() {
    if std::env::var("PLANO_SKIP_VERSION_CHECK").is_ok() {
        return;
    }

    let current = get_version();
    if let Some(latest) = get_latest_version().await {
        let status = check_version_status(current, Some(&latest));
        if status.is_outdated {
            let yellow = console::Style::new().yellow();
            let bold = console::Style::new().bold();
            let dim = console::Style::new().dim();
            println!(
                "\n{} {}",
                yellow.apply_to("⚠ Update available:"),
                bold.apply_to(&latest)
            );
            println!(
                "{}",
                dim.apply_to("Run: cargo install plano-cli  (or download from GitHub releases)")
            );
        } else {
            let dim = console::Style::new().dim();
            println!("{}", dim.apply_to("✓ You're up to date"));
        }
    }
}
