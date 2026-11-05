use bytes::Bytes;
use common::configuration::{
    EffectivePromptCaching, EffectiveRoutingBudget, ModelAlias, SpanAttributes,
    TopLevelRoutingPreference,
};
use common::consts::{MODEL_AFFINITY_HEADER, REQUEST_ID_HEADER};
use common::errors::BrightStaffError;
use hermesllm::clients::SupportedAPIsFromClient;
use hermesllm::{ProviderRequest, ProviderRequestType};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, info_span, warn, Instrument};

use super::extract_or_generate_traceparent;
use crate::affinity::prefix_hash_hex;
use crate::handlers::llm::model_selection::{router_chat_get_upstream_model, RoutingResult};
use crate::handlers::llm::resolve_model_alias;
use crate::handlers::llm::session_router;
use crate::handlers::routing_budget_request;
use crate::metrics as bs_metrics;
use crate::metrics::labels as metric_labels;
use crate::router::orchestrator::OrchestratorService;
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
    /// Whether the routing budget allowed a model switch this turn (`plano.switch.decision=allowed`).
    switched: bool,
    /// Prompt-prefix hash as 16 hex chars, the `x-plano-prefix-hash` format. Key a
    /// consistent-hash LB on it for KV-cache replica stickiness. Omitted when the
    /// request opted out (`X-Plano-Cache: off`) or has no anchorable prompt.
    #[serde(skip_serializing_if = "Option::is_none")]
    prefix_hash: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn routing_decision(
    request: Request<hyper::body::Incoming>,
    orchestrator_service: Arc<OrchestratorService>,
    request_path: String,
    span_attributes: &Option<SpanAttributes>,
    prompt_caching: EffectivePromptCaching,
    routing_budget: Option<EffectiveRoutingBudget>,
    route_on_user_only: bool,
    model_aliases: &Option<HashMap<String, ModelAlias>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_headers = request.headers().clone();
    let request_id: String = request_headers
        .get(REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let explicit_session_id: Option<String> = request_headers
        .get(MODEL_AFFINITY_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    let tenant_id: Option<String> = orchestrator_service
        .tenant_header()
        .and_then(|hdr| request_headers.get(hdr))
        .and_then(|v| v.to_str().ok())
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
        orchestrator_service,
        request_id,
        request_path,
        request_headers,
        custom_attrs,
        explicit_session_id,
        tenant_id,
        prompt_caching,
        routing_budget,
        route_on_user_only,
        model_aliases,
    )
    .instrument(request_span)
    .await
}

#[allow(clippy::too_many_arguments)]
async fn routing_decision_inner(
    request: Request<hyper::body::Incoming>,
    orchestrator_service: Arc<OrchestratorService>,
    request_id: String,
    request_path: String,
    request_headers: hyper::HeaderMap,
    custom_attrs: std::collections::HashMap<String, String>,
    explicit_session_id: Option<String>,
    tenant_id: Option<String>,
    prompt_caching: EffectivePromptCaching,
    routing_budget: Option<EffectiveRoutingBudget>,
    route_on_user_only: bool,
    model_aliases: &Option<HashMap<String, ModelAlias>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    set_service_name(operation_component::ROUTING);
    let routing_budget =
        routing_budget_request::routing_budget_for_request(routing_budget, &request_headers);
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

    // Parse the request body up front so a pin can be validated against prefix drift
    // and a fresh pin can be stored with its prefix hash. This endpoint shares the
    // session cache with the LLM handler, so both must key drift the same way.
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

    // `X-Plano-Cache: off` opts this request out of implicit affinity (same sentinel
    // the LLM handler honors), so callers can bypass stickiness per request.
    let cache_off_for_request = request_headers
        .get(common::consts::PLANO_CACHE_HEADER)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("off"));

    let request_messages = client_request.get_messages();
    let tool_names = client_request.get_tool_names();

    // Session key + prefix hash resolved identically to the LLM handler so pins
    // interoperate across the full-proxy and decision paths.
    // Derive the implicit session key when either prompt-caching affinity or the routing
    // budget is active, so the budget works the same way with caching off — and whenever
    // routing preferences are in play, so a later non-user turn can replay its decision.
    let routing_can_override_model =
        inline_routing_preferences.is_some() || orchestrator_service.has_routing_preferences();
    let implicit_affinity_enabled = prompt_caching.session_affinity
        || routing_budget.is_some()
        || (route_on_user_only && routing_can_override_model);
    let session_router::SessionResolution {
        request_prefix_hash,
        session_id,
    } = session_router::resolve_session(
        explicit_session_id,
        &request_messages,
        tool_names.as_deref(),
        tenant_id.as_deref(),
        implicit_affinity_enabled,
        cache_off_for_request,
    );

    // Resolve the alias before it is used as the session's model lane: the proxy path
    // stores bindings under the resolved model, so an unresolved alias here would look
    // like a different lane and refuse every replay.
    let requested_model = resolve_model_alias(client_request.model(), model_aliases);

    let context_tokens: u64 = if session_id.is_some() && routing_budget.is_some() {
        session_router::actual_context_tokens(&request_messages, &requested_model)
    } else {
        0
    };

    // Mirror the proxy path: opt-in (`routing.route_on_user_only`) skips the router
    // on non-user tails and replays the prior decision.
    let loop_reuse =
        if session_router::should_reuse_prior_decision(route_on_user_only, &request_messages) {
            session_router::reuse_prior_decision(
                &orchestrator_service,
                session_id.as_deref(),
                tenant_id.as_deref(),
                request_prefix_hash,
                &requested_model,
            )
            .await
        } else {
            None
        };

    let routing_result = match loop_reuse {
        Some(reuse) => {
            debug!(
                model = %reuse.model,
                "not a user turn — replaying the prior model instead of re-routing"
            );
            bs_metrics::record_routing_skip(metric_labels::ROUTING_SKIP_NOT_USER_TURN);
            Ok(RoutingResult {
                model_name: reuse.model.clone(),
                models: vec![reuse.model],
                route_name: reuse.route_name,
            })
        }
        None => {
            router_chat_get_upstream_model(
                Arc::clone(&orchestrator_service),
                client_request,
                &request_path,
                &request_id,
                inline_routing_preferences,
            )
            .await
        }
    };

    match routing_result {
        Ok(result) => {
            let candidate_model = result.model_name.clone();
            let decision = session_router::route(
                &orchestrator_service,
                routing_budget.as_ref(),
                session_router::RouteFacts {
                    session_id: session_id.as_deref(),
                    tenant_id: tenant_id.as_deref(),
                    prefix_hash: request_prefix_hash,
                    context_tokens,
                    candidate_model: &candidate_model,
                    candidate_route: result.route_name.as_deref(),
                    requested_model: &requested_model,
                },
            )
            .await;

            // Front the ranked fallback list with the decided model (the anchor, when a
            // switch was vetoed), so 429/5xx fallbacks still work.
            let mut models = result.models;
            if models.first() != Some(&decision.model) {
                models.retain(|m| m != &decision.model);
                models.insert(0, decision.model.clone());
            }

            let response = RoutingDecisionResponse {
                models,
                route: decision.route_name,
                trace_id,
                session_id,
                // `pinned` signals a warm, stuck session — safe for callers to treat as
                // "keep this provider's cache warm".
                pinned: decision.warm,
                switched: decision.switched,
                prefix_hash: request_prefix_hash.map(prefix_hash_hex),
            };

            // Distinguish "decision served" (a concrete model picked) from
            // "no_candidates" (the sentinel "none" returned when nothing
            // matched). The handler still responds 200 in both cases, so RED
            // metrics alone can't tell them apart.
            let outcome = if response.models.first().map(|m| m == "none").unwrap_or(true) {
                metric_labels::ROUTING_SVC_NO_CANDIDATES
            } else {
                metric_labels::ROUTING_SVC_DECISION_SERVED
            };
            bs_metrics::record_routing_service_outcome(outcome);

            info!(
                primary_model = %response.models.first().map(|s| s.as_str()).unwrap_or("none"),
                total_models = response.models.len(),
                route = ?response.route,
                pinned = response.pinned,
                switched = response.switched,
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
            bs_metrics::record_routing_service_outcome(metric_labels::ROUTING_SVC_POLICY_ERROR);
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
            switched: true,
            prefix_hash: Some(prefix_hash_hex(0xdead_beef)),
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["models"][0], "openai/gpt-4o-mini");
        assert_eq!(parsed["models"][1], "openai/gpt-4o");
        assert_eq!(parsed["route"], "code_generation");
        assert_eq!(parsed["trace_id"], "abc123");
        assert_eq!(parsed["session_id"], "sess-abc");
        assert_eq!(parsed["pinned"], true);
        assert_eq!(parsed["switched"], true);
        assert_eq!(parsed["prefix_hash"], "00000000deadbeef");
    }

    #[test]
    fn routing_decision_response_serialization_no_route() {
        let response = RoutingDecisionResponse {
            models: vec!["none".to_string()],
            route: None,
            trace_id: "abc123".to_string(),
            session_id: None,
            pinned: false,
            switched: false,
            prefix_hash: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["models"][0], "none");
        assert!(parsed["route"].is_null());
        assert!(parsed.get("session_id").is_none());
        assert!(parsed.get("prefix_hash").is_none());
        assert_eq!(parsed["pinned"], false);
        assert_eq!(parsed["switched"], false);
    }

    /// Decision-endpoint callers get the same prefix hash the proxy path stamps as
    /// `x-plano-prefix-hash`, so they can key replica stickiness on it; it is omitted
    /// when the request opts out with `X-Plano-Cache: off`.
    #[test]
    fn routing_decision_response_exposes_prefix_hash() {
        let body = br#"{"model":"gpt-4o-mini","messages":[{"role":"system","content":"You are a coding agent."},{"role":"user","content":"hi"}]}"#;
        let client_request = ProviderRequestType::try_from((
            &body[..],
            &SupportedAPIsFromClient::from_endpoint("/v1/chat/completions").unwrap(),
        ))
        .unwrap();
        let messages = client_request.get_messages();

        for (cache_off, expected) in [(false, true), (true, false)] {
            let resolution =
                session_router::resolve_session(None, &messages, None, None, false, cache_off);
            let response = RoutingDecisionResponse {
                models: vec!["openai/gpt-4o-mini".to_string()],
                route: None,
                trace_id: "abc123".to_string(),
                session_id: resolution.session_id,
                pinned: false,
                switched: false,
                prefix_hash: resolution.request_prefix_hash.map(prefix_hash_hex),
            };
            let parsed: serde_json::Value =
                serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap();
            if expected {
                let hash = parsed["prefix_hash"].as_str().unwrap();
                assert_eq!(hash.len(), 16);
                assert!(hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
                assert_eq!(
                    hash,
                    prefix_hash_hex(resolution.request_prefix_hash.unwrap())
                );
            } else {
                assert!(parsed.get("prefix_hash").is_none());
            }
        }
    }

    /// The lane a decision is stored under must be the resolved model, matching the
    /// proxy path. An unresolved alias reads as a different lane, so every
    /// `route_on_user_only` replay would be refused and each step would re-route.
    #[test]
    fn alias_is_resolved_before_becoming_the_session_lane() {
        let aliases = Some(HashMap::from([(
            "arch.codex.default".to_string(),
            ModelAlias {
                target: "openai/gpt-5.4".to_string(),
            },
        )]));

        let body = br#"{"model":"arch.codex.default","messages":[{"role":"user","content":"hi"}]}"#;
        let (cleaned, _) = extract_routing_policy(body).unwrap();
        let client_request = ProviderRequestType::try_from((
            &cleaned[..],
            &SupportedAPIsFromClient::from_endpoint("/v1/chat/completions").unwrap(),
        ))
        .unwrap();

        assert_eq!(client_request.model(), "arch.codex.default");
        assert_eq!(
            resolve_model_alias(client_request.model(), &aliases),
            "openai/gpt-5.4"
        );
    }
}
