use crate::auth::cache::AuthCache;
use crate::auth::pipe_selector::select_pipe;
use crate::auth::token_resolver::{hash_token, AuthError};
use crate::billing::counters::SpendingCounters;
use crate::db::queries::get_spending_limits;
use crate::db::DbPool;
use crate::registry::ApiKeyRegistry;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use serde::Deserialize;
use tracing::{debug, warn};

#[derive(Debug, Deserialize)]
struct ChatRequestBody {
    model: Option<String>,
}

/// Handle ext_authz check: POST /auth/check
/// Supports two modes:
/// - Default (managed proxy): validates xproxy_ token, selects pipe, checks budget
/// - Firewall mode: hashes the real API key, looks up in registry, returns upstream URL
pub async fn handle_auth_check(
    req: Request<Incoming>,
    pool: &DbPool,
    auth_cache: &AuthCache,
    counters: &SpendingCounters,
    api_key_registry: &ApiKeyRegistry,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    // Check if this is a firewall-mode request
    let is_firewall = req
        .headers()
        .get("x-xproxy-mode")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "firewall")
        .unwrap_or(false);

    if is_firewall {
        return handle_firewall_auth_check(req, api_key_registry).await;
    }

    // === Default managed proxy mode ===

    // Extract bearer token from Authorization header
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

    let token_hash = hash_token(token);

    // Resolve token -> auth context (cached)
    let auth_ctx = match auth_cache.get_or_resolve(pool, &token_hash, token).await {
        Ok(ctx) => ctx,
        Err(AuthError::InvalidToken) => {
            return Ok(error_response(
                StatusCode::UNAUTHORIZED,
                "invalid or expired token",
            ));
        }
        Err(e) => {
            warn!(error = %e, "auth check error");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("auth error: {}", e),
            ));
        }
    };

    // Parse model from request body
    let body_bytes = req
        .collect()
        .await
        .map(|b| b.to_bytes())
        .unwrap_or_default();
    let model = serde_json::from_slice::<ChatRequestBody>(&body_bytes)
        .ok()
        .and_then(|b| b.model);

    let model = match model {
        Some(m) => m,
        None => {
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                "model field required in request body",
            ));
        }
    };

    // Select pipe for the model
    let selected_pipe = match select_pipe(&auth_ctx, &model) {
        Ok(pipe) => pipe,
        Err(e) => {
            return Ok(error_response(StatusCode::FORBIDDEN, &e.to_string()));
        }
    };

    // Check spending limits
    if let Err(e) = check_budget(pool, counters, &auth_ctx.user_id, &auth_ctx.project_id).await {
        return Ok(json_response(
            StatusCode::TOO_MANY_REQUESTS,
            &serde_json::json!({
                "error": "spending_limit_exceeded",
                "message": e,
            }),
        ));
    }

    debug!(
        user_id = %auth_ctx.user_id,
        project_id = %auth_ctx.project_id,
        pipe_id = %selected_pipe.pipe_id,
        model = %model,
        "auth check passed"
    );

    // Build response with xproxy headers
    let mut response = Response::new(full_body("OK"));
    *response.status_mut() = StatusCode::OK;

    let headers = response.headers_mut();
    headers.insert(
        "x-xproxy-provider-hint",
        selected_pipe.provider.parse().unwrap(),
    );
    headers.insert(
        "x-xproxy-api-key",
        selected_pipe.api_key_decrypted.parse().unwrap(),
    );
    headers.insert("x-xproxy-model", model.parse().unwrap());
    headers.insert(
        "x-xproxy-user-id",
        auth_ctx.user_id.to_string().parse().unwrap(),
    );
    headers.insert(
        "x-xproxy-project-id",
        auth_ctx.project_id.to_string().parse().unwrap(),
    );
    headers.insert(
        "x-xproxy-pipe-id",
        selected_pipe.pipe_id.to_string().parse().unwrap(),
    );

    Ok(response)
}

/// Firewall mode auth: hash the real API key, look up in registry, return upstream info
async fn handle_firewall_auth_check(
    req: Request<Incoming>,
    registry: &ApiKeyRegistry,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    // Extract the real API key from Authorization header
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let api_key = if let Some(stripped) = auth_header.strip_prefix("Bearer ") {
        stripped.trim()
    } else {
        return Ok(error_response(
            StatusCode::UNAUTHORIZED,
            "missing bearer token (API key required for firewall mode)",
        ));
    };

    let key_hash = hash_token(api_key);

    // Look up in the in-memory registry
    let key_info = match registry.lookup(&key_hash).await {
        Some(info) => info,
        None => {
            return Ok(error_response(
                StatusCode::UNAUTHORIZED,
                "API key not registered. Register your API key at the xproxy dashboard.",
            ));
        }
    };

    debug!(
        project_id = %key_info.project_id,
        provider = %key_info.provider,
        upstream_url = %key_info.upstream_url,
        "firewall auth check passed"
    );

    // Build cluster name: append egress suffix if not "default"
    let cluster_name = if key_info.egress_ip == "default" {
        key_info.provider.clone()
    } else {
        format!("{}-{}", key_info.provider, key_info.egress_ip)
    };

    // Return headers that tell Envoy/WASM where to route and how to identify
    let mut response = Response::new(full_body("OK"));
    *response.status_mut() = StatusCode::OK;

    let headers = response.headers_mut();
    headers.insert("x-xproxy-firewall-mode", "true".parse().unwrap());
    headers.insert(
        "x-xproxy-upstream-url",
        key_info.upstream_url.parse().unwrap(),
    );
    headers.insert(
        "x-xproxy-project-id",
        key_info.project_id.to_string().parse().unwrap(),
    );
    headers.insert("x-xproxy-provider-hint", cluster_name.parse().unwrap());
    headers.insert("x-xproxy-api-key-hash", key_hash.parse().unwrap());

    Ok(response)
}

async fn check_budget(
    pool: &DbPool,
    counters: &SpendingCounters,
    user_id: &uuid::Uuid,
    project_id: &uuid::Uuid,
) -> Result<(), String> {
    let client = pool
        .get_client()
        .await
        .map_err(|e| format!("db error: {}", e))?;

    // Check user limits
    let user_limits = get_spending_limits(&client, "user", *user_id)
        .await
        .unwrap_or_default();

    for limit in &user_limits {
        let limit_micro_cents = limit.limit_cents * 10_000; // cents -> micro-cents
        if !counters.check_budget("user", *user_id, &limit.period_type, limit_micro_cents) {
            return Err(format!(
                "user {} limit exceeded for {} period",
                limit.period_type, limit.period_type
            ));
        }
    }

    // Check project limits
    let project_limits = get_spending_limits(&client, "project", *project_id)
        .await
        .unwrap_or_default();

    for limit in &project_limits {
        let limit_micro_cents = limit.limit_cents * 10_000;
        if !counters.check_budget(
            "project",
            *project_id,
            &limit.period_type,
            limit_micro_cents,
        ) {
            return Err(format!(
                "project {} limit exceeded for {} period",
                limit.period_type, limit.period_type
            ));
        }
    }

    Ok(())
}

fn error_response(status: StatusCode, message: &str) -> Response<BoxBody<Bytes, hyper::Error>> {
    let body = serde_json::json!({ "error": message });
    json_response(status, &body)
}

fn json_response(
    status: StatusCode,
    body: &serde_json::Value,
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let body_bytes = serde_json::to_vec(body).unwrap_or_default();
    let mut response = Response::new(full_body_bytes(body_bytes));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    response
}

fn full_body(s: &str) -> BoxBody<Bytes, hyper::Error> {
    Full::new(Bytes::from(s.to_string()))
        .map_err(|never| match never {})
        .boxed()
}

fn full_body_bytes(bytes: Vec<u8>) -> BoxBody<Bytes, hyper::Error> {
    Full::new(Bytes::from(bytes))
        .map_err(|never| match never {})
        .boxed()
}
