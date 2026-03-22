use std::collections::HashMap;
use std::process::Command;

use anyhow::{bail, Result};

use crate::consts::PLANO_DOCKER_NAME;
use crate::utils::{find_config_file, is_native_plano_running};

pub async fn run(agent_type: &str, file: Option<String>, path: &str, settings: &str) -> Result<()> {
    let native_running = is_native_plano_running();
    let docker_running = if !native_running {
        crate::docker::container_status(PLANO_DOCKER_NAME).await? == "running"
    } else {
        false
    };

    if !native_running && !docker_running {
        bail!("Plano is not running. Start Plano first using 'plano up <config.yaml>' (native or --docker mode).");
    }

    let plano_config_file = find_config_file(path, file.as_deref());
    if !plano_config_file.exists() {
        bail!("Config file not found: {}", plano_config_file.display());
    }

    start_cli_agent(&plano_config_file, agent_type, settings)
}

fn start_cli_agent(
    plano_config_path: &std::path::Path,
    agent_type: &str,
    _settings_json: &str,
) -> Result<()> {
    let config_str = std::fs::read_to_string(plano_config_path)?;
    let config: serde_yaml::Value = serde_yaml::from_str(&config_str)?;

    // Resolve CLI agent endpoint
    let (host, port) = resolve_cli_agent_endpoint(&config)?;
    let base_url = format!("http://{host}:{port}/v1");

    let mut env: HashMap<String, String> = std::env::vars().collect();

    match agent_type {
        "claude" => {
            env.insert("ANTHROPIC_BASE_URL".to_string(), base_url);

            // Check for model alias
            if let Some(model) = config
                .get("model_aliases")
                .and_then(|a| a.get("arch"))
                .and_then(|a| a.get("claude"))
                .and_then(|a| a.get("code"))
                .and_then(|a| a.get("small"))
                .and_then(|a| a.get("fast"))
                .and_then(|a| a.get("target"))
                .and_then(|v| v.as_str())
            {
                env.insert("ANTHROPIC_MODEL".to_string(), model.to_string());
            }

            let status = Command::new("claude").envs(&env).status()?;

            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        "codex" => {
            env.insert("OPENAI_BASE_URL".to_string(), base_url);

            let status = Command::new("codex").envs(&env).status()?;

            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        _ => bail!("Unsupported agent type: {agent_type}"),
    }

    Ok(())
}

fn resolve_cli_agent_endpoint(config: &serde_yaml::Value) -> Result<(String, u16)> {
    // Look for model listener (egress_traffic)
    if let Some(listeners) = config.get("listeners").and_then(|v| v.as_sequence()) {
        for listener in listeners {
            let listener_type = listener.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if listener_type == "model" {
                let host = listener
                    .get("address")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0.0.0.0");
                let port = listener
                    .get("port")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(12000) as u16;
                return Ok((host.to_string(), port));
            }
        }
    }

    // Default
    Ok(("0.0.0.0".to_string(), 12000))
}
