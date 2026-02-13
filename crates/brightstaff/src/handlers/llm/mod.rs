use bytes::Bytes;
use common::configuration::ModelAlias;
use common::consts::{ARCH_IS_STREAMING_HEADER, ARCH_PROVIDER_HINT_HEADER, TRACE_PARENT_HEADER};
use common::llm_providers::LlmProviders;
use hermesllm::apis::openai::Message;
use hermesllm::apis::openai_responses::InputParam;
use hermesllm::clients::{SupportedAPIsFromClient, SupportedUpstreamAPIs};
use hermesllm::{ProviderRequest, ProviderRequestType};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::header::{self};
use hyper::{Request, Response, StatusCode};
use opentelemetry::global;
use opentelemetry::trace::get_active_span;
use opentelemetry_http::HeaderInjector;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, info_span, warn, Instrument};

mod router;

use crate::app_state::AppState;
use crate::handlers::request::extract_request_id;
use crate::handlers::utils::{
    create_streaming_response, truncate_message, ObservableStreamProcessor,
};
use crate::state::response_state_processor::ResponsesStateProcessor;
use crate::state::{
    extract_input_items, retrieve_and_combine_input, StateStorage, StateStorageError,
};
use crate::tracing::{llm as tracing_llm, operation_component, set_service_name};
use router::router_chat_get_upstream_model;

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

pub async fn llm_chat(
    request: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_path = request.uri().path().to_string();
    let request_headers = request.headers().clone();
    let request_id = extract_request_id(&request);

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
    llm_chat_inner(request, state, request_id, request_path, request_headers)
        .instrument(request_span)
        .await
}

async fn llm_chat_inner(
    request: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
    request_id: String,
    request_path: String,
    mut request_headers: hyper::HeaderMap,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    // Set service name for LLM operations
    set_service_name(operation_component::LLM);

    let traceparent = extract_or_generate_traceparent(&request_headers);

    let full_qualified_llm_provider_url = format!("{}{}", state.llm_provider_url, request_path);

    // --- Phase 1: Parse and validate the incoming request ---
    let parsed = match parse_and_validate_request(
        request,
        &request_path,
        &state.model_aliases,
        &state.llm_providers,
    )
    .await
    {
        Ok(p) => p,
        Err(response) => return Ok(response),
    };

    let PreparedRequest {
        mut client_request,
        model_from_request,
        alias_resolved_model,
        model_name_only,
        is_streaming_request,
        is_responses_api_client,
        messages_for_signals,
        temperature,
        tool_names,
        user_message_preview,
    } = parsed;

    // Record LLM-specific span attributes
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

    // --- Phase 2: Resolve conversation state (v1/responses API) ---
    let state_ctx = match resolve_conversation_state(
        &mut client_request,
        is_responses_api_client,
        &state.state_storage,
        &state.llm_providers,
        &alias_resolved_model,
        &request_path,
        is_streaming_request,
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(response) => return Ok(response),
    };

    // Serialize request for upstream BEFORE router consumes it
    let client_request_bytes_for_upstream: Bytes =
        match ProviderRequestType::to_bytes(&client_request) {
            Ok(bytes) => bytes.into(),
            Err(err) => {
                warn!(error = %err, "failed to serialize request for upstream");
                let mut r = Response::new(full(format!("Failed to serialize request: {}", err)));
                *r.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                return Ok(r);
            }
        };

    // --- Phase 3: Route the request ---
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
            Arc::clone(&state.router_service),
            client_request,
            &traceparent,
            &request_path,
            &request_id,
        )
        .await
    }
    .instrument(routing_span)
    .await
    {
        Ok(result) => result,
        Err(err) => {
            let mut internal_error = Response::new(full(err.message));
            *internal_error.status_mut() = err.status_code;
            return Ok(internal_error);
        }
    };

    // Determine final model (router returns "none" when it doesn't select a specific model)
    let router_selected_model = routing_result.model_name;
    let resolved_model = if router_selected_model != "none" {
        router_selected_model
    } else {
        alias_resolved_model.clone()
    };
    tracing::Span::current().record(tracing_llm::MODEL_NAME, resolved_model.as_str());

    // --- Phase 4: Forward to upstream and stream back ---
    send_upstream(
        &state.http_client,
        &full_qualified_llm_provider_url,
        &mut request_headers,
        client_request_bytes_for_upstream,
        &model_from_request,
        &alias_resolved_model,
        &resolved_model,
        &model_name_only,
        &request_path,
        is_streaming_request,
        messages_for_signals,
        state_ctx,
        state.state_storage.clone(),
        request_id,
    )
    .await
}

// ---------------------------------------------------------------------------
// Phase 1 — Parse & validate the incoming request
// ---------------------------------------------------------------------------

/// All pre-validated request data extracted from the raw HTTP request.
struct PreparedRequest {
    client_request: ProviderRequestType,
    model_from_request: String,
    alias_resolved_model: String,
    model_name_only: String,
    is_streaming_request: bool,
    is_responses_api_client: bool,
    messages_for_signals: Option<Vec<Message>>,
    temperature: Option<f32>,
    tool_names: Option<Vec<String>>,
    user_message_preview: Option<String>,
}

/// Parse the body, resolve the model alias, and validate the model exists.
///
/// Returns `Err(Response)` for early-exit error responses (400 etc.).
async fn parse_and_validate_request(
    request: Request<hyper::body::Incoming>,
    request_path: &str,
    model_aliases: &Arc<Option<HashMap<String, ModelAlias>>>,
    llm_providers: &Arc<RwLock<LlmProviders>>,
) -> Result<PreparedRequest, Response<BoxBody<Bytes, hyper::Error>>> {
    let chat_request_bytes = request
        .collect()
        .await
        .map_err(|_| {
            let mut r = Response::new(full("Failed to read request body"));
            *r.status_mut() = StatusCode::BAD_REQUEST;
            r
        })?
        .to_bytes();

    debug!(
        body = %String::from_utf8_lossy(&chat_request_bytes),
        "request body received"
    );

    let api_type = SupportedAPIsFromClient::from_endpoint(request_path).ok_or_else(|| {
        warn!(path = %request_path, "unsupported endpoint");
        let mut r = Response::new(full(format!("Unsupported endpoint: {}", request_path)));
        *r.status_mut() = StatusCode::BAD_REQUEST;
        r
    })?;

    let mut client_request = ProviderRequestType::try_from((&chat_request_bytes[..], &api_type))
        .map_err(|err| {
            warn!(error = %err, "failed to parse request as ProviderRequestType");
            let mut r = Response::new(full(format!("Failed to parse request: {}", err)));
            *r.status_mut() = StatusCode::BAD_REQUEST;
            r
        })?;

    let client_api = SupportedAPIsFromClient::from_endpoint(request_path);
    let is_responses_api_client = matches!(
        client_api,
        Some(SupportedAPIsFromClient::OpenAIResponsesAPI(_))
    );

    let model_from_request = client_request.model().to_string();
    let temperature = client_request.get_temperature();
    let is_streaming_request = client_request.is_streaming();
    let alias_resolved_model = resolve_model_alias(&model_from_request, model_aliases);

    // Validate model exists in configuration
    if llm_providers
        .read()
        .await
        .get(&alias_resolved_model)
        .is_none()
    {
        let err_msg = format!(
            "Model '{}' not found in configured providers",
            alias_resolved_model
        );
        warn!(model = %alias_resolved_model, "model not found in configured providers");
        let mut r = Response::new(full(err_msg));
        *r.status_mut() = StatusCode::BAD_REQUEST;
        return Err(r);
    }

    // Strip provider prefix for upstream (e.g. "openai/gpt-4" → "gpt-4")
    let model_name_only = alias_resolved_model
        .split_once('/')
        .map(|(_, model)| model.to_string())
        .unwrap_or_else(|| alias_resolved_model.clone());

    // Extract span attributes and messages before mutating client_request
    let tool_names = client_request.get_tool_names();
    let user_message_preview = client_request
        .get_recent_user_message()
        .map(|msg| truncate_message(&msg, 50));
    let messages_for_signals = Some(client_request.get_messages());

    // Set the upstream model name and strip routing metadata
    client_request.set_model(model_name_only.clone());
    if client_request.remove_metadata_key("archgw_preference_config") {
        debug!("removed archgw_preference_config from metadata");
    }

    Ok(PreparedRequest {
        client_request,
        model_from_request,
        alias_resolved_model,
        model_name_only,
        is_streaming_request,
        is_responses_api_client,
        messages_for_signals,
        temperature,
        tool_names,
        user_message_preview,
    })
}

// ---------------------------------------------------------------------------
// Phase 2 — Resolve conversation state (v1/responses API)
// ---------------------------------------------------------------------------

/// Holds the state management context resolved from a v1/responses request.
struct ConversationStateContext {
    should_manage_state: bool,
    original_input_items: Vec<hermesllm::apis::openai_responses::InputItem>,
}

/// If the client uses the v1/responses API and the upstream provider doesn't
/// support it natively, we manage conversation state ourselves.
///
/// This resolves `previous_response_id`, merges conversation history, and
/// updates the request in place.
///
/// Returns `Err(Response)` for early-exit (e.g. 409 Conflict).
async fn resolve_conversation_state(
    client_request: &mut ProviderRequestType,
    is_responses_api_client: bool,
    state_storage: &Option<Arc<dyn StateStorage>>,
    llm_providers: &Arc<RwLock<LlmProviders>>,
    alias_resolved_model: &str,
    request_path: &str,
    is_streaming_request: bool,
) -> Result<ConversationStateContext, Response<BoxBody<Bytes, hyper::Error>>> {
    if !is_responses_api_client {
        return Ok(ConversationStateContext {
            should_manage_state: false,
            original_input_items: Vec::new(),
        });
    }

    let (responses_req, state_store) = match (client_request, state_storage) {
        (ProviderRequestType::ResponsesAPIRequest(ref mut req), Some(store)) => (req, store),
        _ => {
            return Ok(ConversationStateContext {
                should_manage_state: false,
                original_input_items: Vec::new(),
            });
        }
    };

    let mut original_input_items = extract_input_items(&responses_req.input);

    // Check whether the upstream supports v1/responses natively
    let upstream_path = get_upstream_path(
        llm_providers,
        alias_resolved_model,
        request_path,
        alias_resolved_model,
        is_streaming_request,
    )
    .await;

    let upstream_api = SupportedUpstreamAPIs::from_endpoint(&upstream_path);
    let should_manage_state = !matches!(
        upstream_api,
        Some(SupportedUpstreamAPIs::OpenAIResponsesAPI(_))
    );

    if !should_manage_state {
        debug!("upstream supports ResponsesAPI natively");
        return Ok(ConversationStateContext {
            should_manage_state: false,
            original_input_items,
        });
    }

    // Retrieve and combine conversation history if previous_response_id exists
    if let Some(ref prev_resp_id) = responses_req.previous_response_id {
        match retrieve_and_combine_input(state_store.clone(), prev_resp_id, original_input_items)
            .await
        {
            Ok(combined_input) => {
                responses_req.input = InputParam::Items(combined_input.clone());
                original_input_items = combined_input;
                info!(
                    items = original_input_items.len(),
                    "updated request with conversation history"
                );
            }
            Err(StateStorageError::NotFound(_)) => {
                warn!(previous_response_id = %prev_resp_id, "previous response_id not found");
                let err_msg = format!(
                    "Conversation state not found for previous_response_id: {}",
                    prev_resp_id
                );
                let mut r = Response::new(full(err_msg));
                *r.status_mut() = StatusCode::CONFLICT;
                return Err(r);
            }
            Err(e) => {
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

    Ok(ConversationStateContext {
        should_manage_state,
        original_input_items,
    })
}

// ---------------------------------------------------------------------------
// Phase 4 — Forward to upstream and stream the response back
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn send_upstream(
    http_client: &reqwest::Client,
    upstream_url: &str,
    request_headers: &mut hyper::HeaderMap,
    body: bytes::Bytes,
    model_from_request: &str,
    alias_resolved_model: &str,
    resolved_model: &str,
    model_name_only: &str,
    request_path: &str,
    is_streaming_request: bool,
    messages_for_signals: Option<Vec<Message>>,
    state_ctx: ConversationStateContext,
    state_storage: Option<Arc<dyn StateStorage>>,
    request_id: String,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let span_name = if model_from_request == resolved_model {
        format!("POST {} {}", request_path, resolved_model)
    } else {
        format!(
            "POST {} {} -> {}",
            request_path, model_from_request, resolved_model
        )
    };
    get_active_span(|span| {
        span.update_name(span_name.clone());
    });

    debug!(
        url = %upstream_url,
        provider_hint = %resolved_model,
        upstream_model = %model_name_only,
        "Routing to upstream"
    );

    request_headers.insert(
        ARCH_PROVIDER_HINT_HEADER,
        header::HeaderValue::from_str(resolved_model).unwrap(),
    );
    request_headers.insert(
        header::HeaderName::from_static(ARCH_IS_STREAMING_HEADER),
        header::HeaderValue::from_str(&is_streaming_request.to_string()).unwrap(),
    );
    request_headers.remove(header::CONTENT_LENGTH);

    // Inject current span's trace context so upstream spans are children of plano(llm)
    global::get_text_map_propagator(|propagator| {
        let cx = tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
        propagator.inject_context(&cx, &mut HeaderInjector(request_headers));
    });

    let request_start_time = std::time::Instant::now();

    let llm_response = match http_client
        .post(upstream_url)
        .headers(request_headers.clone())
        .body(body)
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

    // Propagate upstream headers and status
    let response_headers = llm_response.headers().clone();
    let upstream_status = llm_response.status();
    let mut response = Response::builder().status(upstream_status);
    let headers = response.headers_mut().unwrap();
    for (name, value) in response_headers.iter() {
        headers.insert(name, value.clone());
    }

    let byte_stream = llm_response.bytes_stream();

    // Create base processor for metrics and tracing
    let base_processor = ObservableStreamProcessor::new(
        operation_component::LLM,
        span_name,
        request_start_time,
        messages_for_signals,
    );

    // Wrap with state management processor when needed
    let streaming_response = if let (true, false, Some(state_store)) = (
        state_ctx.should_manage_state,
        state_ctx.original_input_items.is_empty(),
        state_storage,
    ) {
        let content_encoding = response_headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let state_processor = ResponsesStateProcessor::new(
            base_processor,
            state_store,
            state_ctx.original_input_items,
            alias_resolved_model.to_string(),
            resolved_model.to_string(),
            is_streaming_request,
            false,
            content_encoding,
            request_id,
        );
        create_streaming_response(byte_stream, state_processor, 16)
    } else {
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract or generate a W3C `traceparent` header value.
fn extract_or_generate_traceparent(headers: &hyper::HeaderMap) -> String {
    headers
        .get(TRACE_PARENT_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let trace_id = uuid::Uuid::new_v4().to_string().replace("-", "");
            let tp = format!("00-{}-0000000000000000-01", trace_id);
            warn!(
                generated_traceparent = %tp,
                "TRACE_PARENT header missing, generated new traceparent"
            );
            tp
        })
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
async fn get_upstream_path(
    llm_providers: &Arc<RwLock<LlmProviders>>,
    model_name: &str,
    request_path: &str,
    resolved_model: &str,
    is_streaming: bool,
) -> String {
    let (provider_id, base_url_path_prefix) = get_provider_info(llm_providers, model_name).await;

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

/// Helper to get provider info (ProviderId and base_url_path_prefix).
async fn get_provider_info(
    llm_providers: &Arc<RwLock<LlmProviders>>,
    model_name: &str,
) -> (hermesllm::ProviderId, Option<String>) {
    let providers_lock = llm_providers.read().await;

    if let Some(provider) = providers_lock.get(model_name) {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        return (provider_id, prefix);
    }

    if let Some(provider) = providers_lock.default() {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        (provider_id, prefix)
    } else {
        warn!("No default provider found, falling back to OpenAI");
        (hermesllm::ProviderId::OpenAI, None)
    }
}
