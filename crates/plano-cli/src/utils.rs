use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use regex::Regex;

/// Find the repository root by looking for Dockerfile + crates + config dirs.
pub fn find_repo_root() -> Option<PathBuf> {
    let mut current = std::env::current_dir().ok()?;
    loop {
        if current.join("Dockerfile").exists()
            && current.join("crates").exists()
            && current.join("config").exists()
        {
            return Some(current);
        }
        if current.join(".git").exists() && current.join("crates").exists() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

/// Find the appropriate config file path.
pub fn find_config_file(path: &str, file: Option<&str>) -> PathBuf {
    if let Some(f) = file {
        return PathBuf::from(f)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(f));
    }
    let config_yaml = Path::new(path).join("config.yaml");
    if config_yaml.exists() {
        std::fs::canonicalize(&config_yaml).unwrap_or(config_yaml)
    } else {
        let plano_config = Path::new(path).join("plano_config.yaml");
        std::fs::canonicalize(&plano_config).unwrap_or(plano_config)
    }
}

/// Parse a .env file into a HashMap.
pub fn load_env_file(path: &Path) -> Result<HashMap<String, String>> {
    let content = std::fs::read_to_string(path)?;
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    Ok(map)
}

/// Extract LLM provider access keys from config YAML.
pub fn get_llm_provider_access_keys(config_path: &Path) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(config_path)?;
    let config: serde_yaml::Value = serde_yaml::from_str(&content)?;

    let mut keys = Vec::new();

    // Handle legacy llm_providers → model_providers
    let config = if config.get("llm_providers").is_some() && config.get("model_providers").is_some()
    {
        bail!("Please provide either llm_providers or model_providers, not both.");
    } else {
        config
    };

    // Get model_providers from listeners or root
    let model_providers = config
        .get("model_providers")
        .or_else(|| config.get("llm_providers"));

    // Check prompt_targets for authorization headers
    if let Some(targets) = config.get("prompt_targets").and_then(|v| v.as_sequence()) {
        for target in targets {
            if let Some(headers) = target
                .get("endpoint")
                .and_then(|e| e.get("http_headers"))
                .and_then(|h| h.as_mapping())
            {
                for (k, v) in headers {
                    if let (Some(key), Some(val)) = (k.as_str(), v.as_str()) {
                        if key.to_lowercase() == "authorization" {
                            let tokens: Vec<&str> = val.split(' ').collect();
                            if tokens.len() > 1 {
                                keys.push(tokens[1].to_string());
                            } else {
                                keys.push(val.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Get listeners to find model_providers
    let listeners = config.get("listeners");
    let mp_list = if let Some(listeners) = listeners {
        // Collect model_providers from listeners
        let mut all_mp = Vec::new();
        if let Some(seq) = listeners.as_sequence() {
            for listener in seq {
                if let Some(mps) = listener
                    .get("model_providers")
                    .and_then(|v| v.as_sequence())
                {
                    all_mp.extend(mps.iter());
                }
            }
        }
        // Also check root model_providers
        if let Some(mps) = model_providers.and_then(|v| v.as_sequence()) {
            all_mp.extend(mps.iter());
        }
        all_mp
    } else if let Some(mps) = model_providers.and_then(|v| v.as_sequence()) {
        mps.iter().collect()
    } else {
        Vec::new()
    };

    for mp in &mp_list {
        if let Some(key) = mp.get("access_key").and_then(|v| v.as_str()) {
            keys.push(key.to_string());
        }
    }

    // Extract env vars from state_storage_v1_responses.connection_string
    if let Some(state_storage) = config.get("state_storage_v1_responses") {
        if let Some(conn_str) = state_storage
            .get("connection_string")
            .and_then(|v| v.as_str())
        {
            let re = Regex::new(r"\$\{?([A-Z_][A-Z0-9_]*)\}?")?;
            for cap in re.captures_iter(conn_str) {
                keys.push(format!("${}", &cap[1]));
            }
        }
    }

    Ok(keys)
}

/// Check if a TCP port is already in use.
pub fn is_port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("0.0.0.0", port)).is_err()
}

/// Check if the native Plano is running by verifying the PID file.
pub fn is_native_plano_running() -> bool {
    let pid_file = crate::consts::native_pid_file();
    if !pid_file.exists() {
        return false;
    }
    let content = match std::fs::read_to_string(&pid_file) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let pids: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let envoy_pid = pids.get("envoy_pid").and_then(|v| v.as_i64());
    let brightstaff_pid = pids.get("brightstaff_pid").and_then(|v| v.as_i64());

    match (envoy_pid, brightstaff_pid) {
        (Some(ep), Some(bp)) => is_pid_alive(ep as i32) && is_pid_alive(bp as i32),
        _ => false,
    }
}

/// Check if a process is alive using kill(0).
pub fn is_pid_alive(pid: i32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid), None).is_ok()
}

/// Expand environment variables ($VAR and ${VAR}) in a string.
pub fn expand_env_vars(input: &str) -> String {
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}|\$([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    re.replace_all(input, |caps: &regex::Captures| {
        let var_name = caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");
        std::env::var(var_name).unwrap_or_default()
    })
    .into_owned()
}

/// Print the CLI header with version.
pub fn print_cli_header() {
    let style = console::Style::new().bold().color256(141);
    let dim = console::Style::new().dim();
    println!(
        "\n{} {}\n",
        style.apply_to("Plano CLI"),
        dim.apply_to(format!("v{}", crate::consts::PLANO_VERSION))
    );
}

/// Print missing API keys error.
pub fn print_missing_keys(missing_keys: &[String]) {
    let red = console::Style::new().red();
    let bold = console::Style::new().bold();
    let dim = console::Style::new().dim();
    let cyan = console::Style::new().cyan();

    println!(
        "\n{} {}\n",
        red.apply_to("✗"),
        red.apply_to("Missing API keys!")
    );
    for key in missing_keys {
        println!("  {} {}", red.apply_to("•"), bold.apply_to(key));
    }
    println!("\n{}", dim.apply_to("Set the environment variable(s):"));
    for key in missing_keys {
        println!(
            "  {}",
            cyan.apply_to(format!("export {key}=\"your-api-key\""))
        );
    }
    println!(
        "\n{}\n",
        dim.apply_to("Or create a .env file in the config directory.")
    );
}
