use brightstaff::auth::cache::AuthCache;
use brightstaff::billing::budget_checker::BudgetChecker;
use brightstaff::billing::counters::SpendingCounters;
use brightstaff::billing::flusher::UsageFlusher;
use brightstaff::billing::price_calculator::PriceCalculator;
use brightstaff::db::DbPool;
use brightstaff::handlers::agent_chat_completions::agent_chat;
use brightstaff::handlers::auth_check::handle_auth_check;
use brightstaff::handlers::budget_blocked::handle_budget_blocked;
use brightstaff::handlers::function_calling::function_calling_chat_handler;
use brightstaff::handlers::llm::llm_chat;
use brightstaff::handlers::management::handle_management;
use brightstaff::handlers::models::list_models;
use brightstaff::handlers::usage_record::handle_usage_record;
use brightstaff::pricing::PricingRegistry;
use brightstaff::registry::ApiKeyRegistry;
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

/// Shared application state for xproxy
struct AppState {
    db_pool: Option<DbPool>,
    auth_cache: AuthCache,
    counters: SpendingCounters,
    pricing: PricingRegistry,
    usage_tx: tokio::sync::mpsc::Sender<brightstaff::billing::flusher::UsageEvent>,
    jwt_secret: String,
    api_key_registry: ApiKeyRegistry,
    budget_checker: BudgetChecker,
}

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

    // loading plano_config.yaml file (before tracing init so we can read tracing config)
    let plano_config_path = env::var("PLANO_CONFIG_PATH_RENDERED")
        .unwrap_or_else(|_| "./plano_config_rendered.yaml".to_string());
    eprintln!("loading plano_config.yaml from {}", plano_config_path);

    let config_contents =
        fs::read_to_string(&plano_config_path).expect("Failed to read plano_config.yaml");

    let config: Configuration =
        serde_yaml::from_str(&config_contents).expect("Failed to parse plano_config.yaml");

    // Initialize tracing using config.yaml tracing section
    let _tracer_provider = init_tracer(config.tracing.as_ref());
    info!(path = %plano_config_path, "loaded plano_config.yaml");

    let plano_config = Arc::new(config);

    // Initialize xproxy DB pool (optional - only if DATABASE_URL is set)
    let db_pool = match env::var("DATABASE_URL") {
        Ok(url) => match DbPool::new(&url) {
            Ok(pool) => {
                info!("xproxy database pool initialized");
                Some(pool)
            }
            Err(e) => {
                warn!(error = %e, "failed to create xproxy database pool, running without xproxy features");
                None
            }
        },
        Err(_) => {
            info!("DATABASE_URL not set, xproxy features disabled");
            None
        }
    };

    // Initialize xproxy components
    let auth_cache = AuthCache::new();
    let counters = SpendingCounters::new();
    let pricing = PricingRegistry::new();
    let api_key_registry = ApiKeyRegistry::new();
    let budget_checker = BudgetChecker::new();

    // Load pricing data from Portkey if available
    let portkey_dir = env::var("PORTKEY_PRICING_DIR")
        .unwrap_or_else(|_| "pricing/portkey-models/pricing".to_string());
    if std::path::Path::new(&portkey_dir).exists() {
        match pricing.load_from_portkey_dir(&portkey_dir).await {
            Ok(count) => info!(models = count, "loaded portkey pricing data"),
            Err(e) => warn!(error = %e, "failed to load portkey pricing data"),
        }
    }

    // Hydrate spending counters from DB
    if let Some(ref pool) = db_pool {
        match pool.get_client().await {
            Ok(client) => match brightstaff::db::queries::load_current_counters(&client).await {
                Ok(records) => {
                    let hydrate_data: Vec<_> = records
                        .iter()
                        .map(|r| {
                            (
                                r.entity_type.clone(),
                                r.entity_id,
                                r.period_type.clone(),
                                r.period_start,
                                r.spent_micro_cents,
                            )
                        })
                        .collect();
                    counters.hydrate(&hydrate_data);
                    info!(
                        records = hydrate_data.len(),
                        "hydrated spending counters from DB"
                    );
                }
                Err(e) => warn!(error = %e, "failed to load spending counters from DB"),
            },
            Err(e) => warn!(error = %e, "failed to get DB client for counter hydration"),
        }

        // Initial load of API key registry
        match api_key_registry.reload(pool).await {
            Ok(count) => info!(keys = count, "loaded API key registry"),
            Err(e) => warn!(error = %e, "failed to load API key registry"),
        }

        // Start API key registry refresh (every 60s)
        api_key_registry
            .clone()
            .start_refresh_task(pool.clone(), 60);

        // Start background price calculator (every 10s)
        PriceCalculator::start(pool.clone(), pricing.clone(), counters.clone(), 10);

        // Start background budget checker (every 10s)
        budget_checker.clone().start(pool.clone(), 10);
    }

    // Start usage flusher
    let flusher = if let Some(ref pool) = db_pool {
        let f = UsageFlusher::start(pool.clone(), counters.clone(), 10);
        Some(f)
    } else {
        None
    };

    let usage_tx = flusher.as_ref().map(|f| f.sender()).unwrap_or_else(|| {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        tx
    });

    let jwt_secret =
        env::var("JWT_SECRET").unwrap_or_else(|_| "xproxy-dev-secret-change-me".to_string());

    let app_state = Arc::new(AppState {
        db_pool,
        auth_cache,
        counters,
        pricing,
        usage_tx,
        jwt_secret,
        api_key_registry,
        budget_checker,
    });

    // combine agents and filters into a single list of agents
    let all_agents: Vec<Agent> = plano_config
        .agents
        .as_deref()
        .unwrap_or_default()
        .iter()
        .chain(plano_config.filters.as_deref().unwrap_or_default())
        .cloned()
        .collect();

    // Create expanded provider list for /v1/models endpoint
    let llm_providers = LlmProviders::try_from(plano_config.model_providers.clone())
        .expect("Failed to create LlmProviders");
    let llm_providers = Arc::new(RwLock::new(llm_providers));
    let combined_agents_filters_list = Arc::new(RwLock::new(Some(all_agents)));
    let listeners = Arc::new(RwLock::new(plano_config.listeners.clone()));
    let llm_provider_url =
        env::var("LLM_PROVIDER_ENDPOINT").unwrap_or_else(|_| "http://localhost:12001".to_string());

    let listener = TcpListener::bind(bind_address).await?;
    let routing_model_name: String = plano_config
        .routing
        .as_ref()
        .and_then(|r| r.model.clone())
        .unwrap_or_else(|| DEFAULT_ROUTING_MODEL_NAME.to_string());

    let routing_llm_provider = plano_config
        .routing
        .as_ref()
        .and_then(|r| r.model_provider.clone())
        .unwrap_or_else(|| DEFAULT_ROUTING_LLM_PROVIDER.to_string());

    let router_service: Arc<RouterService> = Arc::new(RouterService::new(
        plano_config.model_providers.clone(),
        format!("{llm_provider_url}{CHAT_COMPLETIONS_PATH}"),
        routing_model_name,
        routing_llm_provider,
    ));

    let orchestrator_service: Arc<OrchestratorService> = Arc::new(OrchestratorService::new(
        format!("{llm_provider_url}{CHAT_COMPLETIONS_PATH}"),
        PLANO_ORCHESTRATOR_MODEL_NAME.to_string(),
    ));

    let model_aliases = Arc::new(plano_config.model_aliases.clone());

    // Initialize conversation state storage for v1/responses
    let state_storage: Option<Arc<dyn StateStorage>> =
        if let Some(storage_config) = &plano_config.state_storage {
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
        let app_state = app_state.clone();

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
            let app_state = app_state.clone();

            async move {
                let path = req.uri().path().to_string();

                // xproxy auth check endpoint
                if path == "/auth/check" && req.method() == Method::POST {
                    if let Some(ref pool) = app_state.db_pool {
                        return handle_auth_check(
                            req,
                            pool,
                            &app_state.auth_cache,
                            &app_state.counters,
                            &app_state.api_key_registry,
                        )
                        .await;
                    } else {
                        let mut resp = Response::new(empty());
                        *resp.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
                        return Ok(resp);
                    }
                }

                // xproxy usage recording endpoint
                if path == "/usage/record" && req.method() == Method::POST {
                    return handle_usage_record(
                        req,
                        &app_state.pricing,
                        &app_state.counters,
                        &app_state.usage_tx,
                    )
                    .await;
                }

                // Budget blocked endpoint (for WASM filter polling)
                if path == "/budget/blocked" && req.method() == Method::GET {
                    return handle_budget_blocked(&app_state.budget_checker).await;
                }

                // xproxy management API
                if path.starts_with("/api/v1/") {
                    if let Some(ref pool) = app_state.db_pool {
                        return handle_management(
                            req,
                            &path,
                            pool,
                            &app_state.jwt_secret,
                            &app_state.auth_cache,
                        )
                        .await;
                    } else {
                        let mut resp = Response::new(empty());
                        *resp.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
                        return Ok(resp);
                    }
                }

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
                            llm_providers,
                        )
                        .with_context(parent_cx)
                        .await;
                    }
                }
                match (req.method(), path.as_str()) {
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
                        debug!(method = %req.method(), path = %path, "no route found");
                        let mut not_found = Response::new(empty());
                        *not_found.status_mut() = StatusCode::NOT_FOUND;
                        Ok(not_found)
                    }
                }
            }
        });

        tokio::task::spawn(async move {
            debug!(peer = ?peer_addr, "accepted connection");
            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                warn!(error = ?err, "error serving connection");
            }
        });
    }
}
