use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Result};

use crate::consts::{plano_docker_image, PLANO_DOCKER_NAME};

/// Get Docker container status.
pub async fn container_status(container: &str) -> Result<String> {
    let output = Command::new("docker")
        .args(["inspect", "--type=container", container])
        .output()?;

    if !output.status.success() {
        return Ok("not found".to_string());
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(json[0]["State"]["Status"]
        .as_str()
        .unwrap_or("")
        .to_string())
}

/// Validate config using Docker.
pub async fn validate_config(plano_config_path: &Path) -> Result<()> {
    let abs_path = std::fs::canonicalize(plano_config_path)?;

    let args = vec![
        "docker".to_string(),
        "run".to_string(),
        "--rm".to_string(),
        "-v".to_string(),
        format!("{}:/app/plano_config.yaml:ro", abs_path.display()),
        "--entrypoint".to_string(),
        "planoai".to_string(),
        plano_docker_image(),
        "render-config".to_string(),
    ];

    let output = Command::new(&args[0]).args(&args[1..]).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{}", stderr.trim());
    }

    Ok(())
}

/// Start Plano in Docker.
pub async fn start_plano(
    plano_config_path: &Path,
    env: &HashMap<String, String>,
    foreground: bool,
) -> Result<()> {
    let abs_path = std::fs::canonicalize(plano_config_path)?;

    // Prepare config (replace localhost → host.docker.internal)
    let config_content = std::fs::read_to_string(&abs_path)?;
    let docker_config = if config_content.contains("localhost") {
        let replaced = config_content.replace("localhost", "host.docker.internal");
        let tmp = std::env::temp_dir().join("plano_config_docker.yaml");
        std::fs::write(&tmp, &replaced)?;
        tmp
    } else {
        abs_path.clone()
    };

    // Get gateway ports
    let config: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&abs_path)?)?;
    let mut gateway_ports = Vec::new();
    if let Some(listeners) = config.get("listeners").and_then(|v| v.as_sequence()) {
        for listener in listeners {
            if let Some(port) = listener.get("port").and_then(|v| v.as_u64()) {
                gateway_ports.push(port as u16);
            }
        }
    }
    if gateway_ports.is_empty() {
        gateway_ports.push(12000);
    }

    // Build docker run command
    let mut docker_args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        PLANO_DOCKER_NAME.to_string(),
    ];

    // Port mappings
    docker_args.extend(["-p".to_string(), "12001:12001".to_string()]);
    docker_args.extend(["-p".to_string(), "19901:9901".to_string()]);
    for port in &gateway_ports {
        docker_args.extend(["-p".to_string(), format!("{port}:{port}")]);
    }

    // Volume
    docker_args.extend([
        "-v".to_string(),
        format!("{}:/app/plano_config.yaml:ro", docker_config.display()),
    ]);

    // Environment variables
    for (k, v) in env {
        docker_args.extend(["-e".to_string(), format!("{k}={v}")]);
    }

    docker_args.extend([
        "--add-host".to_string(),
        "host.docker.internal:host-gateway".to_string(),
        plano_docker_image(),
    ]);

    let output = Command::new("docker").args(&docker_args).output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Docker run failed: {}", stderr.trim());
    }

    // Health check
    let green = console::Style::new().green();
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(60);

    loop {
        let mut all_healthy = true;
        for &port in &gateway_ports {
            if reqwest::get(&format!("http://localhost:{port}/healthz"))
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                continue;
            }
            all_healthy = false;
        }

        if all_healthy {
            eprintln!("{} Plano is running (Docker mode)", green.apply_to("✓"));
            for &port in &gateway_ports {
                eprintln!("  http://localhost:{port}");
            }
            break;
        }

        if start.elapsed() > timeout {
            bail!("Health check timed out after 60s");
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    if foreground {
        // Stream logs
        let mut child = tokio::process::Command::new("docker")
            .args(["logs", "-f", PLANO_DOCKER_NAME])
            .spawn()?;

        tokio::select! {
            _ = child.wait() => {}
            _ = tokio::signal::ctrl_c() => {
                let _ = child.kill().await;
                stop_container().await?;
            }
        }
    }

    Ok(())
}

/// Stop Docker container.
pub async fn stop_container() -> Result<()> {
    let _ = Command::new("docker")
        .args(["stop", PLANO_DOCKER_NAME])
        .output();

    let _ = Command::new("docker")
        .args(["rm", "-f", PLANO_DOCKER_NAME])
        .output();

    let green = console::Style::new().green();
    eprintln!("{} Plano stopped (Docker mode).", green.apply_to("✓"));
    Ok(())
}

/// Stream Docker logs.
pub async fn stream_logs(_debug: bool, follow: bool) -> Result<()> {
    let mut args = vec!["logs".to_string()];
    if follow {
        args.push("-f".to_string());
    }
    args.push(PLANO_DOCKER_NAME.to_string());

    let mut child = tokio::process::Command::new("docker").args(&args).spawn()?;

    tokio::select! {
        result = child.wait() => {
            result?;
        }
        _ = tokio::signal::ctrl_c() => {
            let _ = child.kill().await;
        }
    }

    Ok(())
}
