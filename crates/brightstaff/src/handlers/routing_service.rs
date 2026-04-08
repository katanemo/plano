use bytes::Bytes;
use common::configuration::{SpanAttributes, TopLevelRoutingPreference};
use common::consts::{REQUEST_ID_HEADER, ROUTING_SESSION_ID_HEADER};
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
use crate::tracing::{collect_custom_trace_attributes, operation_component, set_service_name};

/// Extracts `routing_preferences` from a JSON body, returning the cleaned body bytes
/// and the parsed preferences. The field is removed from the JSON before re-serializing
/// so downstream parsers don't see it.
pub fn extract_routing_policy(
    raw_bytes: &[u8],
) -> Result<(Bytes, Option<Vec<TopLevelRoutingPreference>>), String> {
    let mut json_body: serde_json::Value = serde_json::from_slice(raw_bytes)
        .map_err(|err| format!("Failed to parse JSON: {}", err))?;

    let routing_preferences = json_body
        .as_object_mut()
        .and_then(|o| o.remove("routing_preferences"))
        .and_then(
            |value| match serde_json::from_value::<Vec<TopLevelRoutingPreference>>(value) {
                Ok(prefs) => {
                    info!(
                        num_routes = prefs.len(),
                        "using inline routing_preferences from request body"
                    );
                    Some(prefs)
                }
                Err(err) => {
                    warn!(error = %err, "failed to parse routing_preferences");
                    None
                }
            },
        );

    let bytes = Bytes::from(serde_json::to_vec(&json_body).unwrap());
    Ok((bytes, routing_preferences))
}

#[derive(serde::Serialize)]
struct RoutingDecisionResponse {
    /// Ranked model list — use first, fall back to next on 429/5xx.
    models: Vec<String>,
    route: Option<String>,
    trace_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    pinned: bool,
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

    let session_id: Option<String> = request_headers
        .get(ROUTING_SESSION_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

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
        session_id,
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
    session_id: Option<String>,
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

    // Session pinning: check cache before doing any routing work
    if let Some(ref sid) = session_id {
        if let Some(cached) = router_service.get_cached_route(sid).await {
            info!(
                session_id = %sid,
                model = %cached.model_name,
                route = ?cached.route_name,
                "returning pinned routing decision from cache"
            );
            let response = RoutingDecisionResponse {
                models: vec![cached.model_name],
                route: cached.route_name,
                trace_id,
                session_id: Some(sid.clone()),
                pinned: true,
            };
            let json = serde_json::to_string(&response).unwrap();
            let body = Full::new(Bytes::from(json))
                .map_err(|never| match never {})
                .boxed();
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(body)
                .unwrap());
        }
    }

    // Parse request body
    let raw_bytes = request.collect().await?.to_bytes();

    debug!(
        body = %String::from_utf8_lossy(&raw_bytes),
        "routing decision request body received"
    );

    // Extract routing_preferences from body before parsing as ProviderRequestType
    let (chat_request_bytes, inline_routing_preferences) = match extract_routing_policy(&raw_bytes)
    {
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

    let routing_result = router_chat_get_upstream_model(
        Arc::clone(&router_service),
        client_request,
        &traceparent,
        &request_path,
        &request_id,
        inline_routing_preferences,
    )
    .await;

    match routing_result {
        Ok(result) => {
            // Cache the result if session_id is present
            if let Some(ref sid) = session_id {
                router_service
                    .cache_route(
                        sid.clone(),
                        result.model_name.clone(),
                        result.route_name.clone(),
                    )
                    .await;
            }

            let response = RoutingDecisionResponse {
                models: result.models,
                route: result.route_name,
                trace_id,
                session_id,
                pinned: false,
            };

            info!(
                primary_model = %response.models.first().map(|s| s.as_str()).unwrap_or("none"),
                total_models = response.models.len(),
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
    use common::configuration::SelectionPreference;

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
        let (cleaned, prefs) = extract_routing_policy(&body).unwrap();

        assert!(prefs.is_none());
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert_eq!(cleaned_json["model"], "gpt-4o-mini");
    }

    #[test]
    fn extract_routing_policy_invalid_json_returns_error() {
        let body = b"not valid json";
        let result = extract_routing_policy(body);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse JSON"));
    }

    #[test]
    fn extract_routing_policy_routing_preferences() {
        let policy = r#""routing_preferences": [
            {
                "name": "code generation",
                "description": "generate new code",
                "models": ["openai/gpt-4o", "openai/gpt-4o-mini"],
                "selection_policy": {"prefer": "fastest"}
            }
        ]"#;
        let body = make_chat_body(policy);
        let (cleaned, prefs) = extract_routing_policy(&body).unwrap();

        let prefs = prefs.expect("should have parsed routing_preferences");
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].name, "code generation");
        assert_eq!(prefs[0].models, vec!["openai/gpt-4o", "openai/gpt-4o-mini"]);

        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert!(cleaned_json.get("routing_preferences").is_none());
    }

    #[test]
    fn extract_routing_policy_preserves_other_fields() {
        let policy = r#""routing_preferences": [{"name": "test", "description": "test", "models": ["gpt-4o"], "selection_policy": {"prefer": "none"}}], "temperature": 0.5, "max_tokens": 100"#;
        let body = make_chat_body(policy);
        let (cleaned, prefs) = extract_routing_policy(&body).unwrap();

        assert!(prefs.is_some());
        let cleaned_json: serde_json::Value = serde_json::from_slice(&cleaned).unwrap();
        assert_eq!(cleaned_json["temperature"], 0.5);
        assert_eq!(cleaned_json["max_tokens"], 100);
        assert!(cleaned_json.get("routing_preferences").is_none());
    }

    #[test]
    fn extract_routing_policy_prefer_null_defaults_to_none() {
        let policy = r#""routing_preferences": [
            {
                "name": "coding",
                "description": "code generation, writing functions, debugging",
                "models": ["openai/gpt-4o", "openai/gpt-4o-mini"],
                "selection_policy": {"prefer": null}
            }
        ]"#;
        let body = make_chat_body(policy);
        let (_cleaned, prefs) = extract_routing_policy(&body).unwrap();

        let prefs = prefs.expect("should parse routing_preferences when prefer is null");
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].selection_policy.prefer, SelectionPreference::None);
    }

    #[test]
    fn extract_routing_policy_selection_policy_missing_defaults_to_none() {
        let policy = r#""routing_preferences": [
            {
                "name": "coding",
                "description": "code generation, writing functions, debugging",
                "models": ["openai/gpt-4o", "openai/gpt-4o-mini"]
            }
        ]"#;
        let body = make_chat_body(policy);
        let (_cleaned, prefs) = extract_routing_policy(&body).unwrap();

        let prefs =
            prefs.expect("should parse routing_preferences when selection_policy is missing");
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].selection_policy.prefer, SelectionPreference::None);
    }

    #[test]
    fn extract_routing_policy_prefer_empty_string_defaults_to_none() {
        let policy = r#""routing_preferences": [
            {
                "name": "coding",
                "description": "code generation, writing functions, debugging",
                "models": ["openai/gpt-4o", "openai/gpt-4o-mini"],
                "selection_policy": {"prefer": ""}
            }
        ]"#;
        let body = make_chat_body(policy);
        let (_cleaned, prefs) = extract_routing_policy(&body).unwrap();

        let prefs =
            prefs.expect("should parse routing_preferences when selection_policy.prefer is empty");
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].selection_policy.prefer, SelectionPreference::None);
    }

    #[test]
    fn routing_decision_response_serialization() {
        let response = RoutingDecisionResponse {
            models: vec![
                "openai/gpt-4o-mini".to_string(),
                "openai/gpt-4o".to_string(),
            ],
            route: Some("code_generation".to_string()),
            trace_id: "abc123".to_string(),
            session_id: Some("sess-abc".to_string()),
            pinned: true,
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["models"][0], "openai/gpt-4o-mini");
        assert_eq!(parsed["models"][1], "openai/gpt-4o");
        assert_eq!(parsed["route"], "code_generation");
        assert_eq!(parsed["trace_id"], "abc123");
        assert_eq!(parsed["session_id"], "sess-abc");
        assert_eq!(parsed["pinned"], true);
    }

    #[test]
    fn routing_decision_response_serialization_no_route() {
        let response = RoutingDecisionResponse {
            models: vec!["none".to_string()],
            route: None,
            trace_id: "abc123".to_string(),
            session_id: None,
            pinned: false,
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["models"][0], "none");
        assert!(parsed["route"].is_null());
        assert!(parsed.get("session_id").is_none());
        assert_eq!(parsed["pinned"], false);
    }
}
