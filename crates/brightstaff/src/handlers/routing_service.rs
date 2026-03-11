use bytes::Bytes;
use common::configuration::{ModelUsagePreference, SpanAttributes};
use common::consts::{REQUEST_ID_HEADER, TRACE_PARENT_HEADER};
use common::errors::BrightStaffError;
use hermesllm::clients::SupportedAPIsFromClient;
use hermesllm::ProviderRequestType;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode};
use std::sync::Arc;
use tracing::{debug, info, info_span, warn, Instrument};

use crate::handlers::policy_provider::PolicyProviderClient;
use crate::handlers::router_chat::router_chat_get_upstream_model;
use crate::router::llm_router::RouterService;
use crate::tracing::{collect_custom_trace_attributes, operation_component, set_service_name};

const ROUTING_POLICY_SIZE_WARNING_BYTES: usize = 5120;
type ExtractRoutingPolicyResult = (Bytes, Option<Vec<ModelUsagePreference>>, Option<String>);

/// Extracts `routing_policy` from a JSON body, returning the cleaned body bytes
/// and parsed preferences. The `routing_policy` field is removed from the JSON
/// before re-serializing so downstream parsers don't see the non-standard field.
///
/// If `warn_on_size` is true, logs a warning when the serialized policy exceeds 5KB.
pub fn extract_routing_policy(
    raw_bytes: &[u8],
    warn_on_size: bool,
) -> Result<ExtractRoutingPolicyResult, String> {
    let mut json_body: serde_json::Value = serde_json::from_slice(raw_bytes)
        .map_err(|err| format!("Failed to parse JSON: {}", err))?;

    let policy_id = json_body
        .as_object_mut()
        .and_then(|obj| obj.remove("policy_id"))
        .map(|policy_id_value| match policy_id_value {
            serde_json::Value::String(policy_id) if !policy_id.trim().is_empty() => Ok(policy_id),
            serde_json::Value::String(_) => Err("policy_id cannot be empty".to_string()),
            _ => Err("policy_id must be a string".to_string()),
        })
        .transpose()?;

    let preferences = json_body
        .as_object_mut()
        .and_then(|obj| obj.remove("routing_policy"))
        .and_then(|policy_value| {
            if warn_on_size {
                let policy_str = serde_json::to_string(&policy_value).unwrap_or_default();
                if policy_str.len() > ROUTING_POLICY_SIZE_WARNING_BYTES {
                    warn!(
                        size_bytes = policy_str.len(),
                        limit_bytes = ROUTING_POLICY_SIZE_WARNING_BYTES,
                        "routing_policy exceeds recommended size limit"
                    );
                }
            }
            match serde_json::from_value::<Vec<ModelUsagePreference>>(policy_value) {
                Ok(prefs) => {
                    info!(
                        num_models = prefs.len(),
                        "using inline routing_policy from request body"
                    );
                    Some(prefs)
                }
                Err(err) => {
                    warn!(error = %err, "failed to parse routing_policy");
                    None
                }
            }
        });

    let bytes = Bytes::from(serde_json::to_vec(&json_body).unwrap());
    Ok((bytes, preferences, policy_id))
}

#[derive(serde::Serialize)]
struct RoutingDecisionResponse {
    model: String,
    route: Option<String>,
    trace_id: String,
}

pub async fn routing_decision(
    request: Request<hyper::body::Incoming>,
    router_service: Arc<RouterService>,
    policy_provider: Option<Arc<PolicyProviderClient>>,
    request_path: String,
    span_attributes: Arc<Option<SpanAttributes>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_headers = request.headers().clone();
    let request_id: String = request_headers
        .get(REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let custom_attrs =
        collect_custom_trace_attributes(&request_headers, span_attributes.as_ref().as_ref());

    let request_span = info_span!(
        "routing_decision",
        component = "routing",
        request_id = %request_id,
        http.method = %request.method(),
        http.path = %request_path,
    );

    routing_decision_inner(
        request,
        router_service,
        policy_provider,
        request_id,
        request_path,
        request_headers,
        custom_attrs,
    )
    .instrument(request_span)
    .await
}

async fn routing_decision_inner(
    request: Request<hyper::body::Incoming>,
    router_service: Arc<RouterService>,
    policy_provider: Option<Arc<PolicyProviderClient>>,
    request_id: String,
    request_path: String,
    request_headers: hyper::HeaderMap,
    custom_attrs: std::collections::HashMap<String, String>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    set_service_name(operation_component::ROUTING);
    opentelemetry::trace::get_active_span(|span| {
        for (key, value) in &custom_attrs {
            span.set_attribute(opentelemetry::KeyValue::new(key.clone(), value.clone()));
        }
    });

    // Extract or generate traceparent
    let traceparent: String = match request_headers
        .get(TRACE_PARENT_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(tp) => tp,
        None => {
            let trace_id = uuid::Uuid::new_v4().to_string().replace("-", "");
            let generated_tp = format!("00-{}-0000000000000000-01", trace_id);
            warn!(
                generated_traceparent = %generated_tp,
                "TRACE_PARENT header missing, generated new traceparent"
            );
            generated_tp
        }
    };

    // Extract trace_id from traceparent (format: 00-{trace_id}-{span_id}-{flags})
    let trace_id = traceparent
        .split('-')
        .nth(1)
        .unwrap_or("unknown")
        .to_string();

    // Parse request body
    let raw_bytes = request.collect().await?.to_bytes();

    debug!(
        body = %String::from_utf8_lossy(&raw_bytes),
        "routing decision request body received"
    );

    // Extract routing_policy from request body before parsing as ProviderRequestType
    let (chat_request_bytes, inline_preferences, policy_id) =
        match extract_routing_policy(&raw_bytes, true) {
            Ok(result) => result,
            Err(err) => {
                warn!(error = %err, "failed to parse request JSON");
                return Ok(BrightStaffError::InvalidRequest(format!(
                    "Failed to parse request JSON: {}",
                    err
                ))
                .into_response());
            }
        };

    let client_request = match ProviderRequestType::try_from((
        &chat_request_bytes[..],
        &SupportedAPIsFromClient::from_endpoint(request_path.as_str()).unwrap(),
    )) {
        Ok(request) => request,
        Err(err) => {
            warn!(error = %err, "failed to parse request for routing decision");
            return Ok(BrightStaffError::InvalidRequest(format!(
                "Failed to parse request: {}",
                err
            ))
            .into_response());
        }
    };

    // Call the existing routing logic with inline preferences
    let routing_result = router_chat_get_upstream_model(
        router_service,
        client_request,
        &traceparent,
        &request_path,
        &request_id,
        inline_preferences,
        policy_id,
        policy_provider,
    )
    .await;

    match routing_result {
        Ok(result) => {
            let response = RoutingDecisionResponse {
                model: result.model_name,
                route: result.route_name,
                trace_id,
            };

            info!(
                model = %response.model,
                route = ?response.route,
                "routing decision completed"
            );

            let json = serde_json::to_string(&response).unwrap();
            let body = Full::new(Bytes::from(json))
                .map_err(|never| match never {})
                .boxed();

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(body)
                .unwrap())
        }
        Err(err) => {
            warn!(error = %err.message, "routing decision failed");
            Ok(BrightStaffError::ForwardedError {
                status_code: err.status_code,
                message: err.message,
            }
            .into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chat_body(extra_fields: &str) -> Vec<u8> {
        let extra = if extra_fields.is_empty() {
            String::new()
        } else {
            format!(", {}", extra_fields)
        };
        format!(
            r#"{{"model": "gpt-4o-mini", "messages": [{{"role": "user", "content": "hello"}}]{}}}"#,
            extra
        )
        .into_bytes()
    }

    #[test]
    fn extract_routing_policy_no_policy() {
        let body = make_chat_body("");
        let (cleaned, prefs, policy_id) = extract_routing_policy(&body, false).unwrap();

        assert!(prefs.is_none());
        assert!(policy_id.is_none());
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert_eq!(cleaned_json["model"], "gpt-4o-mini");
        assert!(cleaned_json.get("routing_policy").is_none());
    }

    #[test]
    fn extract_routing_policy_valid_policy() {
        let policy = r#""routing_policy": [
            {
                "model": "openai/gpt-4o",
                "routing_preferences": [
                    {"name": "coding", "description": "code generation tasks"}
                ]
            },
            {
                "model": "openai/gpt-4o-mini",
                "routing_preferences": [
                    {"name": "general", "description": "general questions"}
                ]
            }
        ]"#;
        let body = make_chat_body(policy);
        let (cleaned, prefs, policy_id) = extract_routing_policy(&body, false).unwrap();

        let prefs = prefs.expect("should have parsed preferences");
        assert_eq!(prefs.len(), 2);
        assert_eq!(prefs[0].model, "openai/gpt-4o");
        assert_eq!(prefs[0].routing_preferences[0].name, "coding");
        assert_eq!(prefs[1].model, "openai/gpt-4o-mini");
        assert_eq!(prefs[1].routing_preferences[0].name, "general");

        // routing_policy should be stripped from cleaned body
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert!(cleaned_json.get("routing_policy").is_none());
        assert!(policy_id.is_none());
        assert_eq!(cleaned_json["model"], "gpt-4o-mini");
    }

    #[test]
    fn extract_routing_policy_invalid_policy_returns_none() {
        // routing_policy is present but has wrong shape
        let policy = r#""routing_policy": "not-an-array""#;
        let body = make_chat_body(policy);
        let (cleaned, prefs, policy_id) = extract_routing_policy(&body, false).unwrap();

        // Invalid policy should be ignored (returns None), not error
        assert!(prefs.is_none());
        // routing_policy should still be stripped from cleaned body
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert!(cleaned_json.get("routing_policy").is_none());
        assert!(policy_id.is_none());
    }

    #[test]
    fn extract_routing_policy_invalid_json_returns_error() {
        let body = b"not valid json";
        let result = extract_routing_policy(body, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse JSON"));
    }

    #[test]
    fn extract_routing_policy_empty_array() {
        let policy = r#""routing_policy": []"#;
        let body = make_chat_body(policy);
        let (_, prefs, policy_id) = extract_routing_policy(&body, false).unwrap();

        let prefs = prefs.expect("empty array is valid");
        assert_eq!(prefs.len(), 0);
        assert!(policy_id.is_none());
    }

    #[test]
    fn extract_routing_policy_preserves_other_fields() {
        let policy = r#""routing_policy": [{"model": "gpt-4o", "routing_preferences": [{"name": "test", "description": "test"}]}], "temperature": 0.5, "max_tokens": 100"#;
        let body = make_chat_body(policy);
        let (cleaned, prefs, policy_id) = extract_routing_policy(&body, false).unwrap();

        assert!(prefs.is_some());
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert_eq!(cleaned_json["temperature"], 0.5);
        assert_eq!(cleaned_json["max_tokens"], 100);
        assert!(cleaned_json.get("routing_policy").is_none());
        assert!(policy_id.is_none());
    }

    #[test]
    fn extract_routing_policy_extracts_and_strips_policy_id() {
        let body = make_chat_body(r#""policy_id": "customer-abc-123""#);
        let (cleaned, prefs, policy_id) = extract_routing_policy(&body, false).unwrap();

        assert!(prefs.is_none());
        assert_eq!(policy_id, Some("customer-abc-123".to_string()));
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert!(cleaned_json.get("policy_id").is_none());
    }

    #[test]
    fn extract_routing_policy_rejects_non_string_policy_id() {
        let body = make_chat_body(r#""policy_id": 123"#);
        let result = extract_routing_policy(&body, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("policy_id must be a string"));
    }

    #[test]
    fn routing_decision_response_serialization() {
        let response = RoutingDecisionResponse {
            model: "openai/gpt-4o".to_string(),
            route: Some("code_generation".to_string()),
            trace_id: "abc123".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["model"], "openai/gpt-4o");
        assert_eq!(parsed["route"], "code_generation");
        assert_eq!(parsed["trace_id"], "abc123");
    }

    #[test]
    fn routing_decision_response_serialization_no_route() {
        let response = RoutingDecisionResponse {
            model: "none".to_string(),
            route: None,
            trace_id: "abc123".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["model"], "none");
        assert!(parsed["route"].is_null());
    }
}
