use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::consts::{
    DEFAULT_NATIVE_OTEL_TRACING_GRPC_ENDPOINT, DEFAULT_OTEL_TRACING_GRPC_ENDPOINT,
};
use crate::utils::{
    find_config_file, get_llm_provider_access_keys, is_port_in_use, load_env_file,
    print_cli_header, print_missing_keys,
};

pub async fn run(
    file: Option<String>,
    path: String,
    foreground: bool,
    with_tracing: bool,
    tracing_port: u16,
    docker: bool,
) -> Result<()> {
    let green = console::Style::new().green();
    let red = console::Style::new().red();
    let dim = console::Style::new().dim();
    let cyan = console::Style::new().cyan();

    print_cli_header();

    let plano_config_file = find_config_file(&path, file.as_deref());

    if !plano_config_file.exists() {
        eprintln!(
            "{} Config file not found: {}",
            red.apply_to("✗"),
            dim.apply_to(plano_config_file.display().to_string())
        );
        std::process::exit(1);
    }

    // Validate configuration
    if !docker {
        eprint!("{}", dim.apply_to("Validating configuration..."));
        match crate::native::runner::validate_config(&plano_config_file) {
            Ok(()) => eprintln!(" {}", green.apply_to("✓")),
            Err(e) => {
                eprintln!("\n{} Validation failed", red.apply_to("✗"));
                eprintln!("  {}", dim.apply_to(format!("{e:#}")));
                std::process::exit(1);
            }
        }
    } else {
        eprint!("{}", dim.apply_to("Validating configuration (Docker)..."));
        match crate::docker::validate_config(&plano_config_file).await {
            Ok(()) => eprintln!(" {}", green.apply_to("✓")),
            Err(e) => {
                eprintln!("\n{} Validation failed", red.apply_to("✗"));
                eprintln!("  {}", dim.apply_to(format!("{e:#}")));
                std::process::exit(1);
            }
        }
    }

    // Set up environment
    let default_otel = if docker {
        DEFAULT_OTEL_TRACING_GRPC_ENDPOINT
    } else {
        DEFAULT_NATIVE_OTEL_TRACING_GRPC_ENDPOINT
    };

    let mut env_stage: HashMap<String, String> = HashMap::new();
    env_stage.insert(
        "OTEL_TRACING_GRPC_ENDPOINT".to_string(),
        default_otel.to_string(),
    );

    // Check access keys
    let access_keys = get_llm_provider_access_keys(&plano_config_file)?;
    let access_keys: Vec<String> = access_keys
        .into_iter()
        .map(|k| k.strip_prefix('$').unwrap_or(&k).to_string())
        .collect();
    let access_keys_set: std::collections::HashSet<_> = access_keys.into_iter().collect();

    let mut missing_keys = Vec::new();
    if !access_keys_set.is_empty() {
        let app_env_file = if let Some(ref f) = file {
            Path::new(f).parent().unwrap_or(Path::new(".")).join(".env")
        } else {
            Path::new(&path).join(".env")
        };

        if !app_env_file.exists() {
            for key in &access_keys_set {
                match std::env::var(key) {
                    Ok(val) => {
                        env_stage.insert(key.clone(), val);
                    }
                    Err(_) => missing_keys.push(key.clone()),
                }
            }
        } else {
            let env_dict = load_env_file(&app_env_file)?;
            for key in &access_keys_set {
                if let Some(val) = env_dict.get(key.as_str()) {
                    env_stage.insert(key.clone(), val.clone());
                } else {
                    missing_keys.push(key.clone());
                }
            }
        }
    }

    if !missing_keys.is_empty() {
        print_missing_keys(&missing_keys);
        std::process::exit(1);
    }

    env_stage.insert(
        "LOG_LEVEL".to_string(),
        std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
    );

    // Handle tracing
    if with_tracing {
        if is_port_in_use(tracing_port) {
            eprintln!(
                "{} Trace collector already running on port {}",
                green.apply_to("✓"),
                cyan.apply_to(tracing_port.to_string())
            );
        } else {
            match crate::trace::listen::start_background(tracing_port).await {
                Ok(()) => {
                    eprintln!(
                        "{} Trace collector listening on {}",
                        green.apply_to("✓"),
                        cyan.apply_to(format!("0.0.0.0:{tracing_port}"))
                    );
                }
                Err(e) => {
                    eprintln!(
                        "{} Failed to start trace collector on port {tracing_port}: {e}",
                        red.apply_to("✗")
                    );
                    std::process::exit(1);
                }
            }
        }

        let tracing_host = if docker {
            "host.docker.internal"
        } else {
            "localhost"
        };
        env_stage.insert(
            "OTEL_TRACING_GRPC_ENDPOINT".to_string(),
            format!("http://{tracing_host}:{tracing_port}"),
        );
    }

    // Build full env
    let mut env: HashMap<String, String> = std::env::vars().collect();
    env.remove("PATH");
    env.extend(env_stage);

    if !docker {
        crate::native::runner::start_native(&plano_config_file, &env, foreground, with_tracing)
            .await?;
    } else {
        crate::docker::start_plano(&plano_config_file, &env, foreground).await?;
    }

    Ok(())
}
