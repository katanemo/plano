use bytes::Bytes;
use common::configuration::{ModelUsagePreference, SpanAttributes};
use common::consts::REQUEST_ID_HEADER;
use common::errors::BrightStaffError;
use hermesllm::clients::SupportedAPIsFromClient;
use hermesllm::ProviderRequestType;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode};
use std::sync::Arc;
use tracing::{debug, info, info_span, warn, Instrument};

use super::extract_or_generate_traceparent;
use crate::handlers::llm::model_selection::router_chat_get_upstream_model;
use crate::router::llm::RouterService;
use crate::router::policy_provider::PolicyProvider;
use crate::tracing::{collect_custom_trace_attributes, operation_component, set_service_name};

const ROUTING_POLICY_SIZE_WARNING_BYTES: usize = 5120;

/// Extracted routing metadata from a request body.
#[derive(Debug, Default)]
pub struct RoutingMetadata {
    /// Inline routing policy (highest priority).
    pub inline_policy: Option<Vec<ModelUsagePreference>>,
    /// Policy ID for external policy provider lookup.
    pub policy_id: Option<String>,
    /// Revision for revision-aware caching.
    pub revision: Option<u64>,
}

/// Extracts routing metadata from a JSON body, returning the cleaned body bytes
/// and parsed metadata.
///
/// Fields removed from the JSON before re-serializing:
/// - `routing_policy`: Inline routing preferences
/// - `policy_id`: External policy identifier
/// - `revision`: Policy revision for cache invalidation
///
/// If `warn_on_size` is true, logs a warning when the serialized policy exceeds 5KB.
pub fn extract_routing_metadata(
    raw_bytes: &[u8],
    warn_on_size: bool,
) -> Result<(Bytes, RoutingMetadata), String> {
    let mut json_body: serde_json::Value = serde_json::from_slice(raw_bytes)
        .map_err(|err| format!("Failed to parse JSON: {}", err))?;

    let mut metadata = RoutingMetadata::default();

    if let Some(obj) = json_body.as_object_mut() {
        // Extract inline routing_policy (highest priority)
        if let Some(policy_value) = obj.remove("routing_policy") {
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
                    metadata.inline_policy = Some(prefs);
                }
                Err(err) => {
                    warn!(error = %err, "failed to parse routing_policy");
                }
            }
        }

        // Extract policy_id for external policy provider
        if let Some(policy_id_value) = obj.remove("policy_id") {
            if let Some(policy_id) = policy_id_value.as_str() {
                debug!(policy_id = %policy_id, "extracted policy_id from request");
                metadata.policy_id = Some(policy_id.to_string());
            }
        }

        // Extract revision for revision-aware caching
        if let Some(revision_value) = obj.remove("revision") {
            if let Some(revision) = revision_value.as_u64() {
                debug!(revision = revision, "extracted revision from request");
                metadata.revision = Some(revision);
            }
        }
    }

    let bytes = Bytes::from(serde_json::to_vec(&json_body).unwrap());
    Ok((bytes, metadata))
}

/// Resolves routing preferences using the following priority:
/// 1. Inline `routing_policy` in request payload (highest priority)
/// 2. `policy_id` + `revision` → HTTP policy provider (with cache)
/// 3. None (fallback to default routing)
pub async fn resolve_routing_preferences(
    metadata: RoutingMetadata,
    policy_provider: Option<&PolicyProvider>,
) -> Option<Vec<ModelUsagePreference>> {
    // Priority 1: Inline policy
    if let Some(inline) = metadata.inline_policy {
        return Some(inline);
    }

    // Priority 2: External policy provider
    if let (Some(provider), Some(policy_id)) = (policy_provider, &metadata.policy_id) {
        match provider.get_policy(policy_id, metadata.revision).await {
            Ok(Some(policy)) => {
                info!(
                    policy_id = %policy_id,
                    num_models = policy.len(),
                    "using policy from external provider"
                );
                return Some(policy);
            }
            Ok(None) => {
                warn!(policy_id = %policy_id, "policy not found from external provider");
            }
            Err(err) => {
                warn!(error = %err, policy_id = %policy_id, "failed to fetch policy from external provider");
            }
        }
    }

    // Priority 3: No preferences (fallback to default)
    None
}

/// Backward-compatible function that only extracts inline routing_policy.
/// Deprecated: Use `extract_routing_metadata` instead.
#[deprecated(note = "Use extract_routing_metadata instead")]
pub fn extract_routing_policy(
    raw_bytes: &[u8],
    warn_on_size: bool,
) -> Result<(Bytes, Option<Vec<ModelUsagePreference>>), String> {
    let (bytes, metadata) = extract_routing_metadata(raw_bytes, warn_on_size)?;
    Ok((bytes, metadata.inline_policy))
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
    request_path: String,
    span_attributes: &Option<SpanAttributes>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_headers = request.headers().clone();
    let request_id: String = request_headers
        .get(REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let custom_attrs = collect_custom_trace_attributes(&request_headers, span_attributes.as_ref());

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

    let traceparent = extract_or_generate_traceparent(&request_headers);

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
    let (chat_request_bytes, inline_preferences) = match extract_routing_policy(&raw_bytes, true) {
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
            Ok(BrightStaffError::InternalServerError(err.message).into_response())
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
        let (cleaned, prefs) = extract_routing_policy(&body, false).unwrap();

        assert!(prefs.is_none());
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
        let (cleaned, prefs) = extract_routing_policy(&body, false).unwrap();

        let prefs = prefs.expect("should have parsed preferences");
        assert_eq!(prefs.len(), 2);
        assert_eq!(prefs[0].model, "openai/gpt-4o");
        assert_eq!(prefs[0].routing_preferences[0].name, "coding");
        assert_eq!(prefs[1].model, "openai/gpt-4o-mini");
        assert_eq!(prefs[1].routing_preferences[0].name, "general");

        // routing_policy should be stripped from cleaned body
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert!(cleaned_json.get("routing_policy").is_none());
        assert_eq!(cleaned_json["model"], "gpt-4o-mini");
    }

    #[test]
    fn extract_routing_policy_invalid_policy_returns_none() {
        // routing_policy is present but has wrong shape
        let policy = r#""routing_policy": "not-an-array""#;
        let body = make_chat_body(policy);
        let (cleaned, prefs) = extract_routing_policy(&body, false).unwrap();

        // Invalid policy should be ignored (returns None), not error
        assert!(prefs.is_none());
        // routing_policy should still be stripped from cleaned body
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert!(cleaned_json.get("routing_policy").is_none());
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
        let (_, prefs) = extract_routing_policy(&body, false).unwrap();

        let prefs = prefs.expect("empty array is valid");
        assert_eq!(prefs.len(), 0);
    }

    #[test]
    fn extract_routing_policy_preserves_other_fields() {
        let policy = r#""routing_policy": [{"model": "gpt-4o", "routing_preferences": [{"name": "test", "description": "test"}]}], "temperature": 0.5, "max_tokens": 100"#;
        let body = make_chat_body(policy);
        let (cleaned, prefs) = extract_routing_policy(&body, false).unwrap();

        assert!(prefs.is_some());
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert_eq!(cleaned_json["temperature"], 0.5);
        assert_eq!(cleaned_json["max_tokens"], 100);
        assert!(cleaned_json.get("routing_policy").is_none());
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
