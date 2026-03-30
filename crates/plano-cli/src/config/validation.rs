use anyhow::{bail, Result};
use std::path::Path;

/// Validate a plano config file against the JSON schema.
pub fn validate_prompt_config(config_path: &Path, schema_path: &Path) -> Result<()> {
    let config_str = std::fs::read_to_string(config_path)?;
    let schema_str = std::fs::read_to_string(schema_path)?;

    let config_yaml: serde_yaml::Value = serde_yaml::from_str(&config_str)?;
    let schema_yaml: serde_yaml::Value = serde_yaml::from_str(&schema_str)?;

    // Convert to JSON for jsonschema validation
    let config_json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&config_yaml)?)?;
    let schema_json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&schema_yaml)?)?;

    let validator = jsonschema::validator_for(&schema_json)
        .map_err(|e| anyhow::anyhow!("Invalid schema: {e}"))?;

    let errors: Vec<_> = validator.iter_errors(&config_json).collect();
    if !errors.is_empty() {
        let mut msg = String::new();
        for err in &errors {
            let path = if err.instance_path.as_str().is_empty() {
                "root".to_string()
            } else {
                err.instance_path.to_string()
            };
            msg.push_str(&format!(
                "{}\n  Location: {}\n  Value: {}\n",
                err, path, err.instance
            ));
        }
        bail!("{msg}");
    }

    Ok(())
}
