use bytes::Bytes;
use common::configuration::{ModelAlias, SpanAttributes};
use common::consts::{
    ARCH_IS_STREAMING_HEADER, ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER, TRACE_PARENT_HEADER,
};
use common::llm_providers::LlmProviders;
use hermesllm::apis::openai_responses::InputParam;
use hermesllm::clients::{SupportedAPIsFromClient, SupportedUpstreamAPIs};
use hermesllm::{ProviderRequest, ProviderRequestType};
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::header::{self};
use hyper::{Request, Response};
use opentelemetry::global;
use opentelemetry::trace::get_active_span;
use opentelemetry_http::HeaderInjector;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, info_span, warn, Instrument};

use crate::handlers::router_chat::router_chat_get_upstream_model;
use crate::handlers::utils::{
    create_streaming_response, truncate_message, ObservableStreamProcessor,
};
use crate::router::llm_router::RouterService;
use crate::state::response_state_processor::ResponsesStateProcessor;
use crate::state::{
    extract_input_items, retrieve_and_combine_input, StateStorage, StateStorageError,
};
use crate::tracing::{
    collect_custom_trace_attributes, llm as tracing_llm, operation_component,
    routing as tracing_routing, set_service_name,
};

use common::errors::BrightStaffError;

fn strip_provider_prefix(model: &str) -> String {
    if let Some((_, model_name)) = model.split_once('/') {
        model_name.to_string()
    } else {
        model.to_string()
    }
}

fn is_retryable_upstream_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::TOO_MANY_REQUESTS
            | reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    )
}

async fn build_arbitrage_candidate_chain(
    llm_providers: &Arc<RwLock<LlmProviders>>,
    primary_model: &str,
) -> Vec<String> {
    let mut chain = Vec::new();
    let providers = llm_providers.read().await;

    if let Some(provider) = providers.get(primary_model) {
        if let Some(arbitrage_policy) = &provider.arbitrage_policy {
            if arbitrage_policy.enabled.unwrap_or(false) {
                for ranked_candidate in arbitrage_policy.rank.clone().unwrap_or_default() {
                    if !chain.contains(&ranked_candidate) {
                        chain.push(ranked_candidate);
                    }
                }
            }
        }
    }

    if !chain.contains(&primary_model.to_string()) {
        chain.push(primary_model.to_string());
    }

    chain
}

pub async fn llm_chat(
    request: Request<hyper::body::Incoming>,
    router_service: Arc<RouterService>,
    full_qualified_llm_provider_url: String,
    model_aliases: Arc<Option<HashMap<String, ModelAlias>>>,
    llm_providers: Arc<RwLock<LlmProviders>>,
    span_attributes: Arc<Option<SpanAttributes>>,
    state_storage: Option<Arc<dyn StateStorage>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_path = request.uri().path().to_string();
    let request_headers = request.headers().clone();
    let request_id: String = match request_headers
        .get(REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(id) => id,
        None => uuid::Uuid::new_v4().to_string(),
    };
    let custom_attrs =
        collect_custom_trace_attributes(&request_headers, span_attributes.as_ref().as_ref());

    // Create a span with request_id that will be included in all log lines
    let request_span = info_span!(
        "llm",
        component = "llm",
        request_id = %request_id,
        http.method = %request.method(),
        http.path = %request_path,
        llm.model = tracing::field::Empty,
        llm.tools = tracing::field::Empty,
        llm.user_message_preview = tracing::field::Empty,
        llm.temperature = tracing::field::Empty,
    );

    // Execute the rest of the handler inside the span
    llm_chat_inner(
        request,
        router_service,
        full_qualified_llm_provider_url,
        model_aliases,
        llm_providers,
        custom_attrs,
        state_storage,
        request_id,
        request_path,
        request_headers,
    )
    .instrument(request_span)
    .await
}

#[allow(clippy::too_many_arguments)]
async fn llm_chat_inner(
    request: Request<hyper::body::Incoming>,
    router_service: Arc<RouterService>,
    full_qualified_llm_provider_url: String,
    model_aliases: Arc<Option<HashMap<String, ModelAlias>>>,
    llm_providers: Arc<RwLock<LlmProviders>>,
    custom_attrs: HashMap<String, String>,
    state_storage: Option<Arc<dyn StateStorage>>,
    request_id: String,
    request_path: String,
    request_headers: hyper::HeaderMap,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    // Set service name for LLM operations
    set_service_name(operation_component::LLM);
    get_active_span(|span| {
        for (key, value) in &custom_attrs {
            span.set_attribute(opentelemetry::KeyValue::new(key.clone(), value.clone()));
        }
    });

    // Extract or generate traceparent - this establishes the trace context for all spans
    let traceparent: String = match request_headers
        .get(TRACE_PARENT_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    {
        Some(tp) => tp,
        None => {
            use uuid::Uuid;
            let trace_id = Uuid::new_v4().to_string().replace("-", "");
            let generated_tp = format!("00-{}-0000000000000000-01", trace_id);
            warn!(
                generated_traceparent = %generated_tp,
                "TRACE_PARENT header missing, generated new traceparent"
            );
            generated_tp
        }
    };

    let raw_bytes = request.collect().await?.to_bytes();

    debug!(
        body = %String::from_utf8_lossy(&raw_bytes),
        "request body received"
    );

    // Extract routing_policy from request body if present
    let (chat_request_bytes, inline_routing_policy) =
        match crate::handlers::routing_service::extract_routing_policy(&raw_bytes, false) {
            Ok(result) => result,
            Err(err) => {
                warn!(error = %err, "failed to parse request JSON");
                return Ok(BrightStaffError::InvalidRequest(format!(
                    "Failed to parse request: {}",
                    err
                ))
                .into_response());
            }
        };

    let mut client_request = match ProviderRequestType::try_from((
        &chat_request_bytes[..],
        &SupportedAPIsFromClient::from_endpoint(request_path.as_str()).unwrap(),
    )) {
        Ok(request) => request,
        Err(err) => {
            warn!(
                error = %err,
                "failed to parse request as ProviderRequestType"
            );
            return Ok(BrightStaffError::InvalidRequest(format!(
                "Failed to parse request: {}",
                err
            ))
            .into_response());
        }
    };

    // === v1/responses state management: Extract input items early ===
    let mut original_input_items = Vec::new();
    let client_api = SupportedAPIsFromClient::from_endpoint(request_path.as_str());
    let is_responses_api_client = matches!(
        client_api,
        Some(SupportedAPIsFromClient::OpenAIResponsesAPI(_))
    );

    // If model is not specified in the request, resolve from default provider
    let model_from_request = client_request.model().to_string();
    let model_from_request = if model_from_request.is_empty() {
        match llm_providers.read().await.default() {
            Some(default_provider) => {
                let default_model = default_provider.name.clone();
                info!(default_model = %default_model, "no model specified in request, using default provider");
                client_request.set_model(default_model.clone());
                default_model
            }
            None => {
                let err_msg = "No model specified in request and no default provider configured";
                warn!("{}", err_msg);
                return Ok(BrightStaffError::NoModelSpecified.into_response());
            }
        }
    } else {
        model_from_request
    };

    // Model alias resolution: update model field in client_request immediately
    // This ensures all downstream objects use the resolved model
    let temperature = client_request.get_temperature();
    let is_streaming_request = client_request.is_streaming();
    let alias_resolved_model = resolve_model_alias(&model_from_request, &model_aliases);
    let (provider_id, _) = get_provider_info(&llm_providers, &alias_resolved_model).await;

    // Validate that the requested model exists in configuration
    // This matches the validation in llm_gateway routing.rs
    if llm_providers
        .read()
        .await
        .get(&alias_resolved_model)
        .is_none()
    {
        warn!(model = %alias_resolved_model, "model not found in configured providers");
        return Ok(BrightStaffError::ModelNotFound(alias_resolved_model).into_response());
    }

    // Handle provider/model slug format (e.g., "openai/gpt-4")
    // Extract just the model name for upstream (providers don't understand the slug)
    let model_name_only = if let Some((_, model)) = alias_resolved_model.split_once('/') {
        model.to_string()
    } else {
        alias_resolved_model.clone()
    };

    // Extract tool names and user message preview for span attributes
    let tool_names = client_request.get_tool_names();
    let user_message_preview = client_request
        .get_recent_user_message()
        .map(|msg| truncate_message(&msg, 50));
    let span = tracing::Span::current();
    if let Some(temp) = temperature {
        span.record(tracing_llm::TEMPERATURE, tracing::field::display(temp));
    }
    if let Some(tools) = &tool_names {
        let formatted_tools = tools
            .iter()
            .map(|name| format!("{}(...)", name))
            .collect::<Vec<_>>()
            .join("\n");
        span.record(tracing_llm::TOOLS, formatted_tools.as_str());
    }
    if let Some(preview) = &user_message_preview {
        span.record(tracing_llm::USER_MESSAGE_PREVIEW, preview.as_str());
    }

    // Extract messages for signal analysis (clone before moving client_request)
    let messages_for_signals = Some(client_request.get_messages());

    // Set the model to just the model name (without provider prefix)
    // This ensures upstream receives "gpt-4" not "openai/gpt-4"
    client_request.set_model(model_name_only.clone());
    if client_request.remove_metadata_key("plano_preference_config") {
        debug!("removed plano_preference_config from metadata");
    }
    if let Some(ref client_api_kind) = client_api {
        let upstream_api =
            provider_id.compatible_api_for_client(client_api_kind, is_streaming_request);
        client_request.normalize_for_upstream(provider_id, &upstream_api);
    }
    // === v1/responses state management: Determine upstream API and combine input if needed ===
    // Do this BEFORE routing since routing consumes the request
    // Only process state if state_storage is configured
    let mut should_manage_state = false;
    if is_responses_api_client {
        if let (
            ProviderRequestType::ResponsesAPIRequest(ref mut responses_req),
            Some(ref state_store),
        ) = (&mut client_request, &state_storage)
        {
            // Extract original input once
            original_input_items = extract_input_items(&responses_req.input);

            // Get the upstream path and check if it's ResponsesAPI
            let upstream_path = get_upstream_path(
                &llm_providers,
                &alias_resolved_model,
                &request_path,
                &alias_resolved_model,
                is_streaming_request,
            )
            .await;

            let upstream_api = SupportedUpstreamAPIs::from_endpoint(&upstream_path);

            // Only manage state if upstream is NOT OpenAIResponsesAPI (needs translation)
            should_manage_state = !matches!(
                upstream_api,
                Some(SupportedUpstreamAPIs::OpenAIResponsesAPI(_))
            );

            if should_manage_state {
                // Retrieve and combine conversation history if previous_response_id exists
                if let Some(ref prev_resp_id) = responses_req.previous_response_id {
                    match retrieve_and_combine_input(
                        state_store.clone(),
                        prev_resp_id,
                        original_input_items, // Pass ownership instead of cloning
                    )
                    .await
                    {
                        Ok(combined_input) => {
                            // Update both the request and original_input_items
                            responses_req.input = InputParam::Items(combined_input.clone());
                            original_input_items = combined_input;
                            info!(
                                items = original_input_items.len(),
                                "updated request with conversation history"
                            );
                        }
                        Err(StateStorageError::NotFound(_)) => {
                            // Return 409 Conflict when previous_response_id not found
                            warn!(previous_response_id = %prev_resp_id, "previous response_id not found");
                            return Ok(BrightStaffError::ConversationStateNotFound(
                                prev_resp_id.to_string(),
                            )
                            .into_response());
                        }
                        Err(e) => {
                            // Log warning but continue on other storage errors
                            warn!(
                                previous_response_id = %prev_resp_id,
                                error = %e,
                                "failed to retrieve conversation state"
                            );
                            // Restore original_input_items since we passed ownership
                            original_input_items = extract_input_items(&responses_req.input);
                        }
                    }
                }
            } else {
                debug!("upstream supports ResponsesAPI natively");
            }
        }
    }

    // Determine routing using the dedicated router_chat module
    // This gets its own span for latency and error tracking
    let routing_span = info_span!(
        "routing",
        component = "routing",
        http.method = "POST",
        http.target = %request_path,
        model.requested = %model_from_request,
        model.alias_resolved = %alias_resolved_model,
        route.selected_model = tracing::field::Empty,
        routing.determination_ms = tracing::field::Empty,
    );
    let routing_result = match async {
        set_service_name(operation_component::ROUTING);
        router_chat_get_upstream_model(
            router_service,
            client_request, // Pass the original request - router_chat will convert it
            &traceparent,
            &request_path,
            &request_id,
            inline_routing_policy,
        )
        .await
    }
    .instrument(routing_span)
    .await
    {
        Ok(result) => result,
        Err(err) => {
            return Ok(BrightStaffError::ForwardedError {
                status_code: err.status_code,
                message: err.message,
            }
            .into_response());
        }
    };

    // Determine final model to use
    // Router returns "none" as a sentinel value when it doesn't select a specific model
    let router_selected_model = routing_result.model_name;
    let primary_model = if router_selected_model != "none" {
        // Router selected a specific model via routing preferences
        router_selected_model
    } else {
        // Router returned "none" sentinel, use validated resolved_model from request
        alias_resolved_model.clone()
    };
    let arbitrage_chain = build_arbitrage_candidate_chain(&llm_providers, &primary_model).await;
    let mut selected_model = primary_model.clone();
    let mut llm_response: Option<reqwest::Response> = None;
    let mut last_transport_error: Option<String> = None;
    let request_start_time = std::time::Instant::now();
    let http_client = reqwest::Client::new();

    for (attempt_idx, candidate_model) in arbitrage_chain.iter().enumerate() {
        selected_model = candidate_model.clone();
        let candidate_model_name = strip_provider_prefix(candidate_model);
        let mut candidate_request = match ProviderRequestType::try_from((
            &chat_request_bytes[..],
            &SupportedAPIsFromClient::from_endpoint(request_path.as_str()).unwrap(),
        )) {
            Ok(request) => request,
            Err(err) => {
                warn!(
                    candidate = %candidate_model,
                    error = %err,
                    "failed to build candidate request"
                );
                return Ok(BrightStaffError::InvalidRequest(format!(
                    "Failed to parse request: {}",
                    err
                ))
                .into_response());
            }
        };

        candidate_request.set_model(candidate_model_name.clone());
        if candidate_request.remove_metadata_key("plano_preference_config") {
            debug!("removed plano_preference_config from candidate metadata");
        }

        let (candidate_provider_id, _) = get_provider_info(&llm_providers, candidate_model).await;
        let selection_reason = if attempt_idx == 0 {
            if candidate_model == &primary_model {
                "router_selected_primary"
            } else {
                "free_tier_available"
            }
        } else {
            "fallback_on_retryable_error"
        };
        let is_fallback_attempt = attempt_idx > 0;
        get_active_span(|span| {
            span.set_attribute(opentelemetry::KeyValue::new(
                tracing_routing::SELECTION_REASON,
                selection_reason.to_string(),
            ));
            span.set_attribute(opentelemetry::KeyValue::new(
                tracing_routing::IS_FALLBACK,
                is_fallback_attempt,
            ));
            span.set_attribute(opentelemetry::KeyValue::new(
                tracing_llm::PROVIDER,
                candidate_provider_id.to_string(),
            ));
            span.set_attribute(opentelemetry::KeyValue::new(
                "routing.attempt_index",
                (attempt_idx + 1) as i64,
            ));
            span.set_attribute(opentelemetry::KeyValue::new(
                "routing.attempt_total",
                arbitrage_chain.len() as i64,
            ));
            span.set_attribute(opentelemetry::KeyValue::new(
                tracing_routing::UPSTREAM_ENDPOINT,
                candidate_model.clone(),
            ));
        });
        if let Some(ref client_api_kind) = client_api {
            let upstream_api = candidate_provider_id
                .compatible_api_for_client(client_api_kind, is_streaming_request);
            candidate_request.normalize_for_upstream(candidate_provider_id, &upstream_api);
        }
        let candidate_request_bytes = ProviderRequestType::to_bytes(&candidate_request).unwrap();

        let mut candidate_headers = request_headers.clone();
        candidate_headers.insert(
            ARCH_PROVIDER_HINT_HEADER,
            header::HeaderValue::from_str(candidate_model).unwrap(),
        );
        candidate_headers.insert(
            header::HeaderName::from_static(ARCH_IS_STREAMING_HEADER),
            header::HeaderValue::from_str(&is_streaming_request.to_string()).unwrap(),
        );
        candidate_headers.remove(header::CONTENT_LENGTH);
        global::get_text_map_propagator(|propagator| {
            let cx =
                tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
            propagator.inject_context(&cx, &mut HeaderInjector(&mut candidate_headers));
        });

        debug!(
            url = %full_qualified_llm_provider_url,
            provider_hint = %candidate_model,
            upstream_model = %candidate_model_name,
            selection_reason = %selection_reason,
            attempt_index = attempt_idx + 1,
            attempt_total = arbitrage_chain.len(),
            "Routing candidate to upstream"
        );

        let response = match http_client
            .post(&full_qualified_llm_provider_url)
            .headers(candidate_headers)
            .body(candidate_request_bytes)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                last_transport_error = Some(err.to_string());
                if attempt_idx + 1 < arbitrage_chain.len() {
                    let next_candidate = arbitrage_chain[attempt_idx + 1].as_str();
                    get_active_span(|span| {
                        span.set_attribute(opentelemetry::KeyValue::new(
                            "routing.fallback_trigger",
                            "transport_error".to_string(),
                        ));
                        span.set_attribute(opentelemetry::KeyValue::new(
                            "routing.next_candidate",
                            next_candidate.to_string(),
                        ));
                    });
                    warn!(
                        candidate = %candidate_model,
                        error = %err,
                        next_candidate = %next_candidate,
                        attempt_index = attempt_idx + 1,
                        attempt_total = arbitrage_chain.len(),
                        "candidate transport failure, trying next fallback"
                    );
                    continue;
                }
                return Ok(BrightStaffError::InternalServerError(format!(
                    "Failed to send request: {}",
                    err
                ))
                .into_response());
            }
        };

        let status = response.status();
        if is_retryable_upstream_status(status) && attempt_idx + 1 < arbitrage_chain.len() {
            let next_candidate = arbitrage_chain[attempt_idx + 1].as_str();
            get_active_span(|span| {
                span.set_attribute(opentelemetry::KeyValue::new(
                    "routing.fallback_trigger",
                    format!("http_{}", status.as_u16()),
                ));
                span.set_attribute(opentelemetry::KeyValue::new(
                    "routing.next_candidate",
                    next_candidate.to_string(),
                ));
            });
            warn!(
                candidate = %candidate_model,
                status = status.as_u16(),
                next_candidate = %next_candidate,
                attempt_index = attempt_idx + 1,
                attempt_total = arbitrage_chain.len(),
                "candidate returned retryable status, trying next fallback"
            );
            continue;
        }

        llm_response = Some(response);
        break;
    }

    let llm_response = match llm_response {
        Some(response) => response,
        None => {
            return Ok(BrightStaffError::InternalServerError(format!(
                "Failed to send request across arbitrage chain: {}",
                last_transport_error.unwrap_or_else(|| "unknown error".to_string())
            ))
            .into_response());
        }
    };

    tracing::Span::current().record(tracing_llm::MODEL_NAME, selected_model.as_str());
    let span_name = if model_from_request == selected_model {
        format!("POST {} {}", request_path, selected_model)
    } else {
        format!(
            "POST {} {} -> {}",
            request_path, model_from_request, selected_model
        )
    };
    get_active_span(|span| {
        span.update_name(span_name.clone());
    });

    // copy over the headers and status code from the original response
    let response_headers = llm_response.headers().clone();
    let upstream_status = llm_response.status();
    let mut response = Response::builder().status(upstream_status);
    let headers = response.headers_mut().unwrap();
    for (header_name, header_value) in response_headers.iter() {
        headers.insert(header_name, header_value.clone());
    }

    // Build LLM span with actual status code using constants
    let byte_stream = llm_response.bytes_stream();

    // Create base processor for metrics and tracing
    let base_processor = ObservableStreamProcessor::new(
        operation_component::LLM,
        span_name,
        request_start_time,
        messages_for_signals,
    );

    // === v1/responses state management: Wrap with ResponsesStateProcessor ===
    // Only wrap if we need to manage state (client is ResponsesAPI AND upstream is NOT ResponsesAPI AND state_storage is configured)
    let streaming_response = if let (true, false, Some(state_store)) = (
        should_manage_state,
        original_input_items.is_empty(),
        state_storage,
    ) {
        // Extract Content-Encoding header to handle decompression for state parsing
        let content_encoding = response_headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Wrap with state management processor to store state after response completes
        let state_processor = ResponsesStateProcessor::new(
            base_processor,
            state_store,
            original_input_items,
            alias_resolved_model.clone(),
            selected_model.clone(),
            is_streaming_request,
            false, // Not OpenAI upstream since should_manage_state is true
            content_encoding,
            request_id,
        );
        create_streaming_response(byte_stream, state_processor, 16)
    } else {
        // Use base processor without state management
        create_streaming_response(byte_stream, base_processor, 16)
    };

    match response.body(streaming_response.body) {
        Ok(response) => Ok(response),
        Err(err) => Ok(BrightStaffError::InternalServerError(format!(
            "Failed to create response: {}",
            err
        ))
        .into_response()),
    }
}
/// Resolves model aliases by looking up the requested model in the model_aliases map.
/// Returns the target model if an alias is found, otherwise returns the original model.
fn resolve_model_alias(
    model_from_request: &str,
    model_aliases: &Arc<Option<HashMap<String, ModelAlias>>>,
) -> String {
    if let Some(aliases) = model_aliases.as_ref() {
        if let Some(model_alias) = aliases.get(model_from_request) {
            debug!(
                "Model Alias: 'From {}' -> 'To {}'",
                model_from_request, model_alias.target
            );
            return model_alias.target.clone();
        }
    }
    model_from_request.to_string()
}

/// Calculates the upstream path for the provider based on the model name.
/// Looks up provider configuration, gets the ProviderId and base_url_path_prefix,
/// then uses target_endpoint_for_provider to calculate the correct upstream path.
async fn get_upstream_path(
    llm_providers: &Arc<RwLock<LlmProviders>>,
    model_name: &str,
    request_path: &str,
    resolved_model: &str,
    is_streaming: bool,
) -> String {
    let (provider_id, base_url_path_prefix) = get_provider_info(llm_providers, model_name).await;

    // Calculate the upstream path using the proper API
    let client_api = SupportedAPIsFromClient::from_endpoint(request_path)
        .expect("Should have valid API endpoint");

    client_api.target_endpoint_for_provider(
        &provider_id,
        request_path,
        resolved_model,
        is_streaming,
        base_url_path_prefix.as_deref(),
    )
}

/// Helper function to get provider info (ProviderId and base_url_path_prefix)
async fn get_provider_info(
    llm_providers: &Arc<RwLock<LlmProviders>>,
    model_name: &str,
) -> (hermesllm::ProviderId, Option<String>) {
    let providers_lock = llm_providers.read().await;

    // Try to find by model name or provider name using LlmProviders::get
    // This handles both "gpt-4" and "openai/gpt-4" formats
    if let Some(provider) = providers_lock.get(model_name) {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        return (provider_id, prefix);
    }

    // Fall back to default provider
    if let Some(provider) = providers_lock.default() {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        (provider_id, prefix)
    } else {
        // Last resort: use OpenAI as hardcoded fallback
        warn!("No default provider found, falling back to OpenAI");
        (hermesllm::ProviderId::OpenAI, None)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_arbitrage_candidate_chain, is_retryable_upstream_status, strip_provider_prefix,
    };
    use common::configuration::{
        ArbitrageFailurePolicy, ArbitragePolicy, LlmProvider, LlmProviderType,
    };
    use common::llm_providers::LlmProviders;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn provider(name: &str, model: &str, default: bool) -> LlmProvider {
        LlmProvider {
            name: name.to_string(),
            model: Some(model.to_string()),
            provider_interface: LlmProviderType::OpenAI,
            default: Some(default),
            stream: None,
            access_key: None,
            endpoint: None,
            port: None,
            rate_limits: None,
            usage: None,
            routing_preferences: None,
            cluster_name: None,
            base_url_path_prefix: None,
            internal: None,
            passthrough_auth: None,
            arbitrage_policy: None,
        }
    }

    #[test]
    fn strips_provider_prefix() {
        assert_eq!(strip_provider_prefix("openai/gpt-4o-mini"), "gpt-4o-mini");
        assert_eq!(strip_provider_prefix("gpt-4o-mini"), "gpt-4o-mini");
    }

    #[test]
    fn retryable_status_matrix_is_deterministic() {
        assert!(is_retryable_upstream_status(
            reqwest::StatusCode::TOO_MANY_REQUESTS
        ));
        assert!(is_retryable_upstream_status(
            reqwest::StatusCode::BAD_GATEWAY
        ));
        assert!(is_retryable_upstream_status(
            reqwest::StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(is_retryable_upstream_status(
            reqwest::StatusCode::GATEWAY_TIMEOUT
        ));
        assert!(!is_retryable_upstream_status(
            reqwest::StatusCode::BAD_REQUEST
        ));
        assert!(!is_retryable_upstream_status(
            reqwest::StatusCode::UNAUTHORIZED
        ));
    }

    #[tokio::test]
    async fn arbitrage_chain_is_ranked_then_primary() {
        let mut primary = provider("openai/gpt-4o-mini", "gpt-4o-mini", true);
        primary.arbitrage_policy = Some(ArbitragePolicy {
            enabled: Some(true),
            rank: Some(vec![
                "groq/llama-3.1-8b-instant".to_string(),
                "together_ai/openai/gpt-oss-20b".to_string(),
            ]),
            on_failure: Some(ArbitrageFailurePolicy {
                fallback_to_primary: Some(true),
            }),
        });

        let providers = vec![
            primary,
            provider("groq/llama-3.1-8b-instant", "llama-3.1-8b-instant", false),
            provider(
                "together_ai/openai/gpt-oss-20b",
                "openai/gpt-oss-20b",
                false,
            ),
        ];
        let llm_providers = Arc::new(RwLock::new(LlmProviders::try_from(providers).unwrap()));

        let chain = build_arbitrage_candidate_chain(&llm_providers, "openai/gpt-4o-mini").await;
        assert_eq!(
            chain,
            vec![
                "groq/llama-3.1-8b-instant".to_string(),
                "together_ai/openai/gpt-oss-20b".to_string(),
                "openai/gpt-4o-mini".to_string(),
            ]
        );
    }
}
