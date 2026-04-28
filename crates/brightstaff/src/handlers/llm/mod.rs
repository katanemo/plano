use bytes::Bytes;
use common::configuration::{FilterPipeline, LlmProvider, ModelAlias};
use common::consts::{
    ARCH_IS_STREAMING_HEADER, ARCH_PROVIDER_HINT_HEADER, ARCH_ROUTING_HEADER, MODEL_AFFINITY_HEADER,
};
use common::llm_providers::LlmProviders;
use common::retry::error_response::build_error_response;
use common::retry::orchestrator::RetryOrchestrator;
use common::retry::{rebuild_request_for_provider, RequestContext, RequestSignature};
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
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, info_span, warn, Instrument};

pub(crate) mod model_selection;

use crate::app_state::AppState;
use crate::handlers::agents::pipeline::PipelineProcessor;
use crate::handlers::extract_request_id;
use crate::handlers::full;
use crate::metrics as bs_metrics;
use crate::state::response_state_processor::ResponsesStateProcessor;
use crate::state::{
    extract_input_items, retrieve_and_combine_input, StateStorage, StateStorageError,
};
use crate::streaming::{
    create_streaming_response, create_streaming_response_with_output_filter, truncate_message,
    LlmMetricsCtx, ObservableStreamProcessor, StreamProcessor,
};
use crate::tracing::{
    collect_custom_trace_attributes, llm as tracing_llm, operation_component,
    plano as tracing_plano, set_service_name,
};
use model_selection::router_chat_get_upstream_model;

const PERPLEXITY_PROVIDER_PREFIX: &str = "perplexity/";

pub async fn llm_chat(
    request: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_path = request.uri().path().to_string();
    let request_headers = request.headers().clone();
    let request_id = extract_request_id(&request);
    let custom_attrs =
        collect_custom_trace_attributes(&request_headers, state.span_attributes.as_ref());

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
        state,
        custom_attrs,
        request_id,
        request_path,
        request_headers,
    )
    .instrument(request_span)
    .await
}

async fn llm_chat_inner(
    request: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
    custom_attrs: HashMap<String, String>,
    request_id: String,
    request_path: String,
    mut request_headers: hyper::HeaderMap,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    // Set service name for LLM operations
    set_service_name(operation_component::LLM);
    get_active_span(|span| {
        for (key, value) in &custom_attrs {
            span.set_attribute(opentelemetry::KeyValue::new(key.clone(), value.clone()));
        }
    });

    // Session pinning: extract session ID and check cache before routing
    let session_id: Option<String> = request_headers
        .get(MODEL_AFFINITY_HEADER)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let tenant_id: Option<String> = state
        .orchestrator_service
        .tenant_header()
        .and_then(|hdr| request_headers.get(hdr))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let cached_route = if let Some(ref sid) = session_id {
        state
            .orchestrator_service
            .get_cached_route(sid, tenant_id.as_deref())
            .await
    } else {
        None
    };
    let (pinned_model, pinned_route_name): (Option<String>, Option<String>) = match cached_route {
        Some(c) => (Some(c.model_name), c.route_name),
        None => (None, None),
    };

    // Record session id on the LLM span for the observability console.
    if let Some(ref sid) = session_id {
        get_active_span(|span| {
            span.set_attribute(opentelemetry::KeyValue::new(
                tracing_plano::SESSION_ID,
                sid.clone(),
            ));
        });
    }
    if let Some(ref route_name) = pinned_route_name {
        get_active_span(|span| {
            span.set_attribute(opentelemetry::KeyValue::new(
                tracing_plano::ROUTE_NAME,
                route_name.clone(),
            ));
        });
    }

    let full_qualified_llm_provider_url = format!("{}{}", state.llm_provider_url, request_path);

    // --- Phase 1: Parse and validate the incoming request ---
    let parsed = match parse_and_validate_request(
        request,
        &request_path,
        &state.model_aliases,
        &state.llm_providers,
        state.signals_enabled,
    )
    .await
    {
        Ok(p) => p,
        Err(response) => return Ok(response),
    };

    let PreparedRequest {
        mut client_request,
        chat_request_bytes,
        model_from_request,
        alias_resolved_model,
        model_name_only,
        is_streaming_request,
        is_responses_api_client,
        messages_for_signals,
        temperature,
        tool_names,
        user_message_preview,
        inline_routing_preferences,
        client_api,
        provider_id,
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

    // --- Phase 1b: Input filter processing for model listener ---
    if let Some(ref input_chain) = state.filter_pipeline.input {
        if !input_chain.is_empty() {
            debug!(input_filters = ?input_chain.filter_ids, "processing model listener input filters");
            let chain = input_chain.to_agent_filter_chain("model_listener");
            let mut pipeline_processor = PipelineProcessor::default();
            match pipeline_processor
                .process_raw_filter_chain(
                    &chat_request_bytes,
                    &chain,
                    &input_chain.agents,
                    &request_headers,
                    &request_path,
                )
                .await
            {
                Ok(filtered_bytes) => {
                    let api_type = SupportedAPIsFromClient::from_endpoint(request_path.as_str())
                        .expect("endpoint validated in parse_and_validate_request");
                    match ProviderRequestType::try_from((&filtered_bytes[..], &api_type)) {
                        Ok(updated_request) => {
                            client_request = updated_request;
                            info!("input filter chain processed successfully");
                        }
                        Err(parse_err) => {
                            warn!(error = %parse_err, "input filter returned invalid request JSON");
                            return Ok(common::errors::BrightStaffError::InvalidRequest(format!(
                                "Input filter returned invalid request: {}",
                                parse_err
                            ))
                            .into_response());
                        }
                    }
                }
                Err(super::agents::pipeline::PipelineError::ClientError {
                    agent,
                    status,
                    body,
                }) => {
                    warn!(agent = %agent, status = %status, body = %body, "client error from filter chain");
                    let error_json = serde_json::json!({
                        "error": "FilterChainError",
                        "agent": agent,
                        "status": status,
                        "agent_response": body
                    });
                    let mut error_response = Response::new(full(error_json.to_string()));
                    *error_response.status_mut() =
                        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
                    error_response.headers_mut().insert(
                        hyper::header::CONTENT_TYPE,
                        hyper::header::HeaderValue::from_static("application/json"),
                    );
                    return Ok(error_response);
                }
                Err(err) => {
                    warn!(error = %err, "filter chain processing failed");
                    let mut internal_error =
                        Response::new(full(format!("Filter chain processing failed: {}", err)));
                    *internal_error.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    return Ok(internal_error);
                }
            }
        }
    }

    // Normalize for upstream after input filters
    if let Some(ref client_api_kind) = client_api {
        let upstream_api =
            provider_id.compatible_api_for_client(client_api_kind, is_streaming_request);
        if let Err(e) = client_request.normalize_for_upstream(provider_id, &upstream_api) {
            warn!(
                "request_id={}: normalize_for_upstream failed: {}",
                request_id, e
            );
            let mut bad_request = Response::new(full(e.message));
            *bad_request.status_mut() = StatusCode::BAD_REQUEST;
            return Ok(bad_request);
        }
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

    // Use the original bytes (from extract_routing_policy) for upstream to preserve
    // JSON key order, whitespace, and unknown fields — critical for prompt cache hits.
    // Only fall back to re-serialization if input filters modified the request.
    let client_request_bytes_for_upstream: Bytes = chat_request_bytes.clone();

    // --- Phase 3: Route the request (or use pinned model from session cache) ---
    let resolved_model = if let Some(cached_model) = pinned_model {
        info!(
            session_id = %session_id.as_deref().unwrap_or(""),
            model = %cached_model,
            "using pinned routing decision from cache"
        );
        cached_model
    } else {
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
                Arc::clone(&state.orchestrator_service),
                client_request,
                &request_path,
                &request_id,
                inline_routing_preferences,
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

        let (router_selected_model, route_name) =
            (routing_result.model_name, routing_result.route_name);
        let model = if router_selected_model != "none" {
            router_selected_model
        } else {
            alias_resolved_model.clone()
        };

        // Record route name on the LLM span (only when the orchestrator produced one).
        if let Some(ref rn) = route_name {
            if !rn.is_empty() && rn != "none" {
                get_active_span(|span| {
                    span.set_attribute(opentelemetry::KeyValue::new(
                        tracing_plano::ROUTE_NAME,
                        rn.clone(),
                    ));
                });
            }
        }

        if let Some(ref sid) = session_id {
            state
                .orchestrator_service
                .cache_route(sid.clone(), tenant_id.as_deref(), model.clone(), route_name)
                .await;
        }

        model
    };
    tracing::Span::current().record(tracing_llm::MODEL_NAME, resolved_model.as_str());

    // --- Phase 4: Forward to upstream and stream back ---
    // Check if the resolved provider has a retry_policy configured.
    // If so, use the RetryOrchestrator to wrap the upstream call with retry logic.
    let resolved_provider: Option<Arc<LlmProvider>> =
        state.llm_providers.read().await.get(&resolved_model);

    let has_retry_policy = resolved_provider
        .as_ref()
        .and_then(|p| p.retry_policy.as_ref())
        .is_some();

    if has_retry_policy {
        send_upstream_with_retry(
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
            &state.filter_pipeline,
            &resolved_provider.unwrap(),
            &state.llm_providers,
        )
        .await
    } else {
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
            &state.filter_pipeline,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Phase 1 — Parse & validate the incoming request
// ---------------------------------------------------------------------------

/// All pre-validated request data extracted from the raw HTTP request.
struct PreparedRequest {
    client_request: ProviderRequestType,
    chat_request_bytes: Bytes,
    model_from_request: String,
    alias_resolved_model: String,
    model_name_only: String,
    is_streaming_request: bool,
    is_responses_api_client: bool,
    messages_for_signals: Option<Vec<Message>>,
    temperature: Option<f32>,
    tool_names: Option<Vec<String>>,
    user_message_preview: Option<String>,
    inline_routing_preferences: Option<Vec<common::configuration::TopLevelRoutingPreference>>,
    client_api: Option<SupportedAPIsFromClient>,
    provider_id: hermesllm::ProviderId,
}

/// Parse the body, resolve the model alias, and validate the model exists.
///
/// Returns `Err(Response)` for early-exit error responses (400 etc.).
async fn parse_and_validate_request(
    request: Request<hyper::body::Incoming>,
    request_path: &str,
    model_aliases: &Option<HashMap<String, ModelAlias>>,
    llm_providers: &Arc<RwLock<LlmProviders>>,
    signals_enabled: bool,
) -> Result<PreparedRequest, Response<BoxBody<Bytes, hyper::Error>>> {
    let raw_bytes = request
        .collect()
        .await
        .map_err(|_| {
            let mut r = Response::new(full("Failed to read request body"));
            *r.status_mut() = StatusCode::BAD_REQUEST;
            r
        })?
        .to_bytes();

    debug!(
        body = %String::from_utf8_lossy(&raw_bytes),
        "request body received"
    );

    // Extract routing_preferences from request body if present
    let (chat_request_bytes, inline_routing_preferences) =
        crate::handlers::routing_service::extract_routing_policy(&raw_bytes).map_err(|err| {
            warn!(error = %err, "failed to parse request JSON");
            let mut r = Response::new(full(format!("Failed to parse request: {}", err)));
            *r.status_mut() = StatusCode::BAD_REQUEST;
            r
        })?;

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

    let is_responses_api_client =
        matches!(api_type, SupportedAPIsFromClient::OpenAIResponsesAPI(_));
    let client_api = Some(api_type);

    let model_from_request = client_request.model().to_string();
    let temperature = client_request.get_temperature();
    let is_streaming_request = client_request.is_streaming();
    let alias_resolved_model = resolve_model_alias(&model_from_request, model_aliases);
    let (provider_id, _, _) = get_provider_info(llm_providers, &alias_resolved_model).await;

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
    let messages_for_signals = if signals_enabled {
        Some(client_request.get_messages())
    } else {
        None
    };

    // Set the upstream model name and strip routing metadata
    client_request.set_model(model_name_only.clone());
    if client_request.remove_metadata_key("archgw_preference_config") {
        debug!("removed archgw_preference_config from metadata");
    }
    if client_request.remove_metadata_key("plano_preference_config") {
        debug!("removed plano_preference_config from metadata");
    }

    Ok(PreparedRequest {
        client_request,
        chat_request_bytes,
        model_from_request,
        alias_resolved_model,
        model_name_only,
        is_streaming_request,
        is_responses_api_client,
        messages_for_signals,
        temperature,
        tool_names,
        user_message_preview,
        inline_routing_preferences,
        client_api,
        provider_id,
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
    filter_pipeline: &Arc<FilterPipeline>,
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

    if let Ok(val) = header::HeaderValue::from_str(resolved_model) {
        request_headers.insert(ARCH_PROVIDER_HINT_HEADER, val);
    }
    request_headers.insert(
        header::HeaderName::from_static(ARCH_IS_STREAMING_HEADER),
        header::HeaderValue::from_static(if is_streaming_request {
            "true"
        } else {
            "false"
        }),
    );
    request_headers.remove(header::CONTENT_LENGTH);

    // Inject current span's trace context so upstream spans are children of plano(llm)
    global::get_text_map_propagator(|propagator| {
        let cx = tracing_opentelemetry::OpenTelemetrySpanExt::context(&tracing::Span::current());
        propagator.inject_context(&cx, &mut HeaderInjector(request_headers));
    });

    let request_start_time = std::time::Instant::now();

    // Labels for LLM upstream metrics. We prefer `resolved_model` (post-routing)
    // and derive the provider from its `provider/model` prefix. This matches the
    // same model id the cost/latency router keys off.
    let (metric_provider_raw, metric_model_raw) = bs_metrics::split_provider_model(resolved_model);
    let metric_provider = metric_provider_raw.to_string();
    let metric_model = metric_model_raw.to_string();

    let llm_response = match http_client
        .post(upstream_url)
        .headers(request_headers.clone())
        .body(body)
        .send()
        .await
    {
        Ok(res) => res,
        Err(err) => {
            let err_class = bs_metrics::llm_error_class_from_reqwest(&err);
            bs_metrics::record_llm_upstream(
                &metric_provider,
                &metric_model,
                0,
                err_class,
                request_start_time.elapsed(),
            );
            let err_msg = format!("Failed to send request: {}", err);
            let mut internal_error = Response::new(full(err_msg));
            *internal_error.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            return Ok(internal_error);
        }
    };

    // Propagate upstream headers and status
    let response_headers = llm_response.headers().clone();
    let upstream_status = llm_response.status();

    // Upstream routers (e.g. DigitalOcean Gradient) may return an
    // `x-model-router-selected-route` header indicating which task-level
    // route the request was classified into (e.g. "Code Generation"). Surface
    // it as `plano.route.name` so the obs console's Route hit % panel can
    // show the breakdown even when Plano's own orchestrator wasn't in the
    // routing path. Any value from Plano's orchestrator already set earlier
    // takes precedence — this only fires when the span doesn't already have
    // a route name.
    if let Some(upstream_route) = response_headers
        .get("x-model-router-selected-route")
        .and_then(|v| v.to_str().ok())
    {
        if !upstream_route.is_empty() {
            get_active_span(|span| {
                span.set_attribute(opentelemetry::KeyValue::new(
                    crate::tracing::plano::ROUTE_NAME,
                    upstream_route.to_string(),
                ));
            });
        }
    }
    // Record the upstream HTTP status on the span for the obs console.
    get_active_span(|span| {
        span.set_attribute(opentelemetry::KeyValue::new(
            crate::tracing::http::STATUS_CODE,
            upstream_status.as_u16() as i64,
        ));
    });

    let mut response = Response::builder().status(upstream_status);
    if let Some(headers) = response.headers_mut() {
        for (name, value) in response_headers.iter() {
            headers.insert(name, value.clone());
        }
    }

    let byte_stream = llm_response.bytes_stream();

    // Create base processor for metrics and tracing
    let base_processor = ObservableStreamProcessor::new(
        operation_component::LLM,
        span_name,
        request_start_time,
        messages_for_signals,
    )
    .with_llm_metrics(LlmMetricsCtx {
        provider: metric_provider.clone(),
        model: metric_model.clone(),
        upstream_status: upstream_status.as_u16(),
    });

    let output_filter_request_headers = if filter_pipeline.has_output_filters() {
        Some(request_headers.clone())
    } else {
        None
    };

    // Pick the right processor: state-aware if needed, otherwise base metrics-only.
    let processor: Box<dyn StreamProcessor> = if let (true, false, Some(state_store)) = (
        state_ctx.should_manage_state,
        state_ctx.original_input_items.is_empty(),
        state_storage,
    ) {
        let content_encoding = response_headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        Box::new(ResponsesStateProcessor::new(
            base_processor,
            state_store,
            state_ctx.original_input_items,
            alias_resolved_model.to_string(),
            resolved_model.to_string(),
            is_streaming_request,
            false,
            content_encoding,
            request_id,
        ))
    } else {
        Box::new(base_processor)
    };

    let streaming_response = if let (Some(output_chain), Some(filter_headers)) = (
        filter_pipeline.output.as_ref().filter(|c| !c.is_empty()),
        output_filter_request_headers,
    ) {
        create_streaming_response_with_output_filter(
            byte_stream,
            processor,
            output_chain.clone(),
            filter_headers,
            request_path.to_string(),
        )
    } else {
        create_streaming_response(byte_stream, processor)
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

/// Retry-aware version of send_upstream. Uses the RetryOrchestrator to wrap
/// the upstream HTTP call with automatic retry and provider failover logic.
#[allow(clippy::too_many_arguments)]
async fn send_upstream_with_retry(
    http_client: &reqwest::Client,
    upstream_url: &str,
    request_headers: &mut hyper::HeaderMap,
    body: bytes::Bytes,
    model_from_request: &str,
    alias_resolved_model: &str,
    resolved_model: &str,
    _model_name_only: &str,
    request_path: &str,
    is_streaming_request: bool,
    messages_for_signals: Option<Vec<Message>>,
    state_ctx: ConversationStateContext,
    state_storage: Option<Arc<dyn StateStorage>>,
    request_id: String,
    filter_pipeline: &Arc<FilterPipeline>,
    primary_provider: &Arc<LlmProvider>,
    llm_providers: &Arc<RwLock<LlmProviders>>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let retry_policy = primary_provider.retry_policy.as_ref().unwrap();

    // Collect all providers for the retry orchestrator
    let all_providers: Vec<LlmProvider> = llm_providers
        .read()
        .await
        .iter()
        .map(|(_, p)| (*p).as_ref().clone())
        .collect();

    // Build request signature
    let request_signature = RequestSignature::new(
        &body,
        request_headers,
        is_streaming_request,
        alias_resolved_model.to_string(),
    );

    // Build request context
    let mut request_context = RequestContext {
        request_id: request_id.clone(),
        attempted_providers: HashSet::new(),
        retry_start_time: None,
        attempt_number: 0,
        request_retry_after_state: HashMap::new(),
        request_latency_block_state: HashMap::new(),
        request_signature: request_signature.clone(),
        errors: vec![],
    };

    let orchestrator = RetryOrchestrator::new_default();

    debug!(
        model = %alias_resolved_model,
        fallback_models = ?retry_policy.fallback_models,
        default_strategy = ?retry_policy.default_strategy,
        default_max_attempts = retry_policy.default_max_attempts,
        "Retry orchestrator initialized for request"
    );

    // Capture references for the forward_fn closure
    let base_url = upstream_url.to_string();
    let original_headers = request_headers.clone();
    let request_path_owned = request_path.to_string();
    let primary_model = alias_resolved_model.to_string();
    let http_client = http_client.clone();

    // The forward_fn handles the actual HTTP call to upstream for each attempt.
    let forward_fn = |body: &Bytes, target_provider: &LlmProvider| {
        let body = body.clone();
        let target_provider = target_provider.clone();
        let base_url = base_url.clone();
        let original_headers = original_headers.clone();
        let _request_path_owned = request_path_owned.clone();
        let primary_model = primary_model.clone();
        let http_client = http_client.clone();

        async move {
            let target_model = target_provider
                .model
                .as_deref()
                .unwrap_or(&target_provider.name);

            let (request_body, mut headers) = if target_model == primary_model {
                (body.clone(), original_headers.clone())
            } else {
                match rebuild_request_for_provider(&body, &target_provider, &original_headers) {
                    Ok((new_body, new_headers)) => (new_body, new_headers),
                    Err(e) => {
                        warn!(error = %e, "Failed to rebuild request for provider");
                        return Err(common::retry::error_detector::TimeoutError { duration_ms: 0 });
                    }
                }
            };

            // Resolve the upstream URL for the target provider.
            // Always route through the Envoy proxy (base_url) and let the
            // provider-hint header select the upstream cluster.  Building a
            // direct URL from the provider endpoint is wrong because the
            // endpoint field stores a bare hostname (no scheme), and
            // bypassing Envoy loses TLS, load-balancing, and observability.
            let upstream_url = base_url.clone();

            // Set provider hint header so the WASM gateway selects the
            // correct provider (and its credentials) for this retry attempt.
            // Do NOT set x-arch-llm-provider here — the WASM gateway sets it
            // via add_http_request_header after provider selection.  If we set
            // it too, Envoy sees a duplicate multi-value header that fails
            // exact-match routing and falls through to the 400 catch-all.
            headers.remove(header::HeaderName::from_static(ARCH_ROUTING_HEADER));
            headers.insert(
                ARCH_PROVIDER_HINT_HEADER,
                header::HeaderValue::from_str(target_model)
                    .unwrap_or_else(|_| header::HeaderValue::from_static("unknown")),
            );
            headers.remove(header::CONTENT_LENGTH);

            // Send the request
            let result = http_client
                .post(&upstream_url)
                .headers(headers)
                .body(request_body.to_vec())
                .send()
                .await;

            match result {
                Ok(res) => {
                    let status = res.status().as_u16();
                    let resp_headers = res.headers().clone();
                    let body_bytes = res.bytes().await.unwrap_or_default();

                    // Debug: log upstream response for retry attempts
                    if status >= 400 {
                        let body_preview = String::from_utf8_lossy(&body_bytes);
                        warn!(
                            "Retry upstream response: status={}, model={}, body={}",
                            status,
                            target_model,
                            &body_preview[..body_preview.len().min(500)]
                        );
                    }

                    let full_body = Full::new(body_bytes)
                        .map_err(|never| match never {})
                        .boxed();

                    let mut builder = Response::builder().status(status);
                    if let Some(hdrs) = builder.headers_mut() {
                        for (name, value) in resp_headers.iter() {
                            if let Ok(hyper_name) =
                                hyper::header::HeaderName::from_bytes(name.as_str().as_bytes())
                            {
                                if let Ok(hyper_value) =
                                    hyper::header::HeaderValue::from_bytes(value.as_bytes())
                                {
                                    hdrs.insert(hyper_name, hyper_value);
                                }
                            }
                        }
                    }

                    Ok(builder.body(full_body).unwrap())
                }
                Err(err) => {
                    warn!(error = %err, "Upstream request failed during retry");
                    Err(common::retry::error_detector::TimeoutError { duration_ms: 0 })
                }
            }
        }
    };

    // Execute the retry orchestrator
    let retry_result = orchestrator
        .execute(
            &body,
            &request_signature,
            primary_provider.as_ref(),
            retry_policy,
            &all_providers,
            &mut request_context,
            forward_fn,
        )
        .await;

    match retry_result {
        Ok(http_response) => {
            // Success (possibly after retries) — stream the response back to client
            let upstream_status = http_response.status();
            let response_headers = http_response.headers().clone();

            let span_name = if model_from_request == resolved_model {
                format!("POST {} {}", request_path, resolved_model)
            } else {
                format!(
                    "POST {} {} -> {}",
                    request_path, model_from_request, resolved_model
                )
            };

            let mut response = Response::builder().status(upstream_status);
            let headers = response.headers_mut().unwrap();
            for (header_name, header_value) in response_headers.iter() {
                headers.insert(header_name, header_value.clone());
            }

            // Collect the body from the HttpResponse
            let body_bytes = http_response
                .into_body()
                .collect()
                .await
                .map(|collected| collected.to_bytes())
                .unwrap_or_default();

            let byte_stream = futures::stream::iter(vec![Ok::<Bytes, reqwest::Error>(body_bytes)]);

            let (metric_provider_raw, metric_model_raw) =
                bs_metrics::split_provider_model(resolved_model);

            let base_processor = ObservableStreamProcessor::new(
                operation_component::LLM,
                span_name,
                std::time::Instant::now(),
                messages_for_signals,
            )
            .with_llm_metrics(LlmMetricsCtx {
                provider: metric_provider_raw.to_string(),
                model: metric_model_raw.to_string(),
                upstream_status: upstream_status.as_u16(),
            });

            let output_filter_request_headers = if filter_pipeline.has_output_filters() {
                Some(request_headers.clone())
            } else {
                None
            };

            let processor: Box<dyn StreamProcessor> = if let (true, false, Some(state_store)) = (
                state_ctx.should_manage_state,
                state_ctx.original_input_items.is_empty(),
                &state_storage,
            ) {
                let content_encoding = response_headers
                    .get("content-encoding")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());

                Box::new(ResponsesStateProcessor::new(
                    base_processor,
                    state_store.clone(),
                    state_ctx.original_input_items,
                    alias_resolved_model.to_string(),
                    resolved_model.to_string(),
                    is_streaming_request,
                    false,
                    content_encoding,
                    request_id.clone(),
                ))
            } else {
                Box::new(base_processor)
            };

            let streaming_response = if let (Some(output_chain), Some(filter_headers)) = (
                filter_pipeline.output.as_ref().filter(|c| !c.is_empty()),
                output_filter_request_headers,
            ) {
                create_streaming_response_with_output_filter(
                    byte_stream,
                    processor,
                    output_chain.clone(),
                    filter_headers,
                    request_path.to_string(),
                )
            } else {
                create_streaming_response(byte_stream, processor)
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
        Err(retry_exhausted_error) => {
            // All retries exhausted — build structured error response
            info!(
                request_id = %request_id,
                total_attempts = retry_exhausted_error.attempts.len(),
                budget_exhausted = retry_exhausted_error.retry_budget_exhausted,
                "All retries exhausted"
            );

            let error_resp = build_error_response(&retry_exhausted_error, &request_id);

            // Convert Full<Bytes> body to BoxBody<Bytes, hyper::Error>
            let (parts, full_body) = error_resp.into_parts();
            let boxed_body = full_body.map_err(|never| match never {}).boxed();

            Ok(Response::from_parts(parts, boxed_body))
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolves model aliases by looking up the requested model in the model_aliases map.
/// Returns the target model if an alias is found, otherwise returns the original model.
fn resolve_model_alias(
    model_from_request: &str,
    model_aliases: &Option<HashMap<String, ModelAlias>>,
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
    let (provider_id, base_url_path_prefix, use_unversioned_paths) =
        get_provider_info(llm_providers, model_name).await;

    let Some(client_api) = SupportedAPIsFromClient::from_endpoint(request_path) else {
        return request_path.to_string();
    };

    client_api.target_endpoint_for_provider(
        &provider_id,
        request_path,
        resolved_model,
        is_streaming,
        base_url_path_prefix.as_deref(),
        use_unversioned_paths,
    )
}

/// Helper to get provider info (ProviderId and base_url_path_prefix).
async fn get_provider_info(
    llm_providers: &Arc<RwLock<LlmProviders>>,
    model_name: &str,
) -> (hermesllm::ProviderId, Option<String>, bool) {
    let providers_lock = llm_providers.read().await;

    if let Some(provider) = providers_lock.get(model_name) {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        let use_unversioned_paths = provider.name.starts_with(PERPLEXITY_PROVIDER_PREFIX);
        return (provider_id, prefix, use_unversioned_paths);
    }

    if let Some(provider) = providers_lock.default() {
        let provider_id = provider.provider_interface.to_provider_id();
        let prefix = provider.base_url_path_prefix.clone();
        let use_unversioned_paths = provider.name.starts_with(PERPLEXITY_PROVIDER_PREFIX);
        (provider_id, prefix, use_unversioned_paths)
    } else {
        warn!("No default provider found, falling back to OpenAI");
        (hermesllm::ProviderId::OpenAI, None, false)
    }
}

#[cfg(test)]
mod tests {
    use super::{get_provider_info, get_upstream_path};
    use common::configuration::{LlmProvider, LlmProviderType};
    use common::llm_providers::LlmProviders;
    use hermesllm::apis::OpenAIApi;
    use hermesllm::clients::SupportedAPIsFromClient;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn build_provider(name: &str, model: &str) -> LlmProvider {
        LlmProvider {
            name: name.to_string(),
            provider_interface: LlmProviderType::OpenAI,
            access_key: Some("test_key".to_string()),
            model: Some(model.to_string()),
            default: Some(false),
            ..Default::default()
        }
    }

    fn providers_lock(providers: Vec<LlmProvider>) -> Arc<RwLock<LlmProviders>> {
        Arc::new(RwLock::new(
            LlmProviders::try_from(providers).expect("test providers should be valid"),
        ))
    }

    #[tokio::test]
    async fn test_get_provider_info_marks_perplexity_as_unversioned() {
        let providers = providers_lock(vec![build_provider("perplexity/sonar-pro", "sonar-pro")]);

        let (provider_id, prefix, use_unversioned_paths) =
            get_provider_info(&providers, "perplexity/sonar-pro").await;

        assert_eq!(provider_id, hermesllm::ProviderId::OpenAI);
        assert_eq!(prefix, None);
        assert!(use_unversioned_paths);
    }

    #[tokio::test]
    async fn test_get_upstream_path_for_perplexity_uses_unversioned_chat_endpoint() {
        let providers = providers_lock(vec![build_provider("perplexity/sonar-pro", "sonar-pro")]);

        let upstream_path = get_upstream_path(
            &providers,
            "perplexity/sonar-pro",
            "/v1/chat/completions",
            "sonar-pro",
            false,
        )
        .await;

        assert_eq!(upstream_path, "/chat/completions");
    }

    #[tokio::test]
    async fn test_get_upstream_path_for_non_perplexity_keeps_v1_chat_endpoint() {
        let providers = providers_lock(vec![build_provider("openai/gpt-4o-mini", "gpt-4o-mini")]);

        let upstream_path = get_upstream_path(
            &providers,
            "openai/gpt-4o-mini",
            "/v1/chat/completions",
            "gpt-4o-mini",
            false,
        )
        .await;

        assert_eq!(upstream_path, "/v1/chat/completions");
    }

    #[tokio::test]
    async fn test_perplexity_with_and_without_versioning_paths() {
        let providers = providers_lock(vec![build_provider("perplexity/sonar-pro", "sonar-pro")]);

        // This is the path Plano should use for Perplexity (works).
        let success_path = get_upstream_path(
            &providers,
            "perplexity/sonar-pro",
            "/v1/chat/completions",
            "sonar-pro",
            false,
        )
        .await;
        assert_eq!(success_path, "/chat/completions");

        // This is the generic OpenAI default path; for Perplexity this would 404.
        let fail_path = SupportedAPIsFromClient::OpenAIChatCompletions(OpenAIApi::ChatCompletions)
            .target_endpoint_for_provider(
                &hermesllm::ProviderId::OpenAI,
                "/v1/chat/completions",
                "sonar-pro",
                false,
                None,
                false,
            );
        assert_eq!(fail_path, "/v1/chat/completions");
        assert_ne!(success_path, fail_path);
    }
}
