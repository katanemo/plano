use brightstaff::app_state::AppState;
use brightstaff::handlers::agents::orchestrator::agent_chat;
use brightstaff::handlers::function_calling::function_calling_chat_handler;
use brightstaff::handlers::llm::llm_chat;
use brightstaff::handlers::models::list_models;
use brightstaff::router::llm::RouterService;
use brightstaff::router::orchestrator::OrchestratorService;
use brightstaff::state::memory::MemoryConversationalStorage;
use brightstaff::state::postgresql::PostgreSQLConversationStorage;
use brightstaff::state::StateStorage;
use brightstaff::tracing::init_tracer;
use bytes::Bytes;
use common::configuration::Configuration;
use common::consts::{
    CHAT_COMPLETIONS_PATH, MESSAGES_PATH, OPENAI_RESPONSES_API_PATH, PLANO_ORCHESTRATOR_MODEL_NAME,
};
use common::llm_providers::LlmProviders;
use http_body_util::{combinators::BoxBody, BodyExt, Empty};
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use opentelemetry::global;
use opentelemetry::trace::FutureExt;
use opentelemetry_http::HeaderExtractor;
use std::sync::Arc;
use std::{env, fs};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

const BIND_ADDRESS: &str = "0.0.0.0:9091";
const DEFAULT_ROUTING_LLM_PROVIDER: &str = "arch-router";
const DEFAULT_ROUTING_MODEL_NAME: &str = "Arch-Router";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// An empty HTTP body (used for 404 / OPTIONS responses).
fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
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
/// The path is read from `ARCH_CONFIG_PATH_RENDERED` (env) or falls back to
/// `./arch_config_rendered.yaml`.
fn load_config() -> Result<Configuration, Box<dyn std::error::Error + Send + Sync>> {
    let path = env::var("ARCH_CONFIG_PATH_RENDERED")
        .unwrap_or_else(|_| "./arch_config_rendered.yaml".to_string());
    eprintln!("loading arch_config.yaml from {}", path);

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
    let all_agents = config
        .agents
        .as_deref()
        .unwrap_or_default()
        .iter()
        .chain(config.filters.as_deref().unwrap_or_default())
        .cloned()
        .collect();

    let llm_providers = LlmProviders::try_from(config.model_providers.clone())
        .map_err(|e| format!("failed to create LlmProviders: {e}"))?;

    let routing_model_name = config
        .routing
        .as_ref()
        .and_then(|r| r.model.clone())
        .unwrap_or_else(|| DEFAULT_ROUTING_MODEL_NAME.to_string());

    let routing_llm_provider = config
        .routing
        .as_ref()
        .and_then(|r| r.model_provider.clone())
        .unwrap_or_else(|| DEFAULT_ROUTING_LLM_PROVIDER.to_string());

    let router_service = Arc::new(RouterService::new(
        config.model_providers.clone(),
        format!("{llm_provider_url}{CHAT_COMPLETIONS_PATH}"),
        routing_model_name,
        routing_llm_provider,
    ));

    let orchestrator_service = Arc::new(OrchestratorService::new(
        format!("{llm_provider_url}{CHAT_COMPLETIONS_PATH}"),
        PLANO_ORCHESTRATOR_MODEL_NAME.to_string(),
    ));

    let state_storage = init_state_storage(config).await?;

    Ok(AppState {
        router_service,
        orchestrator_service,
        model_aliases: Arc::new(config.model_aliases.clone()),
        llm_providers: Arc::new(RwLock::new(llm_providers)),
        agents_list: Arc::new(RwLock::new(Some(all_agents))),
        listeners: Arc::new(RwLock::new(config.listeners.clone())),
        state_storage,
        llm_provider_url,
    })
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
            return agent_chat(
                req,
                Arc::clone(&state.orchestrator_service),
                Arc::clone(&state.agents_list),
                Arc::clone(&state.listeners),
            )
            .with_context(parent_cx)
            .await;
        }
    }

    // --- Standard routes ---
    match (req.method(), path.as_str()) {
        (&Method::POST, CHAT_COMPLETIONS_PATH | MESSAGES_PATH | OPENAI_RESPONSES_API_PATH) => {
            let url = format!("{}{}", state.llm_provider_url, path);
            llm_chat(
                req,
                Arc::clone(&state.router_service),
                url,
                Arc::clone(&state.model_aliases),
                Arc::clone(&state.llm_providers),
                state.state_storage.clone(),
            )
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
    let state = Arc::new(init_app_state(&config).await?);
    run_server(state).await
}
