use common::configuration::{HttpMethod, Parameter};
use std::collections::HashMap;

use serde_json::Value as JsonValue;
use serde_yaml::Value;

// only add params that are of string, number, bool, or sequence type
pub fn filter_tool_params(tool_params: &Option<HashMap<String, Value>>) -> HashMap<String, String> {
    if tool_params.is_none() {
        return HashMap::new();
    }
    tool_params
        .as_ref()
        .unwrap()
        .iter()
        .filter(|(_, value)| {
            value.is_number() || value.is_string() || value.is_bool() || value.is_sequence()
        })
        .filter_map(|(key, value)| match value {
            Value::Number(n) => Some((key.clone(), n.to_string())),
            Value::String(s) => Some((key.clone(), s.clone())),
            Value::Bool(b) => Some((key.clone(), b.to_string())),
            Value::Sequence(seq) => {
                // Convert sequence to comma-separated string for URL params / GET requests
                let items: Vec<String> = seq
                    .iter()
                    .filter_map(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        Value::Bool(b) => Some(b.to_string()),
                        _ => None,
                    })
                    .collect();
                Some((key.clone(), items.join(",")))
            }
            _ => None,
        })
        .collect::<HashMap<String, String>>()
}

pub fn compute_request_path_body(
    endpoint_path: &str,
    tool_params: &Option<HashMap<String, Value>>,
    prompt_target_params: &[Parameter],
    http_method: &HttpMethod,
) -> Result<(String, Option<String>), String> {
    let tool_url_params = filter_tool_params(tool_params);
    let (path_with_params, query_string, additional_params) = common::path::replace_params_in_path(
        endpoint_path,
        &tool_url_params,
        prompt_target_params,
    )?;

    let (path, body) = match http_method {
        HttpMethod::Get => (format!("{}?{}", path_with_params, query_string), None),
        HttpMethod::Post => {
            // Build the POST body as a JSON object, preserving list params as JSON arrays
            let mut body_params: HashMap<String, JsonValue> = additional_params
                .into_iter()
                .map(|(k, v)| (k, JsonValue::String(v)))
                .collect();

            // Override list (sequence) params with proper JSON arrays, replacing the
            // comma-separated string that filter_tool_params produced for path handling
            if let Some(params) = tool_params {
                for (key, value) in params.iter() {
                    if let Value::Sequence(seq) = value {
                        let json_arr: Vec<JsonValue> = seq
                            .iter()
                            .filter_map(|v| serde_json::to_value(v).ok())
                            .collect();
                        body_params.insert(key.clone(), JsonValue::Array(json_arr));
                    }
                }
            }

            if !query_string.is_empty() {
                query_string.split('&').for_each(|param| {
                    let mut parts = param.split('=');
                    let key = parts.next().unwrap();
                    let value = parts.next().unwrap_or("");
                    body_params
                        .entry(key.to_string())
                        .or_insert_with(|| JsonValue::String(value.to_string()));
                });
            }
            let body = serde_json::to_string(&body_params).unwrap();
            (path_with_params, Some(body))
        }
    };

    Ok((path, body))
}

#[cfg(test)]
mod test {
    use common::configuration::{HttpMethod, Parameter};

    #[test]
    fn test_compute_request_path_body() {
        let endpoint_path = "/cluster.open-cluster-management.io/v1/managedclusters/{cluster_name}";
        let tool_params = serde_yaml::from_str(
            r#"
      cluster_name: test1
      hello: hello world
      "#,
        )
        .unwrap();
        let prompt_target_params = vec![Parameter {
            name: "country".to_string(),
            parameter_type: None,
            description: "test target".to_string(),
            required: None,
            enum_values: None,
            default: Some("US".to_string()),
            in_path: None,
            format: None,
        }];
        let http_method = HttpMethod::Get;
        let (path, body) = super::compute_request_path_body(
            endpoint_path,
            &tool_params,
            &prompt_target_params,
            &http_method,
        )
        .unwrap();
        assert_eq!(
            path,
            "/cluster.open-cluster-management.io/v1/managedclusters/test1?hello=hello%20world&country=US"
        );
        assert_eq!(body, None);
    }

    #[test]
    fn test_compute_request_path_body_empty_params() {
        let endpoint_path = "/cluster.open-cluster-management.io/v1/managedclusters/";
        let tool_params = serde_yaml::from_str(r#"{}"#).unwrap();
        let prompt_target_params = vec![Parameter {
            name: "country".to_string(),
            parameter_type: None,
            description: "test target".to_string(),
            required: None,
            enum_values: None,
            default: Some("US".to_string()),
            in_path: None,
            format: None,
        }];
        let http_method = HttpMethod::Get;
        let (path, body) = super::compute_request_path_body(
            endpoint_path,
            &tool_params,
            &prompt_target_params,
            &http_method,
        )
        .unwrap();
        assert_eq!(
            path,
            "/cluster.open-cluster-management.io/v1/managedclusters/?country=US"
        );
        assert_eq!(body, None);
    }

    #[test]
    fn test_compute_request_path_body_override_default_val() {
        let endpoint_path = "/cluster.open-cluster-management.io/v1/managedclusters/";
        let tool_params = serde_yaml::from_str(
            r#"
      country: UK
      "#,
        )
        .unwrap();
        let prompt_target_params = vec![Parameter {
            name: "country".to_string(),
            parameter_type: None,
            description: "test target".to_string(),
            required: None,
            enum_values: None,
            default: Some("US".to_string()),
            in_path: None,
            format: None,
        }];
        let http_method = HttpMethod::Get;
        let (path, body) = super::compute_request_path_body(
            endpoint_path,
            &tool_params,
            &prompt_target_params,
            &http_method,
        )
        .unwrap();
        assert_eq!(
            path,
            "/cluster.open-cluster-management.io/v1/managedclusters/?country=UK"
        );
        assert_eq!(body, None);
    }

    #[test]
    fn test_filter_tool_params_list() {
        use super::filter_tool_params;
        let tool_params = serde_yaml::from_str(
            r#"
      device_ids:
        - device1
        - device2
        - device3
      name: test
      "#,
        )
        .unwrap();
        let params = filter_tool_params(&tool_params);
        assert_eq!(params.get("device_ids").unwrap(), "device1,device2,device3");
        assert_eq!(params.get("name").unwrap(), "test");
    }

    #[test]
    fn test_compute_request_path_body_list_post() {
        let endpoint_path = "/agent/device_reboot";
        let tool_params: Option<std::collections::HashMap<String, serde_yaml::Value>> =
            serde_yaml::from_str(
                r#"
      device_ids:
        - device1
        - device2
      "#,
            )
            .unwrap();
        let prompt_target_params = vec![];
        let http_method = HttpMethod::Post;
        let (path, body) = super::compute_request_path_body(
            endpoint_path,
            &tool_params,
            &prompt_target_params,
            &http_method,
        )
        .unwrap();
        assert_eq!(path, "/agent/device_reboot");
        let body_str = body.unwrap();
        let body_json: serde_json::Value = serde_json::from_str(&body_str).unwrap();
        assert!(body_json["device_ids"].is_array());
        assert_eq!(body_json["device_ids"][0], "device1");
        assert_eq!(body_json["device_ids"][1], "device2");
    }
}
