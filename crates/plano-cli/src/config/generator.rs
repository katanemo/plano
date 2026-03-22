use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{bail, Result};
use serde_json::json;
use url::Url;

use crate::config::validation::validate_prompt_config;
use crate::consts::DEFAULT_OTEL_TRACING_GRPC_ENDPOINT;
use crate::utils::expand_env_vars;

const SUPPORTED_PROVIDERS_WITH_BASE_URL: &[&str] =
    &["azure_openai", "ollama", "qwen", "amazon_bedrock", "plano"];

const SUPPORTED_PROVIDERS_WITHOUT_BASE_URL: &[&str] = &[
    "deepseek",
    "groq",
    "mistral",
    "openai",
    "gemini",
    "anthropic",
    "together_ai",
    "xai",
    "moonshotai",
    "zhipu",
];

fn all_supported_providers() -> Vec<&'static str> {
    let mut all = Vec::new();
    all.extend_from_slice(SUPPORTED_PROVIDERS_WITHOUT_BASE_URL);
    all.extend_from_slice(SUPPORTED_PROVIDERS_WITH_BASE_URL);
    all
}

/// Get endpoint and port from an endpoint string.
fn get_endpoint_and_port(endpoint: &str, protocol: &str) -> (String, u16) {
    if let Some((host, port_str)) = endpoint.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            return (host.to_string(), port);
        }
    }
    let port = if protocol == "http" { 80 } else { 443 };
    (endpoint.to_string(), port)
}

/// Convert legacy dict-style listeners to array format.
pub fn convert_legacy_listeners(
    listeners: &serde_yaml::Value,
    model_providers: &serde_yaml::Value,
) -> Result<(Vec<serde_json::Value>, serde_json::Value, serde_json::Value)> {
    let mp_json: serde_json::Value = serde_json::to_value(model_providers)?;
    let mp_array = if mp_json.is_array() {
        mp_json.clone()
    } else {
        json!([])
    };

    let mut llm_gateway = json!({
        "name": "egress_traffic",
        "type": "model",
        "port": 12000,
        "address": "0.0.0.0",
        "timeout": "30s",
        "model_providers": mp_array,
    });

    let mut prompt_gateway = json!({
        "name": "ingress_traffic",
        "type": "prompt",
        "port": 10000,
        "address": "0.0.0.0",
        "timeout": "30s",
    });

    if listeners.is_null() {
        return Ok((vec![llm_gateway.clone()], llm_gateway, prompt_gateway));
    }

    // Legacy dict format
    if listeners.is_mapping() {
        let mut updated = Vec::new();

        if let Some(egress) = listeners.get("egress_traffic") {
            if let Some(p) = egress.get("port").and_then(|v| v.as_u64()) {
                llm_gateway["port"] = json!(p);
            }
            if let Some(a) = egress.get("address").and_then(|v| v.as_str()) {
                llm_gateway["address"] = json!(a);
            }
            if let Some(t) = egress.get("timeout").and_then(|v| v.as_str()) {
                llm_gateway["timeout"] = json!(t);
            }
        }

        if !mp_array.as_array().is_none_or(|a| a.is_empty()) {
            llm_gateway["model_providers"] = mp_array;
        } else {
            bail!("model_providers cannot be empty when using legacy format");
        }

        updated.push(llm_gateway.clone());

        if let Some(ingress) = listeners.get("ingress_traffic") {
            if !ingress.is_null() && ingress.is_mapping() {
                if let Some(p) = ingress.get("port").and_then(|v| v.as_u64()) {
                    prompt_gateway["port"] = json!(p);
                }
                if let Some(a) = ingress.get("address").and_then(|v| v.as_str()) {
                    prompt_gateway["address"] = json!(a);
                }
                if let Some(t) = ingress.get("timeout").and_then(|v| v.as_str()) {
                    prompt_gateway["timeout"] = json!(t);
                }
                updated.push(prompt_gateway.clone());
            }
        }

        return Ok((updated, llm_gateway, prompt_gateway));
    }

    // Array format
    if let Some(arr) = listeners.as_sequence() {
        let mut result: Vec<serde_json::Value> = Vec::new();
        let mut model_provider_set = false;

        for listener in arr {
            let mut l: serde_json::Value = serde_json::to_value(listener)?;
            let listener_type = l.get("type").and_then(|v| v.as_str()).unwrap_or("");

            if listener_type == "model" {
                if model_provider_set {
                    bail!("Currently only one listener can have model_providers set");
                }
                l["model_providers"] = mp_array.clone();
                model_provider_set = true;
                // Merge into llm_gateway defaults
                if let Some(obj) = l.as_object() {
                    for (k, v) in obj {
                        llm_gateway[k] = v.clone();
                    }
                }
            } else if listener_type == "prompt" {
                if let Some(obj) = l.as_object() {
                    for (k, v) in obj {
                        prompt_gateway[k] = v.clone();
                    }
                }
            }
            result.push(l);
        }

        if !model_provider_set {
            result.push(llm_gateway.clone());
        }

        return Ok((result, llm_gateway, prompt_gateway));
    }

    Ok((vec![llm_gateway.clone()], llm_gateway, prompt_gateway))
}

/// Main config validation and rendering function.
/// Ported from config_generator.py validate_and_render_schema()
pub fn validate_and_render(
    config_path: &Path,
    schema_path: &Path,
    template_path: &Path,
    envoy_output_path: &Path,
    config_output_path: &Path,
) -> Result<()> {
    // Step 1: JSON Schema validation
    validate_prompt_config(config_path, schema_path)?;

    // Step 2: Load and process config
    let config_str = std::fs::read_to_string(config_path)?;
    let mut config_yaml: serde_yaml::Value = serde_yaml::from_str(&config_str)?;

    let mut inferred_clusters: HashMap<String, serde_json::Value> = HashMap::new();

    // Convert legacy llm_providers → model_providers
    if config_yaml.get("llm_providers").is_some() {
        if config_yaml.get("model_providers").is_some() {
            bail!("Please provide either llm_providers or model_providers, not both. llm_providers is deprecated, please use model_providers instead");
        }
        let providers = config_yaml
            .get("llm_providers")
            .cloned()
            .unwrap_or_default();
        config_yaml.as_mapping_mut().unwrap().insert(
            serde_yaml::Value::String("model_providers".to_string()),
            providers,
        );
        config_yaml
            .as_mapping_mut()
            .unwrap()
            .remove(serde_yaml::Value::String("llm_providers".to_string()));
    }

    let listeners_val = config_yaml.get("listeners").cloned().unwrap_or_default();
    let model_providers_val = config_yaml
        .get("model_providers")
        .cloned()
        .unwrap_or_default();

    let (listeners, llm_gateway, prompt_gateway) =
        convert_legacy_listeners(&listeners_val, &model_providers_val)?;

    // Update config with processed listeners
    let listeners_yaml: serde_yaml::Value =
        serde_yaml::from_str(&serde_json::to_string(&listeners)?)?;
    config_yaml.as_mapping_mut().unwrap().insert(
        serde_yaml::Value::String("listeners".to_string()),
        listeners_yaml,
    );

    // Process endpoints from config
    let endpoints_yaml = config_yaml.get("endpoints").cloned().unwrap_or_default();
    let mut endpoints: HashMap<String, serde_json::Value> = if endpoints_yaml.is_mapping() {
        serde_json::from_str(&serde_json::to_string(&endpoints_yaml)?)?
    } else {
        HashMap::new()
    };

    // Process agents and filters → endpoints
    let agents = config_yaml
        .get("agents")
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();
    let filters = config_yaml
        .get("filters")
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();

    let mut agent_id_keys: HashSet<String> = HashSet::new();
    let agents_combined: Vec<_> = agents.iter().chain(filters.iter()).collect();

    for agent in &agents_combined {
        let agent_id = agent.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if !agent_id_keys.insert(agent_id.to_string()) {
            bail!("Duplicate agent id {agent_id}, please provide unique id for each agent");
        }

        let agent_url = agent.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if !agent_id.is_empty() && !agent_url.is_empty() {
            if let Ok(url) = Url::parse(agent_url) {
                if let Some(host) = url.host_str() {
                    let protocol = url.scheme();
                    let port = url
                        .port()
                        .unwrap_or(if protocol == "http" { 80 } else { 443 });
                    endpoints.insert(
                        agent_id.to_string(),
                        json!({
                            "endpoint": host,
                            "port": port,
                            "protocol": protocol,
                        }),
                    );
                }
            }
        }
    }

    // Override inferred clusters with endpoints
    for (name, details) in &endpoints {
        let mut cluster = details.clone();
        // Ensure protocol is always set
        if cluster.get("protocol").is_none() {
            cluster["protocol"] = json!("https");
        }
        if cluster.get("port").is_none() {
            let ep = cluster
                .get("endpoint")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let protocol = cluster
                .get("protocol")
                .and_then(|v| v.as_str())
                .unwrap_or("http");
            let (endpoint, port) = get_endpoint_and_port(ep, protocol);
            cluster["endpoint"] = json!(endpoint);
            cluster["port"] = json!(port);
        }
        // Ensure connect_timeout is set
        if cluster.get("connect_timeout").is_none() {
            cluster["connect_timeout"] = json!("5s");
        }
        inferred_clusters.insert(name.clone(), cluster);
    }

    // Validate prompt_targets reference valid endpoints
    if let Some(targets) = config_yaml
        .get("prompt_targets")
        .and_then(|v| v.as_sequence())
    {
        for target in targets {
            if let Some(name) = target
                .get("endpoint")
                .and_then(|e| e.get("name"))
                .and_then(|n| n.as_str())
            {
                if !inferred_clusters.contains_key(name) {
                    bail!("Unknown endpoint {name}, please add it in endpoints section in your plano_config.yaml file");
                }
            }
        }
    }

    // Process tracing config
    let mut plano_tracing: serde_json::Value = config_yaml
        .get("tracing")
        .map(|v| serde_json::to_value(v).unwrap_or_default())
        .unwrap_or_else(|| json!({}));

    // Resolution order: config yaml > OTEL_TRACING_GRPC_ENDPOINT env var > hardcoded default
    let otel_endpoint = plano_tracing
        .get("opentracing_grpc_endpoint")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            std::env::var("OTEL_TRACING_GRPC_ENDPOINT")
                .unwrap_or_else(|_| DEFAULT_OTEL_TRACING_GRPC_ENDPOINT.to_string())
        });

    // Expand env vars if present
    let otel_endpoint = if otel_endpoint.contains('$') {
        let expanded = expand_env_vars(&otel_endpoint);
        eprintln!("Resolved opentracing_grpc_endpoint to {expanded} after expanding environment variables");
        expanded
    } else {
        otel_endpoint
    };

    // Validate OTEL endpoint
    if !otel_endpoint.is_empty() {
        if let Ok(url) = Url::parse(&otel_endpoint) {
            if url.scheme() != "http" {
                bail!("Invalid opentracing_grpc_endpoint {otel_endpoint}, scheme must be http");
            }
            let path = url.path();
            if !path.is_empty() && path != "/" {
                bail!("Invalid opentracing_grpc_endpoint {otel_endpoint}, path must be empty");
            }
        }
    }
    plano_tracing["opentracing_grpc_endpoint"] = json!(otel_endpoint);

    // Process model providers
    let mut updated_model_providers: Vec<serde_json::Value> = Vec::new();
    let mut model_provider_name_set: HashSet<String> = HashSet::new();
    let mut model_name_keys: HashSet<String> = HashSet::new();
    let mut model_usage_name_keys: HashSet<String> = HashSet::new();
    let mut llms_with_endpoint: Vec<serde_json::Value> = Vec::new();
    let mut llms_with_endpoint_cluster_names: HashSet<String> = HashSet::new();
    let all_providers = all_supported_providers();

    for listener in &listeners {
        let model_providers = match listener.get("model_providers").and_then(|v| v.as_array()) {
            Some(mps) if !mps.is_empty() => mps,
            _ => continue,
        };

        for mp in model_providers {
            let mut mp = mp.clone();

            // Check usage
            if mp.get("usage").and_then(|v| v.as_str()).is_some() {
                // has usage, tracked elsewhere
            }

            let mp_name = mp
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if !mp_name.is_empty() && !model_provider_name_set.insert(mp_name.clone()) {
                bail!("Duplicate model_provider name {mp_name}, please provide unique name for each model_provider");
            }

            let model_name = mp
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Check wildcard
            let is_wildcard = if model_name.contains('/') {
                let tokens: Vec<&str> = model_name.split('/').collect();
                tokens.len() >= 2 && tokens.last() == Some(&"*")
            } else {
                false
            };

            if model_name_keys.contains(&model_name) && !is_wildcard {
                bail!("Duplicate model name {model_name}, please provide unique model name for each model_provider");
            }

            if !is_wildcard {
                model_name_keys.insert(model_name.clone());
            }

            if mp
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty()
            {
                mp["name"] = json!(model_name);
            }
            model_provider_name_set.insert(
                mp.get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            );

            let tokens: Vec<&str> = model_name.split('/').collect();
            if tokens.len() < 2 {
                bail!("Invalid model name {model_name}. Please provide model name in the format <provider>/<model_id> or <provider>/* for wildcards.");
            }

            let provider = tokens[0].trim();
            let is_wildcard = tokens.last().map(|s| s.trim()) == Some("*");

            // Validate wildcard constraints
            if is_wildcard {
                if mp.get("default").and_then(|v| v.as_bool()).unwrap_or(false) {
                    bail!("Model {model_name} is configured as default but uses wildcard (*). Default models cannot be wildcards.");
                }
                if mp
                    .get("routing_preferences")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty())
                {
                    bail!("Model {model_name} has routing_preferences but uses wildcard (*). Models with routing preferences cannot be wildcards.");
                }
            }

            // Validate providers requiring base_url
            if SUPPORTED_PROVIDERS_WITH_BASE_URL.contains(&provider)
                && mp.get("base_url").and_then(|v| v.as_str()).is_none()
            {
                bail!("Provider '{provider}' requires 'base_url' to be set for model {model_name}");
            }

            let model_id = tokens[1..].join("/");

            // Handle unsupported providers
            let mut provider_str = provider.to_string();
            if !is_wildcard && !all_providers.contains(&provider) {
                if mp.get("base_url").is_none() || mp.get("provider_interface").is_none() {
                    bail!("Must provide base_url and provider_interface for unsupported provider {provider} for model {model_name}. Supported providers are: {}", all_providers.join(", "));
                }
                provider_str = mp
                    .get("provider_interface")
                    .and_then(|v| v.as_str())
                    .unwrap_or(provider)
                    .to_string();
            } else if is_wildcard && !all_providers.contains(&provider) {
                if mp.get("base_url").is_none() || mp.get("provider_interface").is_none() {
                    bail!("Must provide base_url and provider_interface for unsupported provider {provider} for wildcard model {model_name}. Supported providers are: {}", all_providers.join(", "));
                }
                provider_str = mp
                    .get("provider_interface")
                    .and_then(|v| v.as_str())
                    .unwrap_or(provider)
                    .to_string();
            } else if all_providers.contains(&provider)
                && mp
                    .get("provider_interface")
                    .and_then(|v| v.as_str())
                    .is_some()
            {
                bail!("Please provide provider interface as part of model name {model_name} using the format <provider>/<model_id>. For example, use 'openai/gpt-3.5-turbo' instead of 'gpt-3.5-turbo' ");
            }

            // Duplicate model_id check
            if !is_wildcard && model_name_keys.contains(&model_id) {
                bail!("Duplicate model_id {model_id}, please provide unique model_id for each model_provider");
            }
            if !is_wildcard {
                model_name_keys.insert(model_id.clone());
            }

            // Validate routing preferences uniqueness
            if let Some(prefs) = mp.get("routing_preferences").and_then(|v| v.as_array()) {
                for pref in prefs {
                    if let Some(name) = pref.get("name").and_then(|v| v.as_str()) {
                        if !model_usage_name_keys.insert(name.to_string()) {
                            bail!("Duplicate routing preference name \"{name}\", please provide unique name for each routing preference");
                        }
                    }
                }
            }

            // Warn if both passthrough_auth and access_key
            if mp
                .get("passthrough_auth")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                && mp.get("access_key").is_some()
            {
                let name = mp.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                eprintln!("WARNING: Model provider '{name}' has both 'passthrough_auth: true' and 'access_key' configured. The access_key will be ignored and the client's Authorization header will be forwarded instead.");
            }

            mp["model"] = json!(model_id);
            mp["provider_interface"] = json!(provider_str);

            // Handle provider vs provider_interface
            if mp.get("provider").is_some() && mp.get("provider_interface").is_some() {
                bail!("Please provide either provider or provider_interface, not both");
            }
            if let Some(p) = mp.get("provider").cloned() {
                mp["provider_interface"] = p;
                mp.as_object_mut().unwrap().remove("provider");
            }

            updated_model_providers.push(mp.clone());

            // Handle base_url → endpoint extraction
            if let Some(base_url) = mp.get("base_url").and_then(|v| v.as_str()) {
                if let Ok(url) = Url::parse(base_url) {
                    let path = url.path();
                    if !path.is_empty() && path != "/" {
                        mp["base_url_path_prefix"] = json!(path);
                    }
                    if !["http", "https"].contains(&url.scheme()) {
                        bail!("Please provide a valid URL with scheme (http/https) in base_url");
                    }
                    let protocol = url.scheme();
                    let port = url
                        .port()
                        .unwrap_or(if protocol == "http" { 80 } else { 443 });
                    let endpoint = url.host_str().unwrap_or("");
                    mp["endpoint"] = json!(endpoint);
                    mp["port"] = json!(port);
                    mp["protocol"] = json!(protocol);
                    let cluster_name = format!("{provider_str}_{endpoint}");
                    mp["cluster_name"] = json!(cluster_name);

                    if llms_with_endpoint_cluster_names.insert(cluster_name) {
                        llms_with_endpoint.push(mp.clone());
                    }
                }
            }
        }
    }

    // Auto-add internal model providers
    let overrides_config: serde_json::Value = config_yaml
        .get("overrides")
        .map(|v| serde_json::to_value(v).unwrap_or_default())
        .unwrap_or_else(|| json!({}));

    let model_name_set: HashSet<String> = updated_model_providers
        .iter()
        .filter_map(|mp| {
            mp.get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    // Auto-add arch-router
    let router_model = overrides_config
        .get("llm_routing_model")
        .and_then(|v| v.as_str())
        .unwrap_or("Arch-Router");
    let router_model_id = if router_model.contains('/') {
        router_model.split_once('/').unwrap().1
    } else {
        router_model
    };
    if !model_usage_name_keys.is_empty() && !model_name_set.contains(router_model_id) {
        updated_model_providers.push(json!({
            "name": "arch-router",
            "provider_interface": "plano",
            "model": router_model_id,
            "internal": true,
        }));
    }

    // Always add arch-function
    if !model_provider_name_set.contains("arch-function") {
        updated_model_providers.push(json!({
            "name": "arch-function",
            "provider_interface": "plano",
            "model": "Arch-Function",
            "internal": true,
        }));
    }

    // Auto-add plano-orchestrator
    let orch_model = overrides_config
        .get("agent_orchestration_model")
        .and_then(|v| v.as_str())
        .unwrap_or("Plano-Orchestrator");
    let orch_model_id = if orch_model.contains('/') {
        orch_model.split_once('/').unwrap().1
    } else {
        orch_model
    };
    if !model_name_set.contains(orch_model_id) {
        updated_model_providers.push(json!({
            "name": "plano/orchestrator",
            "provider_interface": "plano",
            "model": orch_model_id,
            "internal": true,
        }));
    }

    // Update config with processed model_providers
    let mp_yaml: serde_yaml::Value =
        serde_yaml::from_str(&serde_json::to_string(&updated_model_providers)?)?;
    config_yaml.as_mapping_mut().unwrap().insert(
        serde_yaml::Value::String("model_providers".to_string()),
        mp_yaml,
    );

    // Validate only one listener with model_providers
    let mut listeners_with_provider = 0;
    for listener in &listeners {
        if listener
            .get("model_providers")
            .and_then(|v| v.as_array())
            .is_some()
        {
            listeners_with_provider += 1;
            if listeners_with_provider > 1 {
                bail!("Please provide model_providers either under listeners or at root level, not both. Currently we don't support multiple listeners with model_providers");
            }
        }
    }

    // Validate input_filters reference valid agent/filter IDs
    for listener in &listeners {
        if let Some(filters) = listener.get("input_filters").and_then(|v| v.as_array()) {
            let listener_name = listener
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            for fc_id in filters {
                if let Some(id) = fc_id.as_str() {
                    if !agent_id_keys.contains(id) {
                        let available: Vec<_> = agent_id_keys.iter().cloned().collect();
                        let mut available_sorted = available;
                        available_sorted.sort();
                        bail!("Listener '{listener_name}' references input_filters id '{id}' which is not defined in agents or filters. Available ids: {}", available_sorted.join(", "));
                    }
                }
            }
        }
    }

    // Validate model aliases
    if let Some(aliases) = config_yaml.get("model_aliases") {
        if let Some(mapping) = aliases.as_mapping() {
            for (alias_key, alias_val) in mapping {
                let alias_name = alias_key.as_str().unwrap_or("");
                if let Some(target) = alias_val.get("target").and_then(|v| v.as_str()) {
                    if !model_name_keys.contains(target) {
                        let mut available: Vec<_> = model_name_keys.iter().cloned().collect();
                        available.sort();
                        bail!("Model alias 2 - '{alias_name}' targets '{target}' which is not defined as a model. Available models: {}", available.join(", "));
                    }
                }
            }
        }
    }

    // Generate rendered config strings
    let plano_config_string = serde_yaml::to_string(&config_yaml)?;

    // Handle agent orchestrator
    let use_agent_orchestrator = overrides_config
        .get("use_agent_orchestrator")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let agent_orchestrator = if use_agent_orchestrator {
        if endpoints.is_empty() {
            bail!("Please provide agent orchestrator in the endpoints section in your plano_config.yaml file");
        } else if endpoints.len() > 1 {
            bail!("Please provide single agent orchestrator in the endpoints section in your plano_config.yaml file");
        } else {
            Some(endpoints.keys().next().unwrap().clone())
        }
    } else {
        None
    };

    let upstream_connect_timeout = overrides_config
        .get("upstream_connect_timeout")
        .and_then(|v| v.as_str())
        .unwrap_or("5s");
    let upstream_tls_ca_path = overrides_config
        .get("upstream_tls_ca_path")
        .and_then(|v| v.as_str())
        .unwrap_or("/etc/ssl/certs/ca-certificates.crt");

    // Render template
    let template_filename = template_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("envoy.template.yaml");

    let mut tera = tera::Tera::default();
    let template_content = std::fs::read_to_string(template_path)?;
    // Convert Jinja2 syntax to Tera syntax
    // indent(N) → indent(width=N)
    let template_content = regex::Regex::new(r"indent\((\d+)\)")
        .unwrap()
        .replace_all(&template_content, "indent(width=$1)")
        .to_string();
    // var.split(":") | first → var | split(pat=":") | first
    let template_content = regex::Regex::new(r#"(\w+)\.split\("([^"]+)"\)"#)
        .unwrap()
        .replace_all(&template_content, r#"$1 | split(pat="$2")"#)
        .to_string();
    // default('value') → default(value='value')
    let template_content = regex::Regex::new(r"default\('([^']+)'\)")
        .unwrap()
        .replace_all(&template_content, "default(value='$1')")
        .to_string();
    // replace(" ", "_") → replace(from=" ", to="_")
    let template_content = regex::Regex::new(r#"replace\("([^"]*)",\s*"([^"]*)"\)"#)
        .unwrap()
        .replace_all(&template_content, r#"replace(from="$1", to="$2")"#)
        .to_string();
    // dict.items() → dict (Tera iterates dicts directly)
    let template_content = template_content.replace(".items()", "");
    tera.add_raw_template(template_filename, &template_content)?;

    let mut context = tera::Context::new();
    context.insert("prompt_gateway_listener", &prompt_gateway);
    context.insert("llm_gateway_listener", &llm_gateway);
    context.insert("plano_config", &plano_config_string);
    context.insert("plano_llm_config", &plano_config_string);
    context.insert("plano_clusters", &inferred_clusters);
    context.insert("plano_model_providers", &updated_model_providers);
    context.insert("plano_tracing", &plano_tracing);
    context.insert("local_llms", &llms_with_endpoint);
    context.insert("agent_orchestrator", &agent_orchestrator);
    context.insert("listeners", &listeners);
    context.insert("upstream_connect_timeout", upstream_connect_timeout);
    context.insert("upstream_tls_ca_path", upstream_tls_ca_path);

    let rendered = tera.render(template_filename, &context)?;

    // Write output files
    if let Some(parent) = envoy_output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(envoy_output_path, &rendered)?;

    if let Some(parent) = config_output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(config_output_path, &plano_config_string)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_endpoint_and_port_with_port() {
        let (host, port) = get_endpoint_and_port("example.com:8080", "http");
        assert_eq!(host, "example.com");
        assert_eq!(port, 8080);
    }

    #[test]
    fn test_get_endpoint_and_port_http() {
        let (host, port) = get_endpoint_and_port("example.com", "http");
        assert_eq!(host, "example.com");
        assert_eq!(port, 80);
    }

    #[test]
    fn test_get_endpoint_and_port_https() {
        let (host, port) = get_endpoint_and_port("example.com", "https");
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
    }
}
