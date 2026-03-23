use std::path::Path;

use anyhow::Result;

use crate::config;

/// Render config files for Docker/supervisord use.
/// Reads paths from environment variables (matching the old Python config_generator).
pub async fn run() -> Result<()> {
    let config_file =
        std::env::var("PLANO_CONFIG_FILE").unwrap_or_else(|_| "/app/plano_config.yaml".to_string());
    let schema_file = std::env::var("PLANO_CONFIG_SCHEMA_FILE")
        .unwrap_or_else(|_| "plano_config_schema.yaml".to_string());
    let template_root = std::env::var("TEMPLATE_ROOT").unwrap_or_else(|_| "./".to_string());
    let template_file = std::env::var("ENVOY_CONFIG_TEMPLATE_FILE")
        .unwrap_or_else(|_| "envoy.template.yaml".to_string());
    let config_rendered = std::env::var("PLANO_CONFIG_FILE_RENDERED")
        .unwrap_or_else(|_| "/app/plano_config_rendered.yaml".to_string());
    let envoy_rendered = std::env::var("ENVOY_CONFIG_FILE_RENDERED")
        .unwrap_or_else(|_| "/etc/envoy/envoy.yaml".to_string());

    let template_path = Path::new(&template_root).join(&template_file);

    config::validate_and_render(
        Path::new(&config_file),
        Path::new(&schema_file),
        &template_path,
        Path::new(&envoy_rendered),
        Path::new(&config_rendered),
    )?;

    Ok(())
}
