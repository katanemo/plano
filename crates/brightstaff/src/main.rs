use brightstaff::app_state::AppState;
use brightstaff::handlers::agents::orchestrator::agent_chat;
use brightstaff::handlers::empty;
use brightstaff::handlers::function_calling::function_calling_chat_handler;
use brightstaff::handlers::llm::llm_chat;
use brightstaff::handlers::models::list_models;
use brightstaff::handlers::routing_service::routing_decision;
use brightstaff::router::llm::RouterService;
use brightstaff::router::model_metrics::ModelMetricsService;
use brightstaff::router::orchestrator::OrchestratorService;
use brightstaff::session_cache::memory::MemorySessionCache;
use brightstaff::session_cache::SessionCache;
use brightstaff::state::memory::MemoryConversationalStorage;
use brightstaff::state::postgresql::PostgreSQLConversationStorage;
use brightstaff::state::StateStorage;
use brightstaff::tracing::init_tracer;
use bytes::Bytes;
use common::configuration::{
    Agent, Configuration, FilterPipeline, ListenerType, ResolvedFilterChain,
};
use common::consts::{CHAT_COMPLETIONS_PATH, MESSAGES_PATH, OPENAI_RESPONSES_API_PATH};
use common::llm_providers::LlmProviders;
use http_body_util::combinators::BoxBody;
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use opentelemetry::global;
use opentelemetry::trace::FutureExt;
use opentelemetry_http::HeaderExtractor;
use std::collections::HashMap;
use std::sync::Arc;
use std::{env, fs};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

const BIND_ADDRESS: &str = "0.0.0.0:9091";
const DEFAULT_ROUTING_LLM_PROVIDER: &str = "arch-router";
const DEFAULT_ROUTING_MODEL_NAME: &str = "Arch-Router";
const DEFAULT_ORCHESTRATOR_LLM_PROVIDER: &str = "plano-orchestrator";
const DEFAULT_ORCHESTRATOR_MODEL_NAME: &str = "Plano-Orchestrator";

/// Parse a version string like `v0.4.0`, `v0.3.0`, `0.2.0` into a `(major, minor, patch)` tuple.
/// Missing parts default to 0. Non-numeric parts are treated as 0.
fn parse_semver(version: &str) -> (u32, u32, u32) {
    let v = version.trim_start_matches('v');
    let mut parts = v.splitn(3, '.').map(|p| p.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major, minor, patch)
}

/// CORS pre-flight response for the models endpoint.
fn cors_preflight() -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let mut response = Response::new(empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    let h = response.headers_mut();
    h.insert("Allow", HeaderValue::from_static("GET, OPTIONS"));
    h.insert("Access-Control-Allow-Origin", HeaderValue::from_static("*"));
    h.insert(
        "Access-Control-Allow-Headers",
        HeaderValue::from_static("Authorization, Content-Type"),
    );
    h.insert(
        "Access-Control-Allow-Methods",
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    h.insert("Content-Type", HeaderValue::from_static("application/json"));
    Ok(response)
}

// ---------------------------------------------------------------------------
// Configuration loading
// ---------------------------------------------------------------------------

/// Load and parse the YAML configuration file.
///
/// The path is read from `PLANO_CONFIG_PATH_RENDERED` (env) or falls back to
/// `./plano_config_rendered.yaml`.
fn load_config() -> Result<Configuration, Box<dyn std::error::Error + Send + Sync>> {
    let path = env::var("PLANO_CONFIG_PATH_RENDERED")
        .unwrap_or_else(|_| "./plano_config_rendered.yaml".to_string());
    eprintln!("loading plano_config.yaml from {}", path);

    let contents = fs::read_to_string(&path).map_err(|e| format!("failed to read {path}: {e}"))?;

    let config: Configuration =
        serde_yaml::from_str(&contents).map_err(|e| format!("failed to parse {path}: {e}"))?;

    Ok(config)
}

// ---------------------------------------------------------------------------
// Application state initialization
// ---------------------------------------------------------------------------

/// Build the shared [`AppState`] from a parsed [`Configuration`].
async fn init_app_state(
    config: &Configuration,
) -> Result<AppState, Box<dyn std::error::Error + Send + Sync>> {
    let llm_provider_url =
        env::var("LLM_PROVIDER_ENDPOINT").unwrap_or_else(|_| "http://localhost:12001".to_string());

    // Combine agents and filters into a single list
    let all_agents: Vec<Agent> = config
        .agents
        .as_deref()
        .unwrap_or_default()
        .iter()
        .chain(config.filters.as_deref().unwrap_or_default())
        .cloned()
        .collect();

    let global_agent_map: HashMap<String, Agent> = all_agents
        .iter()
        .map(|a| (a.id.clone(), a.clone()))
        .collect();

    let llm_providers = LlmProviders::try_from(config.model_providers.clone())
        .map_err(|e| format!("failed to create LlmProviders: {e}"))?;

    let model_listener_count = config
        .listeners
        .iter()
        .filter(|l| l.listener_type == ListenerType::Model)
        .count();
    if model_listener_count > 1 {
        return Err(format!(
            "only one model listener is allowed, found {}",
            model_listener_count
        )
        .into());
    }
    let model_listener = config
        .listeners
        .iter()
        .find(|l| l.listener_type == ListenerType::Model);
    let resolve_chain = |filter_ids: Option<Vec<String>>| -> Option<ResolvedFilterChain> {
        filter_ids.map(|ids| {
            let agents = ids
                .iter()
                .filter_map(|id| {
                    global_agent_map
                        .get(id)
                        .map(|a: &Agent| (id.clone(), a.clone()))
                })
                .collect();
            ResolvedFilterChain {
                filter_ids: ids,
                agents,
            }
        })
    };
    let filter_pipeline = Arc::new(FilterPipeline {
        input: resolve_chain(model_listener.and_then(|l| l.input_filters.clone())),
        output: resolve_chain(model_listener.and_then(|l| l.output_filters.clone())),
    });

    let overrides = config.overrides.clone().unwrap_or_default();

    let routing_model_name: String = overrides
        .llm_routing_model
        .as_deref()
        .map(|m| m.split_once('/').map(|(_, id)| id).unwrap_or(m))
        .unwrap_or(DEFAULT_ROUTING_MODEL_NAME)
        .to_string();

    let routing_llm_provider = config
        .model_providers
        .iter()
        .find(|p| p.model.as_deref() == Some(routing_model_name.as_str()))
        .map(|p| p.name.clone())
        .unwrap_or_else(|| DEFAULT_ROUTING_LLM_PROVIDER.to_string());

    let session_ttl_seconds = config.routing.as_ref().and_then(|r| r.session_ttl_seconds);
    let session_cache = init_session_cache(config).await?;

    // Validate that top-level routing_preferences requires v0.4.0+.
    let config_version = parse_semver(&config.version);
    let is_v040_plus = config_version >= (0, 4, 0);

    if !is_v040_plus && config.routing_preferences.is_some() {
        return Err(
            "top-level routing_preferences requires version v0.4.0 or above. \
             Update the version field or remove routing_preferences."
                .into(),
        );
    }

    // Validate that all models referenced in top-level routing_preferences exist in model_providers.
    // The CLI renders model_providers with `name` = "openai/gpt-4o" and `model` = "gpt-4o",
    // so we accept a match against either field.
    if let Some(ref route_prefs) = config.routing_preferences {
        let provider_model_names: std::collections::HashSet<&str> = config
            .model_providers
            .iter()
            .flat_map(|p| std::iter::once(p.name.as_str()).chain(p.model.as_deref()))
            .collect();
        for pref in route_prefs {
            for model in &pref.models {
                if !provider_model_names.contains(model.as_str()) {
                    return Err(format!(
                        "routing_preferences route '{}' references model '{}' \
                         which is not declared in model_providers",
                        pref.name, model
                    )
                    .into());
                }
            }
        }
    }

    // Validate and initialize ModelMetricsService if model_metrics_sources is configured.
    let metrics_service: Option<Arc<ModelMetricsService>> = if let Some(ref sources) =
        config.model_metrics_sources
    {
        use common::configuration::MetricsSource;
        let cost_count = sources
            .iter()
            .filter(|s| matches!(s, MetricsSource::Cost(_)))
            .count();
        let latency_count = sources
            .iter()
            .filter(|s| matches!(s, MetricsSource::Latency(_)))
            .count();
        if cost_count > 1 {
            return Err("model_metrics_sources: only one cost metrics source is allowed".into());
        }
        if latency_count > 1 {
            return Err("model_metrics_sources: only one latency metrics source is allowed".into());
        }
        let svc = ModelMetricsService::new(sources, reqwest::Client::new()).await;
        Some(Arc::new(svc))
    } else {
        None
    };

    // Validate that selection_policy.prefer is compatible with the configured metric sources.
    if let Some(ref prefs) = config.routing_preferences {
        use common::configuration::{MetricsSource, SelectionPreference};

        let has_cost_source = config
            .model_metrics_sources
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|s| matches!(s, MetricsSource::Cost(_)));
        let has_latency_source = config
            .model_metrics_sources
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|s| matches!(s, MetricsSource::Latency(_)));

        for pref in prefs {
            if pref.selection_policy.prefer == SelectionPreference::Cheapest && !has_cost_source {
                return Err(format!(
                    "routing_preferences route '{}' uses prefer: cheapest but no cost metrics source is configured — \
                     add a cost metrics source to model_metrics_sources",
                    pref.name
                )
                .into());
            }
            if pref.selection_policy.prefer == SelectionPreference::Fastest && !has_latency_source {
                return Err(format!(
                    "routing_preferences route '{}' uses prefer: fastest but no latency metrics source is configured — \
                     add a latency metrics source to model_metrics_sources",
                    pref.name
                )
                .into());
            }
        }
    }

    // Warn about models in routing_preferences that have no matching pricing/latency data.
    if let (Some(ref prefs), Some(ref svc)) = (&config.routing_preferences, &metrics_service) {
        let cost_data = svc.cost_snapshot().await;
        let latency_data = svc.latency_snapshot().await;
        for pref in prefs {
            use common::configuration::SelectionPreference;
            for model in &pref.models {
                let missing = match pref.selection_policy.prefer {
                    SelectionPreference::Cheapest => !cost_data.contains_key(model.as_str()),
                    SelectionPreference::Fastest => !latency_data.contains_key(model.as_str()),
                    _ => false,
                };
                if missing {
                    warn!(
                        model = %model,
                        route = %pref.name,
                        "model has no metric data — will be ranked last"
                    );
                }
            }
        }
    }

    let router_service = Arc::new(RouterService::new(
        config.routing_preferences.clone(),
        metrics_service,
        format!("{llm_provider_url}{CHAT_COMPLETIONS_PATH}"),
        routing_model_name,
        routing_llm_provider,
        session_ttl_seconds,
        session_cache,
    ));

    // Spawn background task to clean up expired session cache entries every 5 minutes
    {
        let router_service = Arc::clone(&router_service);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                router_service.cleanup_expired_sessions().await;
            }
        });
    }

    let orchestrator_model_name: String = overrides
        .agent_orchestration_model
        .as_deref()
        .map(|m| m.split_once('/').map(|(_, id)| id).unwrap_or(m))
        .unwrap_or(DEFAULT_ORCHESTRATOR_MODEL_NAME)
        .to_string();

    let orchestrator_llm_provider: String = config
        .model_providers
        .iter()
        .find(|p| p.model.as_deref() == Some(orchestrator_model_name.as_str()))
        .map(|p| p.name.clone())
        .unwrap_or_else(|| DEFAULT_ORCHESTRATOR_LLM_PROVIDER.to_string());

    let orchestrator_service = Arc::new(OrchestratorService::new(
        format!("{llm_provider_url}{CHAT_COMPLETIONS_PATH}"),
        orchestrator_model_name,
        orchestrator_llm_provider,
    ));

    let state_storage = init_state_storage(config).await?;

    let span_attributes = config
        .tracing
        .as_ref()
        .and_then(|tracing| tracing.span_attributes.clone());

    Ok(AppState {
        router_service,
        orchestrator_service,
        model_aliases: config.model_aliases.clone(),
        llm_providers: Arc::new(RwLock::new(llm_providers)),
        agents_list: Some(all_agents),
        listeners: config.listeners.clone(),
        state_storage,
        llm_provider_url,
        span_attributes,
        http_client: reqwest::Client::new(),
        filter_pipeline,
    })
}

/// Initialize the session cache backend based on config.
/// Defaults to the in-memory backend when no `session_cache` is configured.
async fn init_session_cache(
    config: &Configuration,
) -> Result<Arc<dyn SessionCache>, Box<dyn std::error::Error + Send + Sync>> {
    use common::configuration::SessionCacheType;
    use std::time::Duration;

    let session_ttl_seconds = config.routing.as_ref().and_then(|r| r.session_ttl_seconds);
    let session_max_entries = config.routing.as_ref().and_then(|r| r.session_max_entries);

    const DEFAULT_SESSION_TTL_SECONDS: u64 = 600;
    const DEFAULT_SESSION_MAX_ENTRIES: usize = 10_000;
    const MAX_SESSION_MAX_ENTRIES: usize = 10_000;

    let ttl = Duration::from_secs(session_ttl_seconds.unwrap_or(DEFAULT_SESSION_TTL_SECONDS));
    let max_entries = session_max_entries
        .unwrap_or(DEFAULT_SESSION_MAX_ENTRIES)
        .min(MAX_SESSION_MAX_ENTRIES);

    let cache_config = config
        .routing
        .as_ref()
        .and_then(|r| r.session_cache.as_ref());

    let cache_type = cache_config
        .map(|c| &c.cache_type)
        .unwrap_or(&SessionCacheType::Memory);

    match cache_type {
        SessionCacheType::Memory => {
            info!(storage_type = "memory", "initialized session cache");
            Ok(Arc::new(MemorySessionCache::new(ttl, max_entries)))
        }
        SessionCacheType::Redis => {
            use brightstaff::session_cache::redis::RedisSessionCache;
            let url = cache_config
                .and_then(|c| c.url.as_ref())
                .ok_or("session_cache.url is required when type is redis")?;
            info!(storage_type = "redis", url = %url, "initializing session cache");
            let cache = RedisSessionCache::new(url)
                .await
                .map_err(|e| format!("failed to connect to Redis session cache: {e}"))?;
            Ok(Arc::new(cache))
        }
    }
}

/// Initialize the conversation state storage backend (if configured).
async fn init_state_storage(
    config: &Configuration,
) -> Result<Option<Arc<dyn StateStorage>>, Box<dyn std::error::Error + Send + Sync>> {
    let Some(storage_config) = &config.state_storage else {
        info!("no state_storage configured, conversation state management disabled");
        return Ok(None);
    };

    let storage: Arc<dyn StateStorage> = match storage_config.storage_type {
        common::configuration::StateStorageType::Memory => {
            info!(
                storage_type = "memory",
                "initialized conversation state storage"
            );
            Arc::new(MemoryConversationalStorage::new())
        }
        common::configuration::StateStorageType::Postgres => {
            let connection_string = storage_config
                .connection_string
                .as_ref()
                .ok_or("connection_string is required for postgres state_storage")?;

            debug!(connection_string = %connection_string, "postgres connection");
            info!(
                storage_type = "postgres",
                "initializing conversation state storage"
            );

            Arc::new(
                PostgreSQLConversationStorage::new(connection_string.clone())
                    .await
                    .map_err(|e| format!("failed to initialize Postgres state storage: {e}"))?,
            )
        }
    };

    Ok(Some(storage))
}

// ---------------------------------------------------------------------------
// Request routing
// ---------------------------------------------------------------------------

/// Route an incoming HTTP request to the appropriate handler.
async fn route(
    req: Request<Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let parent_cx = global::get_text_map_propagator(|p| p.extract(&HeaderExtractor(req.headers())));
    let path = req.uri().path().to_string();

    // --- Agent routes (/agents/...) ---
    if let Some(stripped) = path.strip_prefix("/agents") {
        if matches!(
            stripped,
            CHAT_COMPLETIONS_PATH | MESSAGES_PATH | OPENAI_RESPONSES_API_PATH
        ) {
            return agent_chat(req, Arc::clone(&state))
                .with_context(parent_cx)
                .await;
        }
    }

    // --- Routing decision routes (/routing/...) ---
    if let Some(stripped) = path.strip_prefix("/routing") {
        let stripped = stripped.to_string();
        if matches!(
            stripped.as_str(),
            CHAT_COMPLETIONS_PATH | MESSAGES_PATH | OPENAI_RESPONSES_API_PATH
        ) {
            return routing_decision(
                req,
                Arc::clone(&state.router_service),
                stripped,
                &state.span_attributes,
            )
            .with_context(parent_cx)
            .await;
        }
    }

    // --- Standard routes ---
    match (req.method(), path.as_str()) {
        (&Method::POST, CHAT_COMPLETIONS_PATH | MESSAGES_PATH | OPENAI_RESPONSES_API_PATH) => {
            llm_chat(req, Arc::clone(&state))
                .with_context(parent_cx)
                .await
        }
        (&Method::POST, "/function_calling") => {
            let url = format!("{}/v1/chat/completions", state.llm_provider_url);
            function_calling_chat_handler(req, url)
                .with_context(parent_cx)
                .await
        }
        (&Method::GET, "/v1/models" | "/agents/v1/models") => {
            Ok(list_models(Arc::clone(&state.llm_providers)).await)
        }
        (&Method::OPTIONS, "/v1/models" | "/agents/v1/models") => cors_preflight(),
        _ => {
            debug!(method = %req.method(), path = %path, "no route found");
            let mut not_found = Response::new(empty());
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

// ---------------------------------------------------------------------------
// Server loop
// ---------------------------------------------------------------------------

/// Accept connections and spawn a task per connection.
///
/// Listens for `SIGINT` / `ctrl-c` and shuts down gracefully, allowing
/// in-flight connections to finish.
async fn run_server(state: Arc<AppState>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| BIND_ADDRESS.to_string());
    let listener = TcpListener::bind(&bind_address).await?;
    info!(address = %bind_address, "server listening");

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                let peer_addr = stream.peer_addr()?;
                let io = TokioIo::new(stream);
                let state = Arc::clone(&state);

                tokio::task::spawn(async move {
                    debug!(peer = ?peer_addr, "accepted connection");

                    let service = service_fn(move |req| {
                        let state = Arc::clone(&state);
                        async move { route(req, state).await }
                    });

                    if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                        warn!(error = ?err, "error serving connection");
                    }
                });
            }
            _ = &mut shutdown => {
                info!("received shutdown signal, stopping server");
                break;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = load_config()?;
    let _tracer_provider = init_tracer(config.tracing.as_ref());
    info!("loaded plano_config.yaml");
    let state = Arc::new(init_app_state(&config).await?);
    run_server(state).await
}
