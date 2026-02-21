use crate::auth::cache::AuthCache;
use crate::auth::jwt::{create_token, validate_token};
use crate::auth::token_resolver::hash_token;
use crate::db::queries;
use crate::db::DbPool;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

// === Request/Response types ===

#[derive(Deserialize)]
struct CreateUserRequest {
    email: String,
    password: String,
    display_name: Option<String>,
}

#[derive(Serialize)]
struct UserResponse {
    id: Uuid,
    email: String,
    display_name: Option<String>,
    created_at: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct LoginResponse {
    token: String,
    user_id: Uuid,
}

#[derive(Deserialize)]
struct CreateProjectRequest {
    name: String,
    description: Option<String>,
}

#[derive(Serialize)]
struct ProjectResponse {
    id: Uuid,
    name: String,
    description: Option<String>,
    created_at: String,
}

#[derive(Deserialize)]
struct CreatePipeRequest {
    name: String,
    provider: String,
    api_key: String,
    model_filter: Option<String>,
}

#[derive(Serialize)]
struct PipeResponse {
    id: Uuid,
    name: String,
    provider: String,
    model_filter: Option<String>,
    created_at: String,
}

#[derive(Deserialize)]
struct CreateTokenRequest {
    name: String,
}

#[derive(Serialize)]
struct TokenResponse {
    id: Uuid,
    token: String, // returned only once at creation
    name: String,
}

#[derive(Deserialize)]
struct SpendingLimitRequest {
    entity_type: String,
    entity_id: Uuid,
    period_type: String,
    limit_cents: i64,
}

#[derive(Deserialize)]
struct RegisterApiKeyRequest {
    api_key: String,
    provider: String,
    upstream_url: String,
    display_name: Option<String>,
    egress_ip: Option<String>,
}

#[derive(Serialize)]
struct RegisteredApiKeyResponse {
    id: Uuid,
    provider: String,
    upstream_url: String,
    display_name: Option<String>,
    is_active: bool,
    created_at: String,
}

#[derive(Deserialize)]
struct CustomPricingRequest {
    project_id: Uuid,
    provider: String,
    model: String,
    input_price_per_million: f64,
    output_price_per_million: f64,
}

#[derive(Serialize)]
struct CustomPricingResponse {
    id: Uuid,
    project_id: Option<Uuid>,
    provider: String,
    model: String,
    input_price_per_million: f64,
    output_price_per_million: f64,
    created_at: String,
}

// === Route dispatcher ===

pub async fn handle_management(
    req: Request<Incoming>,
    path: &str,
    pool: &DbPool,
    jwt_secret: &str,
    auth_cache: &AuthCache,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    match (req.method().as_str(), path) {
        ("POST", "/api/v1/users") => handle_create_user(req, pool).await,
        ("POST", "/api/v1/auth/login") => handle_login(req, pool, jwt_secret).await,
        ("GET", "/api/v1/projects") => {
            with_jwt_auth(req, jwt_secret, |user_id, _req| async move {
                handle_list_projects(pool, user_id).await
            })
            .await
        }
        ("POST", "/api/v1/projects") => {
            with_jwt_auth(req, jwt_secret, |user_id, req| async move {
                handle_create_project(req, pool, user_id).await
            })
            .await
        }
        _ if path.starts_with("/api/v1/projects/") => {
            handle_project_sub_routes(req, path, pool, jwt_secret, auth_cache).await
        }
        ("GET", "/api/v1/spending-limits") | ("PUT", "/api/v1/spending-limits") => {
            with_jwt_auth(req, jwt_secret, |_user_id, req| async move {
                handle_spending_limits(req, pool).await
            })
            .await
        }
        ("POST", "/api/v1/pricing/custom") => {
            with_jwt_auth(req, jwt_secret, |_user_id, req| async move {
                handle_upsert_custom_pricing(req, pool).await
            })
            .await
        }
        ("GET", "/api/v1/pricing/custom") => {
            with_jwt_auth(req, jwt_secret, |_user_id, _req| async move {
                handle_list_custom_pricing(pool, None).await
            })
            .await
        }
        _ => Ok(not_found()),
    }
}

async fn handle_project_sub_routes(
    req: Request<Incoming>,
    path: &str,
    pool: &DbPool,
    jwt_secret: &str,
    auth_cache: &AuthCache,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    // Parse /api/v1/projects/:id/...
    let segments: Vec<&str> = path
        .trim_start_matches("/api/v1/projects/")
        .split('/')
        .collect();
    let project_id = match segments.first().and_then(|s| Uuid::parse_str(s).ok()) {
        Some(id) => id,
        None => {
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                "invalid project id",
            ))
        }
    };

    let sub_path = segments.get(1).copied().unwrap_or("");
    let sub_id = segments.get(2).and_then(|s| Uuid::parse_str(s).ok());

    match (req.method().as_str(), sub_path) {
        ("GET", "pipes") => {
            with_jwt_auth(req, jwt_secret, |_user_id, _req| async move {
                handle_list_pipes(pool, project_id).await
            })
            .await
        }
        ("POST", "pipes") => {
            with_jwt_auth(req, jwt_secret, |_user_id, req| async move {
                handle_create_pipe(req, pool, project_id).await
            })
            .await
        }
        ("POST", "tokens") => {
            with_jwt_auth(req, jwt_secret, |_user_id, req| async move {
                handle_create_token(req, pool, project_id).await
            })
            .await
        }
        ("DELETE", "tokens") if sub_id.is_some() => {
            let token_db_id = sub_id.unwrap();
            let cache = auth_cache.clone();
            with_jwt_auth(req, jwt_secret, |_user_id, _req| async move {
                handle_revoke_token(pool, project_id, token_db_id, &cache).await
            })
            .await
        }
        // Firewall mode: registered API keys
        ("GET", "api-keys") => {
            with_jwt_auth(req, jwt_secret, |_user_id, _req| async move {
                handle_list_api_keys(pool, project_id).await
            })
            .await
        }
        ("POST", "api-keys") => {
            with_jwt_auth(req, jwt_secret, |_user_id, req| async move {
                handle_register_api_key(req, pool, project_id).await
            })
            .await
        }
        ("DELETE", "api-keys") if sub_id.is_some() => {
            let key_id = sub_id.unwrap();
            with_jwt_auth(req, jwt_secret, |_user_id, _req| async move {
                handle_delete_api_key(pool, project_id, key_id).await
            })
            .await
        }
        // Per-project custom pricing
        ("GET", "pricing") => {
            with_jwt_auth(req, jwt_secret, |_user_id, _req| async move {
                handle_list_custom_pricing(pool, Some(project_id)).await
            })
            .await
        }
        _ => Ok(not_found()),
    }
}

// === Handler implementations ===

async fn handle_create_user(
    req: Request<Incoming>,
    pool: &DbPool,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let body = parse_body::<CreateUserRequest>(req).await;
    let body = match body {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    let password_hash = match bcrypt::hash(&body.password, bcrypt::DEFAULT_COST) {
        Ok(h) => h,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::create_user(
        &client,
        &body.email,
        &password_hash,
        body.display_name.as_deref(),
    )
    .await
    {
        Ok(user) => Ok(json_response(
            StatusCode::CREATED,
            &serde_json::json!(UserResponse {
                id: user.id,
                email: user.email,
                display_name: user.display_name,
                created_at: user.created_at.to_rfc3339(),
            }),
        )),
        Err(e) => Ok(error_response(StatusCode::CONFLICT, &e.to_string())),
    }
}

async fn handle_login(
    req: Request<Incoming>,
    pool: &DbPool,
    jwt_secret: &str,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let body = parse_body::<LoginRequest>(req).await;
    let body = match body {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    let user = match queries::get_user_by_email(&client, &body.email).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return Ok(error_response(
                StatusCode::UNAUTHORIZED,
                "invalid credentials",
            ))
        }
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    if !bcrypt::verify(&body.password, &user.password_hash).unwrap_or(false) {
        return Ok(error_response(
            StatusCode::UNAUTHORIZED,
            "invalid credentials",
        ));
    }

    match create_token(user.id, &user.email, jwt_secret) {
        Ok(token) => Ok(json_response(
            StatusCode::OK,
            &serde_json::json!(LoginResponse {
                token,
                user_id: user.id,
            }),
        )),
        Err(e) => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        )),
    }
}

async fn handle_list_projects(
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::list_projects(&client, user_id).await {
        Ok(projects) => {
            let resp: Vec<ProjectResponse> = projects
                .into_iter()
                .map(|p| ProjectResponse {
                    id: p.id,
                    name: p.name,
                    description: p.description,
                    created_at: p.created_at.to_rfc3339(),
                })
                .collect();
            Ok(json_response(StatusCode::OK, &serde_json::json!(resp)))
        }
        Err(e) => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        )),
    }
}

async fn handle_create_project(
    req: Request<Incoming>,
    pool: &DbPool,
    user_id: Uuid,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let body = parse_body::<CreateProjectRequest>(req).await;
    let body = match body {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::create_project(&client, user_id, &body.name, body.description.as_deref()).await {
        Ok(project) => Ok(json_response(
            StatusCode::CREATED,
            &serde_json::json!(ProjectResponse {
                id: project.id,
                name: project.name,
                description: project.description,
                created_at: project.created_at.to_rfc3339(),
            }),
        )),
        Err(e) => Ok(error_response(StatusCode::CONFLICT, &e.to_string())),
    }
}

async fn handle_list_pipes(
    pool: &DbPool,
    project_id: Uuid,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::list_pipes(&client, project_id).await {
        Ok(pipes) => {
            let resp: Vec<PipeResponse> = pipes
                .into_iter()
                .map(|p| PipeResponse {
                    id: p.id,
                    name: p.name,
                    provider: p.provider,
                    model_filter: p.model_filter,
                    created_at: p.created_at.to_rfc3339(),
                })
                .collect();
            Ok(json_response(StatusCode::OK, &serde_json::json!(resp)))
        }
        Err(e) => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        )),
    }
}

async fn handle_create_pipe(
    req: Request<Incoming>,
    pool: &DbPool,
    project_id: Uuid,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let body = parse_body::<CreatePipeRequest>(req).await;
    let body = match body {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    // TODO: encrypt api_key before storing
    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::create_pipe(
        &client,
        project_id,
        &body.name,
        &body.provider,
        &body.api_key,
        body.model_filter.as_deref(),
    )
    .await
    {
        Ok(pipe) => Ok(json_response(
            StatusCode::CREATED,
            &serde_json::json!(PipeResponse {
                id: pipe.id,
                name: pipe.name,
                provider: pipe.provider,
                model_filter: pipe.model_filter,
                created_at: pipe.created_at.to_rfc3339(),
            }),
        )),
        Err(e) => Ok(error_response(StatusCode::CONFLICT, &e.to_string())),
    }
}

async fn handle_create_token(
    req: Request<Incoming>,
    pool: &DbPool,
    project_id: Uuid,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let body = parse_body::<CreateTokenRequest>(req).await;
    let body = match body {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    // Generate a random token with xproxy_ prefix
    let raw_token = format!("xproxy_{}", Uuid::new_v4().to_string().replace('-', ""));
    let token_hash_value = hash_token(&raw_token);

    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::create_proxy_token(&client, project_id, &token_hash_value, &body.name).await {
        Ok(id) => Ok(json_response(
            StatusCode::CREATED,
            &serde_json::json!(TokenResponse {
                id,
                token: raw_token, // returned only once
                name: body.name,
            }),
        )),
        Err(e) => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        )),
    }
}

async fn handle_revoke_token(
    pool: &DbPool,
    project_id: Uuid,
    token_id: Uuid,
    _auth_cache: &AuthCache,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::revoke_proxy_token(&client, token_id, project_id).await {
        Ok(true) => {
            // We can't easily invalidate by token_id since cache is keyed by hash.
            // The TTL (60s) will handle eventual cleanup.
            Ok(json_response(
                StatusCode::OK,
                &serde_json::json!({ "status": "revoked" }),
            ))
        }
        Ok(false) => Ok(error_response(StatusCode::NOT_FOUND, "token not found")),
        Err(e) => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        )),
    }
}

async fn handle_spending_limits(
    req: Request<Incoming>,
    pool: &DbPool,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    if req.method() == "PUT" {
        let body = parse_body::<SpendingLimitRequest>(req).await;
        let body = match body {
            Ok(b) => b,
            Err(resp) => return Ok(resp),
        };

        let client = match pool.get_client().await {
            Ok(c) => c,
            Err(e) => {
                return Ok(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &e.to_string(),
                ))
            }
        };

        match queries::upsert_spending_limit(
            &client,
            &body.entity_type,
            body.entity_id,
            &body.period_type,
            body.limit_cents,
        )
        .await
        {
            Ok(limit) => Ok(json_response(StatusCode::OK, &serde_json::json!(limit))),
            Err(e) => Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            )),
        }
    } else {
        // GET - for now return empty array as we'd need entity params
        Ok(json_response(StatusCode::OK, &serde_json::json!([])))
    }
}

// === Firewall mode: API key registration ===

async fn handle_register_api_key(
    req: Request<Incoming>,
    pool: &DbPool,
    project_id: Uuid,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let body = parse_body::<RegisterApiKeyRequest>(req).await;
    let body = match body {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    let key_hash = hash_token(&body.api_key);

    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    let egress_ip = body.egress_ip.as_deref().unwrap_or("default");

    match queries::register_api_key(
        &client,
        project_id,
        &key_hash,
        &body.provider,
        &body.upstream_url,
        body.display_name.as_deref(),
        egress_ip,
    )
    .await
    {
        Ok(id) => Ok(json_response(
            StatusCode::CREATED,
            &serde_json::json!({
                "id": id,
                "project_id": project_id,
                "provider": body.provider,
                "upstream_url": body.upstream_url,
                "display_name": body.display_name,
                "egress_ip": egress_ip,
                "status": "registered"
            }),
        )),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("duplicate key") || msg.contains("unique") {
                Ok(error_response(
                    StatusCode::CONFLICT,
                    "API key already registered",
                ))
            } else {
                Ok(error_response(StatusCode::INTERNAL_SERVER_ERROR, &msg))
            }
        }
    }
}

async fn handle_list_api_keys(
    pool: &DbPool,
    project_id: Uuid,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::list_registered_api_keys(&client, project_id).await {
        Ok(keys) => {
            let resp: Vec<RegisteredApiKeyResponse> = keys
                .into_iter()
                .map(|k| RegisteredApiKeyResponse {
                    id: k.id,
                    provider: k.provider,
                    upstream_url: k.upstream_url,
                    display_name: k.display_name,
                    is_active: k.is_active,
                    created_at: k.created_at.to_rfc3339(),
                })
                .collect();
            Ok(json_response(StatusCode::OK, &serde_json::json!(resp)))
        }
        Err(e) => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        )),
    }
}

async fn handle_delete_api_key(
    pool: &DbPool,
    project_id: Uuid,
    key_id: Uuid,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::delete_registered_api_key(&client, key_id, project_id).await {
        Ok(true) => Ok(json_response(
            StatusCode::OK,
            &serde_json::json!({ "status": "deleted" }),
        )),
        Ok(false) => Ok(error_response(StatusCode::NOT_FOUND, "API key not found")),
        Err(e) => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        )),
    }
}

// === Custom pricing ===

async fn handle_upsert_custom_pricing(
    req: Request<Incoming>,
    pool: &DbPool,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let body = parse_body::<CustomPricingRequest>(req).await;
    let body = match body {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::upsert_custom_pricing(
        &client,
        body.project_id,
        &body.provider,
        &body.model,
        body.input_price_per_million,
        body.output_price_per_million,
    )
    .await
    {
        Ok(id) => Ok(json_response(
            StatusCode::OK,
            &serde_json::json!({
                "id": id,
                "project_id": body.project_id,
                "provider": body.provider,
                "model": body.model,
                "input_price_per_million": body.input_price_per_million,
                "output_price_per_million": body.output_price_per_million,
            }),
        )),
        Err(e) => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        )),
    }
}

async fn handle_list_custom_pricing(
    pool: &DbPool,
    project_id: Option<Uuid>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let client = match pool.get_client().await {
        Ok(c) => c,
        Err(e) => {
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &e.to_string(),
            ))
        }
    };

    match queries::list_custom_pricing(&client, project_id).await {
        Ok(rows) => {
            let resp: Vec<CustomPricingResponse> = rows
                .into_iter()
                .map(|r| CustomPricingResponse {
                    id: r.id,
                    project_id: r.project_id,
                    provider: r.provider,
                    model: r.model,
                    input_price_per_million: r.input_price_per_million,
                    output_price_per_million: r.output_price_per_million,
                    created_at: r.created_at.to_rfc3339(),
                })
                .collect();
            Ok(json_response(StatusCode::OK, &serde_json::json!(resp)))
        }
        Err(e) => Ok(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &e.to_string(),
        )),
    }
}

// === Helpers ===

async fn with_jwt_auth<F, Fut>(
    req: Request<Incoming>,
    jwt_secret: &str,
    handler: F,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error>
where
    F: FnOnce(Uuid, Request<Incoming>) -> Fut,
    Fut: std::future::Future<Output = Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error>>,
{
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = if let Some(stripped) = auth_header.strip_prefix("Bearer ") {
        stripped.trim()
    } else {
        return Ok(error_response(
            StatusCode::UNAUTHORIZED,
            "missing bearer token",
        ));
    };

    match validate_token(token, jwt_secret) {
        Ok(claims) => {
            let user_id = match Uuid::parse_str(&claims.sub) {
                Ok(id) => id,
                Err(_) => {
                    return Ok(error_response(
                        StatusCode::UNAUTHORIZED,
                        "invalid token claims",
                    ))
                }
            };
            handler(user_id, req).await
        }
        Err(e) => {
            warn!(error = %e, "JWT validation failed");
            Ok(error_response(StatusCode::UNAUTHORIZED, "invalid token"))
        }
    }
}

async fn parse_body<T: serde::de::DeserializeOwned>(
    req: Request<Incoming>,
) -> Result<T, Response<BoxBody<Bytes, hyper::Error>>> {
    let body_bytes = req
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();

    serde_json::from_slice::<T>(&body_bytes)
        .map_err(|e| error_response(StatusCode::BAD_REQUEST, &format!("invalid JSON: {}", e)))
}

fn error_response(status: StatusCode, message: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
    json_response(status, &serde_json::json!({ "error": message }))
}

fn json_response(
    status: StatusCode,
    body: &serde_json::Value,
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let body_bytes = serde_json::to_vec(body).unwrap_or_default();
    let mut response = Response::new(
        Full::new(Bytes::from(body_bytes))
            .map_err(|never| match never {})
            .boxed(),
    );
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    response
}

fn not_found() -> Response<BoxBody<Bytes, hyper::Error>> {
    error_response(StatusCode::NOT_FOUND, "not found")
}
