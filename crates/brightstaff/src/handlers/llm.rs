use bytes::Bytes;
use common::configuration::{LlmProvider, ModelAlias};
use common::consts::{
    ARCH_IS_STREAMING_HEADER, ARCH_PROVIDER_HINT_HEADER, REQUEST_ID_HEADER, TRACE_PARENT_HEADER,
};
use common::traces::TraceCollector;
use hermesllm::apis::openai_responses::InputParam;
use hermesllm::clients::{SupportedAPIsFromClient, SupportedUpstreamAPIs};
use hermesllm::{ProviderRequest, ProviderRequestType};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::header::{self};
use hyper::{Request, Response, StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::handlers::router_chat::router_chat_get_upstream_model;
use crate::handlers::utils::{
    create_streaming_response, truncate_message, ObservableStreamProcessor,
};
use crate::router::llm_router::RouterService;
use crate::state::response_state_processor::ResponsesStateProcessor;
use crate::state::{
    extract_input_items, retrieve_and_combine_input, StateStorage, StateStorageError,
};
use crate::tracing::operation_component;

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

pub async fn llm_chat(
    request: Request<hyper::body::Incoming>,
    router_service: Arc<RouterService>,
    full_qualified_llm_provider_url: String,
    model_aliases: Arc<Option<HashMap<String, ModelAlias>>>,
    llm_providers: Arc<RwLock<Vec<LlmProvider>>>,
    trace_collector: Arc<TraceCollector>,
    state_storage: Option<Arc<dyn StateStorage>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_path = request.uri().path().to_string();
    let request_headers = request.headers().clone();
    let request_id = request_headers
        .get(REQUEST_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Extract or generate traceparent - this establishes the trace context for all spans
    let traceparent: String = request_headers
        .get(TRACE_PARENT_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            use uuid::Uuid;
            let trace_id = Uuid::new_v4().to_string().replace("-", "");
            format!("00-{}-0000000000000000-01", trace_id)
        });

    let mut request_headers = request_headers;
    let chat_request_bytes = request.collect().await?.to_bytes();

    debug!(
        "[PLANO_REQ_ID:{}] | REQUEST_BODY (UTF8): {}",
        request_id,
        String::from_utf8_lossy(&chat_request_bytes)
    );

    let mut client_request = match ProviderRequestType::try_from((
        &chat_request_bytes[..],
        &SupportedAPIsFromClient::from_endpoint(request_path.as_str()).unwrap(),
    )) {
        Ok(request) => request,
        Err(err) => {
            warn!(
                "[PLANO_REQ_ID:{}] | FAILURE | Failed to parse request as ProviderRequestType: {}",
                request_id, err
            );
            let err_msg = format!(
                "[PLANO_REQ_ID:{}] | FAILURE | Failed to parse request: {}",
                request_id, err
            );
            let mut bad_request = Response::new(full(err_msg));
            *bad_request.status_mut() = StatusCode::BAD_REQUEST;
            return Ok(bad_request);
        }
    };

    // === v1/responses state management: Extract input items early ===
    let mut original_input_items = Vec::new();
    let client_api = SupportedAPIsFromClient::from_endpoint(request_path.as_str());
    let is_responses_api_client = matches!(
        client_api,
        Some(SupportedAPIsFromClient::OpenAIResponsesAPI(_))
    );

    // Model alias resolution: update model field in client_request immediately
    // This ensures all downstream objects use the resolved model
    let model_from_request = client_request.model().to_string();
    let temperature = client_request.get_temperature();
    let is_streaming_request = client_request.is_streaming();
    let resolved_model = resolve_model_alias(&model_from_request, &model_aliases);

    // Extract tool names and user message preview for span attributes
    let tool_names = client_request.get_tool_names();
    let user_message_preview = client_request
        .get_recent_user_message()
        .map(|msg| truncate_message(&msg, 50));

    // Extract messages for signal analysis (clone before moving client_request)
    let messages_for_signals = client_request.get_messages();

    client_request.set_model(resolved_model.clone());
    if client_request.remove_metadata_key("archgw_preference_config") {
        debug!(
            "[PLANO_REQ_ID:{}] Removed archgw_preference_config from metadata",
            request_id
        );
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
                &resolved_model,
                &request_path,
                &resolved_model,
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
                            info!("[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Updated request with conversation history ({} items)", request_id, original_input_items.len());
                        }
                        Err(StateStorageError::NotFound(_)) => {
                            // Return 409 Conflict when previous_response_id not found
                            warn!("[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Previous response_id not found: {}", request_id, prev_resp_id);
                            let err_msg = format!(
                                "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Conversation state not found for previous_response_id: {}",
                                request_id, prev_resp_id
                            );
                            let mut conflict_response = Response::new(full(err_msg));
                            *conflict_response.status_mut() = StatusCode::CONFLICT;
                            return Ok(conflict_response);
                        }
                        Err(e) => {
                            // Log warning but continue on other storage errors
                            warn!(
                                "[PLANO_REQ_ID:{}] | STATE_PROCESSOR | Failed to retrieve conversation state for {}: {}",
                                request_id, prev_resp_id, e
                            );
                            // Restore original_input_items since we passed ownership
                            original_input_items = extract_input_items(&responses_req.input);
                        }
                    }
                }
            } else {
                debug!(
                    "[PLANO_REQ_ID:{}] | BRIGHT_STAFF | Upstream supports ResponsesAPI natively.",
                    request_id
                );
            }
        }
    }

    // Serialize request for upstream BEFORE router consumes it
    let client_request_bytes_for_upstream = ProviderRequestType::to_bytes(&client_request).unwrap();

    // Determine routing using the dedicated router_chat module
    let routing_result = match router_chat_get_upstream_model(
        router_service,
        client_request, // Pass the original request - router_chat will convert it
        &request_headers,
        trace_collector.clone(),
        &traceparent,
        &request_path,
    )
    .await
    {
        Ok(result) => result,
        Err(err) => {
            let mut internal_error = Response::new(full(err.message));
            *internal_error.status_mut() = err.status_code;
            return Ok(internal_error);
        }
    };

    let model_name = routing_result.model_name;

    debug!(
        "[PLANO_REQ_ID:{}] | ARCH_ROUTER URL | {}, Resolved Model: {}",
        request_id, full_qualified_llm_provider_url, model_name
    );

    request_headers.insert(
        ARCH_PROVIDER_HINT_HEADER,
        header::HeaderValue::from_str(&model_name).unwrap(),
    );

    request_headers.insert(
        header::HeaderName::from_static(ARCH_IS_STREAMING_HEADER),
        header::HeaderValue::from_str(&is_streaming_request.to_string()).unwrap(),
    );
    // remove content-length header if it exists
    request_headers.remove(header::CONTENT_LENGTH);

    // Capture start time right before sending request to upstream
    let request_start_time = std::time::Instant::now();
    let request_start_system_time = std::time::SystemTime::now();

    let llm_response = match reqwest::Client::new()
        .post(full_qualified_llm_provider_url)
        .headers(request_headers)
        .body(client_request_bytes_for_upstream)
        .send()
        .await
    {
        Ok(res) => res,
        Err(err) => {
            let err_msg = format!("Failed to send request: {}", err);
            let mut internal_error = Response::new(full(err_msg));
            *internal_error.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            return Ok(internal_error);
        }
    };

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

    // Build the LLM span (will be finalized after streaming completes)
    let llm_span = build_llm_span(
        &traceparent,
        &request_path,
        &resolved_model,
        &model_name,
        upstream_status.as_u16(),
        is_streaming_request,
        request_start_system_time,
        tool_names,
        user_message_preview,
        temperature,
        &llm_providers,
    )
    .await;

    // Create base processor for metrics and tracing
    let base_processor = ObservableStreamProcessor::new(
        trace_collector,
        operation_component::LLM,
        llm_span,
        request_start_time,
        Some(messages_for_signals),
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
            resolved_model.clone(),
            model_name.clone(),
            is_streaming_request,
            false, // Not OpenAI upstream since should_manage_state is true
            content_encoding,
            request_id.clone(),
        );
        create_streaming_response(byte_stream, state_processor, 16)
    } else {
        // Use base processor without state management
        create_streaming_response(byte_stream, base_processor, 16)
    };

    match response.body(streaming_response.body) {
        Ok(response) => Ok(response),
        Err(err) => {
            let err_msg = format!("Failed to create response: {}", err);
            let mut internal_error = Response::new(full(err_msg));
            *internal_error.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            Ok(internal_error)
        }
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

/// Builds the LLM span with all required and optional attributes.
#[allow(clippy::too_many_arguments)]
async fn build_llm_span(
    traceparent: &str,
    request_path: &str,
    resolved_model: &str,
    model_name: &str,
    status_code: u16,
    is_streaming: bool,
    start_time: std::time::SystemTime,
    tool_names: Option<Vec<String>>,
    user_message_preview: Option<String>,
    temperature: Option<f32>,
    llm_providers: &Arc<RwLock<Vec<LlmProvider>>>,
) -> common::traces::Span {
    use crate::tracing::{http, llm, OperationNameBuilder};
    use common::traces::{parse_traceparent, SpanBuilder, SpanKind};

    // Calculate the upstream path based on provider configuration
    let upstream_path = get_upstream_path(
        llm_providers,
        model_name,
        request_path,
        resolved_model,
        is_streaming,
    )
    .await;

    // Build operation name showing path transformation if different
    let operation_name = if request_path != upstream_path {
        OperationNameBuilder::new()
            .with_method("POST")
            .with_path(format!("{} >> {}", request_path, upstream_path))
            .with_target(resolved_model)
            .build()
    } else {
        OperationNameBuilder::new()
            .with_method("POST")
            .with_path(request_path)
            .with_target(resolved_model)
            .build()
    };

    let (trace_id, parent_span_id) = parse_traceparent(traceparent);

    let mut span_builder = SpanBuilder::new(&operation_name)
        .with_trace_id(&trace_id)
        .with_kind(SpanKind::Client)
        .with_start_time(start_time)
        .with_attribute(http::METHOD, "POST")
        .with_attribute(http::STATUS_CODE, status_code.to_string())
        .with_attribute(http::TARGET, request_path.to_string())
        .with_attribute(http::UPSTREAM_TARGET, upstream_path)
        .with_attribute(llm::MODEL_NAME, resolved_model.to_string())
        .with_attribute(llm::IS_STREAMING, is_streaming.to_string());

    // Only set parent span ID if it exists (not a root span)
    if let Some(parent) = parent_span_id {
        span_builder = span_builder.with_parent_span_id(&parent);
    }

    // Add optional attributes
    if let Some(temp) = temperature {
        span_builder = span_builder.with_attribute(llm::TEMPERATURE, temp.to_string());
    }

    if let Some(tools) = tool_names {
        let formatted_tools = tools
            .iter()
            .map(|name| format!("{}(...)", name))
            .collect::<Vec<_>>()
            .join("\n");
        span_builder = span_builder.with_attribute(llm::TOOLS, formatted_tools);
    }

    if let Some(preview) = user_message_preview {
        span_builder = span_builder.with_attribute(llm::USER_MESSAGE_PREVIEW, preview);
    }

    span_builder.build()
}

/// Calculates the upstream path for the provider based on the model name.
/// Looks up provider configuration, gets the ProviderId and base_url_path_prefix,
/// then uses target_endpoint_for_provider to calculate the correct upstream path.
async fn get_upstream_path(
    llm_providers: &Arc<RwLock<Vec<LlmProvider>>>,
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
    llm_providers: &Arc<RwLock<Vec<LlmProvider>>>,
    model_name: &str,
) -> (hermesllm::ProviderId, Option<String>) {
    let providers_lock = llm_providers.read().await;

    // First, try to find by model name or provider name
    let provider = providers_lock.iter().find(|p| {
        p.model.as_ref().map(|m| m == model_name).unwrap_or(false) || p.name == model_name
    });

    if let Some(provider) = provider {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        return (provider_id, prefix);
    }

    let default_provider = providers_lock.iter().find(|p| p.default.unwrap_or(false));

    if let Some(provider) = default_provider {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        (provider_id, prefix)
    } else {
        // Last resort: use OpenAI as hardcoded fallback
        warn!("No default provider found, falling back to OpenAI");
        (hermesllm::ProviderId::OpenAI, None)
    }
}
