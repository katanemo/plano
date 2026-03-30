use std::process::Command;

use anyhow::{bail, Result};

use crate::consts::plano_docker_image;
use crate::utils::{find_repo_root, print_cli_header};

pub async fn run(docker: bool) -> Result<()> {
    let dim = console::Style::new().dim();
    let red = console::Style::new().red();
    let bold = console::Style::new().bold();

    if !docker {
        print_cli_header();

        let repo_root = find_repo_root().ok_or_else(|| {
            anyhow::anyhow!(
                "Could not find repository root. Make sure you're inside the plano repository."
            )
        })?;

        let crates_dir = repo_root.join("crates");

        // Check cargo is available
        if which::which("cargo").is_err() {
            eprintln!(
                "{} {} not found. Install Rust: https://rustup.rs",
                red.apply_to("✗"),
                bold.apply_to("cargo")
            );
            std::process::exit(1);
        }

        // Build WASM plugins
        eprintln!(
            "{}",
            dim.apply_to("Building WASM plugins (wasm32-wasip1)...")
        );
        let status = Command::new("cargo")
            .args([
                "build",
                "--release",
                "--target",
                "wasm32-wasip1",
                "-p",
                "llm_gateway",
                "-p",
                "prompt_gateway",
            ])
            .current_dir(&crates_dir)
            .status()?;

        if !status.success() {
            bail!("WASM build failed");
        }

        // Build brightstaff
        eprintln!("{}", dim.apply_to("Building brightstaff (native)..."));
        let status = Command::new("cargo")
            .args(["build", "--release", "-p", "brightstaff"])
            .current_dir(&crates_dir)
            .status()?;

        if !status.success() {
            bail!("brightstaff build failed");
        }

        let wasm_dir = crates_dir.join("target/wasm32-wasip1/release");
        let native_dir = crates_dir.join("target/release");

        println!("\n{}:", bold.apply_to("Build artifacts"));
        println!("  {}", wasm_dir.join("prompt_gateway.wasm").display());
        println!("  {}", wasm_dir.join("llm_gateway.wasm").display());
        println!("  {}", native_dir.join("brightstaff").display());
    } else {
        let repo_root =
            find_repo_root().ok_or_else(|| anyhow::anyhow!("Could not find repository root."))?;

        let dockerfile = repo_root.join("Dockerfile");
        if !dockerfile.exists() {
            bail!("Dockerfile not found at {}", dockerfile.display());
        }

        println!("Building plano image from {}...", repo_root.display());
        let status = Command::new("docker")
            .args([
                "build",
                "-f",
                &dockerfile.to_string_lossy(),
                "-t",
                &plano_docker_image(),
                &repo_root.to_string_lossy(),
                "--add-host=host.docker.internal:host-gateway",
            ])
            .status()?;

        if !status.success() {
            bail!("Docker build failed");
        }

        println!("plano image built successfully.");
    }

    Ok(())
}
