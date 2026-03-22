use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use indicatif::{ProgressBar, ProgressStyle};

use crate::consts::{
    plano_bin_dir, plano_plugins_dir, plano_release_base_url, ENVOY_VERSION, PLANO_VERSION,
};
use crate::utils::find_repo_root;

/// Get the platform slug for binary downloads.
fn get_platform_slug() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("linux", "x86_64") => Ok("linux-amd64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("macos", "x86_64") => {
            bail!("macOS x86_64 (Intel) is not supported. Pre-built binaries are only available for Apple Silicon (arm64).");
        }
        _ => bail!(
            "Unsupported platform {os}/{arch}. Supported: linux-amd64, linux-arm64, darwin-arm64"
        ),
    }
}

/// Download a file with a progress bar.
async fn download_file(url: &str, dest: &Path, label: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let resp = client.get(url).send().await?;

    if !resp.status().is_success() {
        bail!("Download failed: HTTP {}", resp.status());
    }

    let total = resp.content_length().unwrap_or(0);
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(&format!(
                "  {label} {{bar:30}} {{percent}}% ({{bytes}}/{{total_bytes}})"
            ))
            .unwrap()
            .progress_chars("█░░"),
    );

    let bytes = resp.bytes().await?;
    pb.set_position(bytes.len() as u64);
    pb.finish();
    println!();

    fs::write(dest, &bytes)?;
    Ok(())
}

/// Check for locally-built WASM plugins.
fn find_local_wasm_plugins() -> Option<(PathBuf, PathBuf)> {
    let repo_root = find_repo_root()?;
    let wasm_dir = repo_root.join("crates/target/wasm32-wasip1/release");
    let prompt_gw = wasm_dir.join("prompt_gateway.wasm");
    let llm_gw = wasm_dir.join("llm_gateway.wasm");
    if prompt_gw.exists() && llm_gw.exists() {
        Some((prompt_gw, llm_gw))
    } else {
        None
    }
}

/// Check for locally-built brightstaff binary.
fn find_local_brightstaff() -> Option<PathBuf> {
    let repo_root = find_repo_root()?;
    let path = repo_root.join("crates/target/release/brightstaff");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Ensure Envoy binary is available. Returns path to binary.
pub async fn ensure_envoy_binary() -> Result<PathBuf> {
    let bin_dir = plano_bin_dir();
    let envoy_path = bin_dir.join("envoy");
    let version_path = bin_dir.join("envoy.version");

    if envoy_path.exists() {
        if let Ok(cached) = fs::read_to_string(&version_path) {
            if cached.trim() == ENVOY_VERSION {
                tracing::info!("Envoy {} (cached)", ENVOY_VERSION);
                return Ok(envoy_path);
            }
            tracing::info!("Envoy version changed, re-downloading...");
        }
    }

    let slug = get_platform_slug()?;
    let url = format!(
        "https://github.com/tetratelabs/archive-envoy/releases/download/{ENVOY_VERSION}/envoy-{ENVOY_VERSION}-{slug}.tar.xz"
    );

    fs::create_dir_all(&bin_dir)?;

    let tmp_path = bin_dir.join("envoy.tar.xz");
    download_file(&url, &tmp_path, &format!("Envoy {ENVOY_VERSION}")).await?;

    tracing::info!("Extracting Envoy {}...", ENVOY_VERSION);

    // Extract using tar command (tar.xz not well supported by Rust tar crate)
    let status = tokio::process::Command::new("tar")
        .args([
            "xf",
            &tmp_path.to_string_lossy(),
            "-C",
            &bin_dir.to_string_lossy(),
        ])
        .status()
        .await?;

    if !status.success() {
        bail!("Failed to extract Envoy archive");
    }

    // Find and move the envoy binary
    let mut found = false;
    for entry in walkdir(&bin_dir)? {
        if entry.file_name() == Some(std::ffi::OsStr::new("envoy")) && entry != envoy_path {
            fs::copy(&entry, &envoy_path)?;
            found = true;
            break;
        }
    }

    // Clean up extracted directories
    for entry in fs::read_dir(&bin_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    let _ = fs::remove_file(&tmp_path);

    if !found && !envoy_path.exists() {
        bail!("Could not find envoy binary in the downloaded archive");
    }

    fs::set_permissions(&envoy_path, fs::Permissions::from_mode(0o755))?;
    fs::write(&version_path, ENVOY_VERSION)?;
    Ok(envoy_path)
}

/// Simple recursive file walker.
fn walkdir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut results = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                results.extend(walkdir(&path)?);
            } else {
                results.push(path);
            }
        }
    }
    Ok(results)
}

/// Ensure WASM plugins are available. Returns (prompt_gw_path, llm_gw_path).
pub async fn ensure_wasm_plugins() -> Result<(PathBuf, PathBuf)> {
    // 1. Local source build
    if let Some(local) = find_local_wasm_plugins() {
        tracing::info!("Using locally-built WASM plugins");
        return Ok(local);
    }

    // 2. Cached download
    let plugins_dir = plano_plugins_dir();
    let version_path = plugins_dir.join("wasm.version");
    let prompt_gw_path = plugins_dir.join("prompt_gateway.wasm");
    let llm_gw_path = plugins_dir.join("llm_gateway.wasm");

    if prompt_gw_path.exists() && llm_gw_path.exists() {
        if let Ok(cached) = fs::read_to_string(&version_path) {
            if cached.trim() == PLANO_VERSION {
                tracing::info!("WASM plugins {} (cached)", PLANO_VERSION);
                return Ok((prompt_gw_path, llm_gw_path));
            }
        }
    }

    // 3. Download
    fs::create_dir_all(&plugins_dir)?;
    let base = plano_release_base_url();

    for (name, dest) in [
        ("prompt_gateway.wasm", &prompt_gw_path),
        ("llm_gateway.wasm", &llm_gw_path),
    ] {
        let url = format!("{base}/{PLANO_VERSION}/{name}.gz");
        let gz_dest = dest.with_extension("wasm.gz");
        download_file(&url, &gz_dest, &format!("{name} ({PLANO_VERSION})")).await?;

        // Decompress
        tracing::info!("Decompressing {name}...");
        let gz_data = fs::read(&gz_dest)?;
        let mut decoder = flate2::read::GzDecoder::new(&gz_data[..]);
        let mut out = fs::File::create(dest)?;
        std::io::copy(&mut decoder, &mut out)?;
        let _ = fs::remove_file(&gz_dest);
    }

    fs::write(&version_path, PLANO_VERSION)?;
    Ok((prompt_gw_path, llm_gw_path))
}

/// Ensure brightstaff binary is available. Returns path.
pub async fn ensure_brightstaff_binary() -> Result<PathBuf> {
    // 1. Local source build
    if let Some(local) = find_local_brightstaff() {
        tracing::info!("Using locally-built brightstaff");
        return Ok(local);
    }

    // 2. Cached download
    let bin_dir = plano_bin_dir();
    let brightstaff_path = bin_dir.join("brightstaff");
    let version_path = bin_dir.join("brightstaff.version");

    if brightstaff_path.exists() {
        if let Ok(cached) = fs::read_to_string(&version_path) {
            if cached.trim() == PLANO_VERSION {
                tracing::info!("brightstaff {} (cached)", PLANO_VERSION);
                return Ok(brightstaff_path);
            }
        }
    }

    // 3. Download
    let slug = get_platform_slug()?;
    let url = format!(
        "{}/{PLANO_VERSION}/brightstaff-{slug}.gz",
        plano_release_base_url()
    );

    fs::create_dir_all(&bin_dir)?;
    let gz_path = bin_dir.join("brightstaff.gz");
    download_file(
        &url,
        &gz_path,
        &format!("brightstaff ({PLANO_VERSION}, {slug})"),
    )
    .await?;

    tracing::info!("Decompressing brightstaff...");
    let gz_data = fs::read(&gz_path)?;
    let mut decoder = flate2::read::GzDecoder::new(&gz_data[..]);
    let mut out = fs::File::create(&brightstaff_path)?;
    std::io::copy(&mut decoder, &mut out)?;
    let _ = fs::remove_file(&gz_path);

    fs::set_permissions(&brightstaff_path, fs::Permissions::from_mode(0o755))?;
    fs::write(&version_path, PLANO_VERSION)?;
    Ok(brightstaff_path)
}
