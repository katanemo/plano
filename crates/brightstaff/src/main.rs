use brightstaff::handlers::agent_chat_completions::agent_chat;
use brightstaff::handlers::function_calling::function_calling_chat_handler;
use brightstaff::handlers::llm::llm_chat;
use brightstaff::handlers::models::list_models;
use brightstaff::router::llm_router::RouterService;
use brightstaff::router::plano_orchestrator::OrchestratorService;
use brightstaff::state::memory::MemoryConversationalStorage;
use brightstaff::state::postgresql::PostgreSQLConversationStorage;
use brightstaff::state::StateStorage;
use brightstaff::utils::tracing::init_tracer;
use bytes::Bytes;
use common::configuration::{Agent, Configuration};
use common::consts::{
    CHAT_COMPLETIONS_PATH, MESSAGES_PATH, OPENAI_RESPONSES_API_PATH, PLANO_ORCHESTRATOR_MODEL_NAME,
};
use common::llm_providers::LlmProviders;
use http_body_util::{combinators::BoxBody, BodyExt, Empty};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use opentelemetry::trace::FutureExt;
use opentelemetry::{global, Context};
use opentelemetry_http::HeaderExtractor;
use std::sync::Arc;
use std::{env, fs};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

pub mod router;

const BIND_ADDRESS: &str = "0.0.0.0:9091";
const DEFAULT_ROUTING_LLM_PROVIDER: &str = "arch-router";
const DEFAULT_ROUTING_MODEL_NAME: &str = "Arch-Router";

// Utility function to extract the context from the incoming request headers
fn extract_context_from_request(req: &Request<Incoming>) -> Context {
    global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(req.headers()))
    })
}

fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| BIND_ADDRESS.to_string());

    // loading arch_config.yaml file (before tracing init so we can read tracing config)
    let arch_config_path = env::var("ARCH_CONFIG_PATH_RENDERED")
        .unwrap_or_else(|_| "./arch_config_rendered.yaml".to_string());
    eprintln!("loading arch_config.yaml from {}", arch_config_path);

    let config_contents =
        fs::read_to_string(&arch_config_path).expect("Failed to read arch_config.yaml");

    let config: Configuration =
        serde_yaml::from_str(&config_contents).expect("Failed to parse arch_config.yaml");

    // Initialize tracing using config.yaml tracing section
    let _tracer_provider = init_tracer(config.tracing.as_ref());
    info!(path = %arch_config_path, "loaded arch_config.yaml");

    let arch_config = Arc::new(config);

    // combine agents and filters into a single list of agents
    let all_agents: Vec<Agent> = arch_config
        .agents
        .as_deref()
        .unwrap_or_default()
        .iter()
        .chain(arch_config.filters.as_deref().unwrap_or_default())
        .cloned()
        .collect();

    // Create expanded provider list for /v1/models endpoint
    let llm_providers = LlmProviders::try_from(arch_config.model_providers.clone())
        .expect("Failed to create LlmProviders");
    let llm_providers = Arc::new(RwLock::new(llm_providers));
    let combined_agents_filters_list = Arc::new(RwLock::new(Some(all_agents)));
    let listeners = Arc::new(RwLock::new(arch_config.listeners.clone()));
    let llm_provider_url =
        env::var("LLM_PROVIDER_ENDPOINT").unwrap_or_else(|_| "http://localhost:12001".to_string());

    let listener = TcpListener::bind(bind_address).await?;
    let routing_model_name: String = arch_config
        .routing
        .as_ref()
        .and_then(|r| r.model.clone())
        .unwrap_or_else(|| DEFAULT_ROUTING_MODEL_NAME.to_string());

    let routing_llm_provider = arch_config
        .routing
        .as_ref()
        .and_then(|r| r.model_provider.clone())
        .unwrap_or_else(|| DEFAULT_ROUTING_LLM_PROVIDER.to_string());

    let router_service: Arc<RouterService> = Arc::new(RouterService::new(
        arch_config.model_providers.clone(),
        format!("{llm_provider_url}{CHAT_COMPLETIONS_PATH}"),
        routing_model_name,
        routing_llm_provider,
    ));

    let orchestrator_service: Arc<OrchestratorService> = Arc::new(OrchestratorService::new(
        format!("{llm_provider_url}{CHAT_COMPLETIONS_PATH}"),
        PLANO_ORCHESTRATOR_MODEL_NAME.to_string(),
    ));

    let model_aliases = Arc::new(arch_config.model_aliases.clone());

    // Initialize trace collector and start background flusher
    // Tracing is enabled if the tracing config is present in arch_config.yaml
    // Pass Some(true/false) to override, or None to use env var OTEL_TRACING_ENABLED
    // OpenTelemetry automatic instrumentation is configured in utils/tracing.rs

    // Initialize conversation state storage for v1/responses
    // Configurable via arch_config.yaml state_storage section
    // If not configured, state management is disabled
    // Environment variables are substituted by envsubst before config is read
    let state_storage: Option<Arc<dyn StateStorage>> =
        if let Some(storage_config) = &arch_config.state_storage {
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
                        .expect("connection_string is required for postgres state_storage");

                    debug!(connection_string = %connection_string, "postgres connection");
                    info!(
                        storage_type = "postgres",
                        "initializing conversation state storage"
                    );
                    Arc::new(
                        PostgreSQLConversationStorage::new(connection_string.clone())
                            .await
                            .expect("Failed to initialize Postgres state storage"),
                    )
                }
            };
            Some(storage)
        } else {
            info!("no state_storage configured, conversation state management disabled");
            None
        };

    loop {
        let (stream, _) = listener.accept().await?;
        let peer_addr = stream.peer_addr()?;
        let io = TokioIo::new(stream);

        let router_service: Arc<RouterService> = Arc::clone(&router_service);
        let orchestrator_service: Arc<OrchestratorService> = Arc::clone(&orchestrator_service);
        let model_aliases: Arc<
            Option<std::collections::HashMap<String, common::configuration::ModelAlias>>,
        > = Arc::clone(&model_aliases);
        let llm_provider_url = llm_provider_url.clone();

        let llm_providers = llm_providers.clone();
        let agents_list = combined_agents_filters_list.clone();
        let listeners = listeners.clone();
        let state_storage = state_storage.clone();
        let service = service_fn(move |req| {
            let router_service = Arc::clone(&router_service);
            let orchestrator_service = Arc::clone(&orchestrator_service);
            let parent_cx = extract_context_from_request(&req);
            let llm_provider_url = llm_provider_url.clone();
            let llm_providers = llm_providers.clone();
            let model_aliases = Arc::clone(&model_aliases);
            let agents_list = agents_list.clone();
            let listeners = listeners.clone();
            let state_storage = state_storage.clone();

            async move {
                let path = req.uri().path();
                // Check if path starts with /agents
                if path.starts_with("/agents") {
                    // Check if it matches one of the agent API paths
                    let stripped_path = path.strip_prefix("/agents").unwrap();
                    if matches!(
                        stripped_path,
                        CHAT_COMPLETIONS_PATH | MESSAGES_PATH | OPENAI_RESPONSES_API_PATH
                    ) {
                        let fully_qualified_url = format!("{}{}", llm_provider_url, stripped_path);
                        return agent_chat(
                            req,
                            orchestrator_service,
                            fully_qualified_url,
                            agents_list,
                            listeners,
                        )
                        .with_context(parent_cx)
                        .await;
                    }
                }
                match (req.method(), path) {
                    (
                        &Method::POST,
                        CHAT_COMPLETIONS_PATH | MESSAGES_PATH | OPENAI_RESPONSES_API_PATH,
                    ) => {
                        let fully_qualified_url = format!("{}{}", llm_provider_url, path);
                        llm_chat(
                            req,
                            router_service,
                            fully_qualified_url,
                            model_aliases,
                            llm_providers,
                            state_storage,
                        )
                        .with_context(parent_cx)
                        .await
                    }
                    (&Method::POST, "/function_calling") => {
                        let fully_qualified_url =
                            format!("{}{}", llm_provider_url, "/v1/chat/completions");
                        function_calling_chat_handler(req, fully_qualified_url)
                            .with_context(parent_cx)
                            .await
                    }
                    (&Method::GET, "/v1/models" | "/agents/v1/models") => {
                        Ok(list_models(llm_providers).await)
                    }
                    // hack for now to get openw-web-ui to work
                    (&Method::OPTIONS, "/v1/models" | "/agents/v1/models") => {
                        let mut response = Response::new(empty());
                        *response.status_mut() = StatusCode::NO_CONTENT;
                        response
                            .headers_mut()
                            .insert("Allow", "GET, OPTIONS".parse().unwrap());
                        response
                            .headers_mut()
                            .insert("Access-Control-Allow-Origin", "*".parse().unwrap());
                        response.headers_mut().insert(
                            "Access-Control-Allow-Headers",
                            "Authorization, Content-Type".parse().unwrap(),
                        );
                        response.headers_mut().insert(
                            "Access-Control-Allow-Methods",
                            "GET, POST, OPTIONS".parse().unwrap(),
                        );
                        response
                            .headers_mut()
                            .insert("Content-Type", "application/json".parse().unwrap());

                        Ok(response)
                    }
                    _ => {
                        debug!(method = %req.method(), path = %req.uri().path(), "no route found");
                        let mut not_found = Response::new(empty());
                        *not_found.status_mut() = StatusCode::NOT_FOUND;
                        Ok(not_found)
                    }
                }
            }
        });

        tokio::task::spawn(async move {
            debug!(peer = ?peer_addr, "accepted connection");
            if let Err(err) = http1::Builder::new()
                // .serve_connection(io, service_fn(chat_completion))
                .serve_connection(io, service)
                .await
            {
                warn!(error = ?err, "error serving connection");
            }
        });
    }
}
