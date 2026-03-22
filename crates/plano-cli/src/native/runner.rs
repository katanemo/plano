use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

use crate::config;
use crate::consts::{native_pid_file, plano_run_dir};
use crate::native::binaries;
use crate::utils::{expand_env_vars, find_repo_root, is_pid_alive};

/// Find the config directory containing schema and templates.
fn find_config_dir() -> Result<PathBuf> {
    // Check repo root first
    if let Some(repo_root) = find_repo_root() {
        let config_dir = repo_root.join("config");
        if config_dir.is_dir() && config_dir.join("plano_config_schema.yaml").exists() {
            return Ok(config_dir);
        }
    }

    // Check if installed alongside the binary
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let config_dir = parent.join("config");
            if config_dir.is_dir() {
                return Ok(config_dir);
            }
            // Check ../config (for bin/plano layout)
            if let Some(grandparent) = parent.parent() {
                let config_dir = grandparent.join("config");
                if config_dir.is_dir() {
                    return Ok(config_dir);
                }
            }
        }
    }

    bail!("Could not find config templates. Make sure you're inside the plano repository or have the config directory available.")
}

/// Validate config without starting processes.
pub fn validate_config(plano_config_path: &Path) -> Result<()> {
    let config_dir = find_config_dir()?;
    let run_dir = plano_run_dir();
    fs::create_dir_all(&run_dir)?;

    config::validate_and_render(
        plano_config_path,
        &config_dir.join("plano_config_schema.yaml"),
        &config_dir.join("envoy.template.yaml"),
        &run_dir.join("envoy.yaml"),
        &run_dir.join("plano_config_rendered.yaml"),
    )
}

/// Render native config. Returns (envoy_config_path, plano_config_rendered_path).
pub async fn render_native_config(
    plano_config_path: &Path,
    env: &HashMap<String, String>,
    with_tracing: bool,
) -> Result<(PathBuf, PathBuf)> {
    let run_dir = plano_run_dir();
    fs::create_dir_all(&run_dir)?;

    let (prompt_gw_path, llm_gw_path) = binaries::ensure_wasm_plugins().await?;

    // If --with-tracing, inject tracing config if not already present
    let effective_config_path = if with_tracing {
        let content = fs::read_to_string(plano_config_path)?;
        let mut config: serde_yaml::Value = serde_yaml::from_str(&content)?;

        let tracing = config.as_mapping_mut().and_then(|m| {
            m.entry(serde_yaml::Value::String("tracing".to_string()))
                .or_insert(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
                .as_mapping_mut()
        });

        if let Some(tracing) = tracing {
            if !tracing.contains_key(serde_yaml::Value::String("random_sampling".to_string())) {
                tracing.insert(
                    serde_yaml::Value::String("random_sampling".to_string()),
                    serde_yaml::Value::Number(serde_yaml::Number::from(100)),
                );
            }
        }

        let path = run_dir.join("config_with_tracing.yaml");
        fs::write(&path, serde_yaml::to_string(&config)?)?;
        path
    } else {
        plano_config_path.to_path_buf()
    };

    let envoy_config_path = run_dir.join("envoy.yaml");
    let plano_config_rendered_path = run_dir.join("plano_config_rendered.yaml");
    let config_dir = find_config_dir()?;

    // Temporarily set env vars for config rendering
    for (k, v) in env {
        std::env::set_var(k, v);
    }

    config::validate_and_render(
        &effective_config_path,
        &config_dir.join("plano_config_schema.yaml"),
        &config_dir.join("envoy.template.yaml"),
        &envoy_config_path,
        &plano_config_rendered_path,
    )?;

    // Post-process envoy.yaml: replace Docker paths with local paths
    let mut envoy_content = fs::read_to_string(&envoy_config_path)?;

    envoy_content = envoy_content.replace(
        "/etc/envoy/proxy-wasm-plugins/prompt_gateway.wasm",
        &prompt_gw_path.to_string_lossy(),
    );
    envoy_content = envoy_content.replace(
        "/etc/envoy/proxy-wasm-plugins/llm_gateway.wasm",
        &llm_gw_path.to_string_lossy(),
    );

    // Replace /var/log/ with local log directory
    let log_dir = run_dir.join("logs");
    fs::create_dir_all(&log_dir)?;
    envoy_content = envoy_content.replace("/var/log/", &format!("{}/", log_dir.display()));

    // Platform-specific CA cert path
    if cfg!(target_os = "macos") {
        envoy_content =
            envoy_content.replace("/etc/ssl/certs/ca-certificates.crt", "/etc/ssl/cert.pem");
    }

    fs::write(&envoy_config_path, &envoy_content)?;

    // Run envsubst-equivalent on both rendered files
    for path in [&envoy_config_path, &plano_config_rendered_path] {
        let content = fs::read_to_string(path)?;
        let expanded = expand_env_vars(&content);
        fs::write(path, expanded)?;
    }

    Ok((envoy_config_path, plano_config_rendered_path))
}

/// Start Envoy and brightstaff natively.
pub async fn start_native(
    plano_config_path: &Path,
    env: &HashMap<String, String>,
    foreground: bool,
    with_tracing: bool,
) -> Result<()> {
    let pid_file = native_pid_file();
    let run_dir = plano_run_dir();

    // Stop existing instance
    if pid_file.exists() {
        tracing::info!("Stopping existing Plano instance...");
        stop_native()?;
    }

    let envoy_path = binaries::ensure_envoy_binary().await?;
    binaries::ensure_wasm_plugins().await?;
    let brightstaff_path = binaries::ensure_brightstaff_binary().await?;

    let (envoy_config_path, plano_config_rendered_path) =
        render_native_config(plano_config_path, env, with_tracing).await?;

    tracing::info!("Configuration rendered");

    let log_dir = run_dir.join("logs");
    fs::create_dir_all(&log_dir)?;

    let log_level = env.get("LOG_LEVEL").map(|s| s.as_str()).unwrap_or("info");

    // Build env for subprocesses
    let mut proc_env: HashMap<String, String> = std::env::vars().collect();
    proc_env.insert("RUST_LOG".to_string(), log_level.to_string());
    proc_env.insert(
        "PLANO_CONFIG_PATH_RENDERED".to_string(),
        plano_config_rendered_path.to_string_lossy().to_string(),
    );
    for (k, v) in env {
        proc_env.insert(k.clone(), v.clone());
    }

    // Start brightstaff
    let brightstaff_pid = daemon_exec(
        &[brightstaff_path.to_string_lossy().to_string()],
        &proc_env,
        &log_dir.join("brightstaff.log"),
    )?;
    tracing::info!("Started brightstaff (PID {brightstaff_pid})");

    // Start envoy
    let envoy_pid = daemon_exec(
        &[
            envoy_path.to_string_lossy().to_string(),
            "-c".to_string(),
            envoy_config_path.to_string_lossy().to_string(),
            "--component-log-level".to_string(),
            format!("wasm:{log_level}"),
            "--log-format".to_string(),
            "[%Y-%m-%d %T.%e][%l] %v".to_string(),
        ],
        &proc_env,
        &log_dir.join("envoy.log"),
    )?;
    tracing::info!("Started envoy (PID {envoy_pid})");

    // Save PIDs
    fs::create_dir_all(plano_run_dir())?;
    let pids = serde_json::json!({
        "envoy_pid": envoy_pid,
        "brightstaff_pid": brightstaff_pid,
    });
    fs::write(&pid_file, serde_json::to_string(&pids)?)?;

    // Health check
    let gateway_ports = get_gateway_ports(plano_config_path)?;
    tracing::info!("Waiting for listeners to become healthy...");

    let start = Instant::now();
    let timeout = Duration::from_secs(60);
    let green = console::Style::new().green();

    loop {
        let mut all_healthy = true;
        for &port in &gateway_ports {
            if !health_check_endpoint(&format!("http://localhost:{port}/healthz")).await {
                all_healthy = false;
            }
        }

        if all_healthy {
            eprintln!("{} Plano is running (native mode)", green.apply_to("✓"));
            for &port in &gateway_ports {
                eprintln!("  http://localhost:{port}");
            }
            break;
        }

        if !is_pid_alive(brightstaff_pid) {
            bail!(
                "brightstaff exited unexpectedly. Check logs: {}",
                log_dir.join("brightstaff.log").display()
            );
        }
        if !is_pid_alive(envoy_pid) {
            bail!(
                "envoy exited unexpectedly. Check logs: {}",
                log_dir.join("envoy.log").display()
            );
        }
        if start.elapsed() > timeout {
            stop_native()?;
            bail!(
                "Health check timed out after 60s. Check logs in: {}",
                log_dir.display()
            );
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    if foreground {
        tracing::info!("Running in foreground. Press Ctrl+C to stop.");
        tracing::info!("Logs: {}", log_dir.display());

        let mut log_files = vec![
            log_dir.join("envoy.log").to_string_lossy().to_string(),
            log_dir
                .join("brightstaff.log")
                .to_string_lossy()
                .to_string(),
        ];

        // Add access logs
        if let Ok(entries) = fs::read_dir(&log_dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("access_") && name.ends_with(".log") {
                        log_files.push(entry.path().to_string_lossy().to_string());
                    }
                }
            }
        }

        let mut tail_args = vec!["tail".to_string(), "-f".to_string()];
        tail_args.extend(log_files);

        let mut child = tokio::process::Command::new(&tail_args[0])
            .args(&tail_args[1..])
            .spawn()?;

        tokio::select! {
            _ = child.wait() => {}
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Stopping Plano...");
                let _ = child.kill().await;
                stop_native()?;
            }
        }
    } else {
        tracing::info!("Logs: {}", log_dir.display());
        tracing::info!("Run 'plano down' to stop.");
    }

    Ok(())
}

/// Double-fork daemon execution. Returns the grandchild PID.
fn daemon_exec(args: &[String], env: &HashMap<String, String>, log_path: &Path) -> Result<i32> {
    use std::process::{Command, Stdio};

    let log_file = fs::File::create(log_path)?;

    let child = Command::new(&args[0])
        .args(&args[1..])
        .envs(env)
        .stdin(Stdio::null())
        .stdout(log_file.try_clone()?)
        .stderr(log_file)
        .spawn()?;

    Ok(child.id() as i32)
}

/// Stop natively-running Envoy and brightstaff processes.
pub fn stop_native() -> Result<()> {
    let pid_file = native_pid_file();
    if !pid_file.exists() {
        tracing::info!("No native Plano instance found (PID file missing).");
        return Ok(());
    }

    let content = fs::read_to_string(&pid_file)?;
    let pids: serde_json::Value = serde_json::from_str(&content)?;

    let envoy_pid = pids.get("envoy_pid").and_then(|v| v.as_i64());
    let brightstaff_pid = pids.get("brightstaff_pid").and_then(|v| v.as_i64());

    for (name, pid) in [("envoy", envoy_pid), ("brightstaff", brightstaff_pid)] {
        let Some(pid) = pid else { continue };
        let pid = pid as i32;
        let nix_pid = Pid::from_raw(pid);

        match kill(nix_pid, Signal::SIGTERM) {
            Ok(()) => {
                tracing::info!("Sent SIGTERM to {name} (PID {pid})");
            }
            Err(nix::errno::Errno::ESRCH) => {
                tracing::info!("{name} (PID {pid}) already stopped");
                continue;
            }
            Err(e) => {
                tracing::error!("Error stopping {name} (PID {pid}): {e}");
                continue;
            }
        }

        // Wait for graceful shutdown
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if Instant::now() > deadline {
                let _ = kill(nix_pid, Signal::SIGKILL);
                tracing::info!("Sent SIGKILL to {name} (PID {pid})");
                break;
            }
            if !is_pid_alive(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    let _ = fs::remove_file(&pid_file);
    let green = console::Style::new().green();
    eprintln!("{} Plano stopped (native mode).", green.apply_to("✓"));
    Ok(())
}

/// Stream native logs.
pub fn native_logs(debug: bool, follow: bool) -> Result<()> {
    let log_dir = plano_run_dir().join("logs");
    if !log_dir.is_dir() {
        bail!(
            "No native log directory found at {}. Is Plano running?",
            log_dir.display()
        );
    }

    let mut log_files: Vec<String> = Vec::new();

    // Collect access logs
    if let Ok(entries) = fs::read_dir(&log_dir) {
        let mut access_logs: Vec<_> = entries
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.starts_with("access_") && n.ends_with(".log"))
                    .unwrap_or(false)
            })
            .map(|e| e.path().to_string_lossy().to_string())
            .collect();
        access_logs.sort();
        log_files.extend(access_logs);
    }

    if debug {
        log_files.push(log_dir.join("envoy.log").to_string_lossy().to_string());
        log_files.push(
            log_dir
                .join("brightstaff.log")
                .to_string_lossy()
                .to_string(),
        );
    }

    // Filter to existing files
    log_files.retain(|f| Path::new(f).exists());
    if log_files.is_empty() {
        bail!("No log files found in {}", log_dir.display());
    }

    let mut tail_args = vec!["tail".to_string()];
    if follow {
        tail_args.push("-f".to_string());
    }
    tail_args.extend(log_files);

    let mut child = std::process::Command::new(&tail_args[0])
        .args(&tail_args[1..])
        .spawn()?;

    let _ = child.wait();
    Ok(())
}

/// Get gateway ports from config.
fn get_gateway_ports(plano_config_path: &Path) -> Result<Vec<u16>> {
    let content = fs::read_to_string(plano_config_path)?;
    let config: serde_yaml::Value = serde_yaml::from_str(&content)?;

    let mut ports = Vec::new();
    if let Some(listeners) = config.get("listeners") {
        if let Some(seq) = listeners.as_sequence() {
            for listener in seq {
                if let Some(port) = listener.get("port").and_then(|v| v.as_u64()) {
                    ports.push(port as u16);
                }
            }
        } else if let Some(map) = listeners.as_mapping() {
            for (_, v) in map {
                if let Some(port) = v.get("port").and_then(|v| v.as_u64()) {
                    ports.push(port as u16);
                }
            }
        }
    }

    ports.sort();
    ports.dedup();
    if ports.is_empty() {
        ports.push(12000); // default
    }
    Ok(ports)
}

/// Health check an endpoint.
async fn health_check_endpoint(url: &str) -> bool {
    reqwest::get(url)
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}
