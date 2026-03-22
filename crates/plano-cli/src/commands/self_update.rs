use std::fs;
use std::os::unix::fs::PermissionsExt;

use anyhow::{bail, Result};

use crate::consts::{PLANO_GITHUB_REPO, PLANO_VERSION};

pub async fn run(target_version: Option<&str>) -> Result<()> {
    let green = console::Style::new().green();
    let bold = console::Style::new().bold();
    let dim = console::Style::new().dim();
    let cyan = console::Style::new().cyan();

    println!(
        "\n{} {}",
        bold.apply_to("planoai"),
        dim.apply_to("self-update")
    );

    // Determine target version
    let version = if let Some(v) = target_version {
        v.to_string()
    } else {
        println!("  {}", dim.apply_to("Checking for latest version..."));
        fetch_latest_version()
            .await?
            .ok_or_else(|| anyhow::anyhow!("Could not determine latest version"))?
    };

    let current = PLANO_VERSION;
    if version == current && target_version.is_none() {
        println!(
            "\n  {} Already up to date ({})",
            green.apply_to("✓"),
            cyan.apply_to(current)
        );
        return Ok(());
    }

    println!(
        "  {} → {}",
        dim.apply_to(format!("Current: {current}")),
        cyan.apply_to(&version)
    );

    // Detect platform
    let platform = get_platform_slug()?;

    // Download URL
    let url = format!(
        "https://github.com/{PLANO_GITHUB_REPO}/releases/download/{version}/planoai-{platform}.gz"
    );

    println!("  {}", dim.apply_to(format!("Downloading from {url}...")));

    let client = reqwest::Client::new();
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        bail!(
            "Download failed: HTTP {}. Version {} may not exist for platform {}.",
            resp.status(),
            version,
            platform
        );
    }

    let gz_bytes = resp.bytes().await?;

    // Decompress
    let mut decoder = flate2::read::GzDecoder::new(&gz_bytes[..]);
    let mut binary_data = Vec::new();
    std::io::copy(&mut decoder, &mut binary_data)?;

    // Find current binary path
    let current_exe = std::env::current_exe()?;
    let exe_path = current_exe.canonicalize()?;

    println!(
        "  {}",
        dim.apply_to(format!("Installing to {}", exe_path.display()))
    );

    // Write to a temp file next to the binary, then atomically rename
    let tmp_path = exe_path.with_extension("update-tmp");
    fs::write(&tmp_path, &binary_data)?;
    fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o755))?;

    // Atomic replace
    fs::rename(&tmp_path, &exe_path)?;

    println!(
        "\n  {} Updated planoai to {}\n",
        green.apply_to("✓"),
        bold.apply_to(&version)
    );

    Ok(())
}

async fn fetch_latest_version() -> Result<Option<String>> {
    let url = format!("https://api.github.com/repos/{PLANO_GITHUB_REPO}/releases/latest");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    let resp = client
        .get(&url)
        .header("User-Agent", "planoai-cli")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let json: serde_json::Value = resp.json().await?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .map(|s| s.strip_prefix('v').unwrap_or(s).to_string());

    Ok(tag)
}

fn get_platform_slug() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("linux", "x86_64") => Ok("linux-amd64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("macos", "x86_64") => {
            bail!("macOS x86_64 (Intel) is not supported.")
        }
        _ => bail!("Unsupported platform: {os}/{arch}"),
    }
}
