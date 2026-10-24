//! `POST /v1/embeddings`: an OpenAI-shaped embeddings passthrough.
//!
//! Embeddings do not go through the chat pipeline. The WASM `llm_gateway` filter
//! and hermesllm only model chat APIs, so this handler calls the provider directly
//! (reqwest) instead of going back through `llm_provider_url`. There is no routing:
//! the model named by the caller (after `model_aliases`) is the model that runs.
//!
//! The pure core is [`build_upstream_request`] (parsed body + providers + aliases in,
//! upstream url/headers/body or an error out) and [`prompt_tokens_from_response`];
//! [`embeddings`] is the thin network shell around them.

use bytes::Bytes;
use common::configuration::{LlmProvider, LlmProviderType, ModelAlias};
use common::llm_providers::LlmProviders;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use hyper::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use hyper::{Request, Response, StatusCode};
use opentelemetry::trace::get_active_span;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info_span, warn, Instrument};

use crate::app_state::AppState;
use crate::handlers::llm::resolve_model_alias;
use crate::handlers::{extract_request_id, full, take_attribution_from_headers, Attribution};
use crate::metrics as bs_metrics;
use crate::metrics::labels as metric_labels;
use crate::tracing::{llm as tracing_llm, operation_component, set_service_name};

/// Same `api-version` the chat path uses for Azure OpenAI (hermesllm `endpoints.rs`).
const AZURE_API_VERSION: &str = "2025-01-01-preview";
const AZURE_DEPLOYMENTS_PREFIX: &str = "openai/deployments";
const DEFAULT_OPENAI_HOST: &str = "api.openai.com";

/// An error to hand straight back to the client as an OpenAI-style JSON error.
#[derive(Debug)]
pub struct EmbeddingsError {
    pub status: StatusCode,
    pub body: Value,
}

impl EmbeddingsError {
    fn new(status: StatusCode, message: String, param: Option<&str>, code: Option<&str>) -> Self {
        Self {
            status,
            body: json!({
                "error": {
                    "message": message,
                    "type": "invalid_request_error",
                    "param": param,
                    "code": code,
                }
            }),
        }
    }

    fn into_response(self) -> Response<BoxBody<Bytes, hyper::Error>> {
        json_response(self.status, self.body.to_string())
    }
}

/// Everything needed to send the request upstream, and the labels to meter it under.
#[derive(Debug)]
pub struct UpstreamRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Bytes,
    /// `provider` metric label.
    pub provider_label: String,
    /// `model` metric label (the provider's bare model name).
    pub model_label: String,
}

fn json_response(
    status: StatusCode,
    body: impl Into<Bytes>,
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let mut response = Response::new(full(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

/// Build the upstream URL for an embeddings call, mirroring the conventions of
/// hermesllm's `target_endpoint_for_provider` (which only covers chat endpoints).
///
/// The Rust side of the config only carries `endpoint` (host), `port` and
/// `base_url_path_prefix` (the CLI splits `base_url` into them and drops the scheme),
/// so the scheme is inferred from the port the CLI defaulted: 443 (or unset) is
/// https, anything else is http.
pub fn upstream_url(provider: &LlmProvider, bare_model: &str) -> Result<String, String> {
    let host = match (provider.endpoint.as_deref(), &provider.provider_interface) {
        (Some(h), _) if !h.is_empty() => h,
        (_, LlmProviderType::OpenAI) => DEFAULT_OPENAI_HOST,
        _ => {
            return Err(format!(
                "Provider '{}' has no endpoint configured for embeddings",
                provider.name
            ))
        }
    };
    let (scheme, authority) = match provider.port {
        None | Some(443) => ("https", host.to_string()),
        Some(80) => ("http", host.to_string()),
        Some(port) => ("http", format!("{host}:{port}")),
    };
    let prefix = provider
        .base_url_path_prefix
        .as_deref()
        .map(|p| p.trim_matches('/'))
        .filter(|p| !p.is_empty());

    let path = if provider.provider_interface == LlmProviderType::AzureOpenAI {
        format!(
            "/{}/{}/embeddings?api-version={}",
            prefix.unwrap_or(AZURE_DEPLOYMENTS_PREFIX),
            bare_model,
            AZURE_API_VERSION
        )
    } else {
        format!("/{}/embeddings", prefix.unwrap_or("v1"))
    };
    Ok(format!("{scheme}://{authority}{path}"))
}

/// Headers sent to the provider: content-type, the provider's own credential, and the
/// provider's configured extra headers. Nothing from the client's request is copied,
/// so neither the attribution header nor the client's `Authorization` can leak.
fn upstream_headers(provider: &LlmProvider) -> Result<HeaderMap, EmbeddingsError> {
    let Some(key) = provider.access_key.as_deref().filter(|k| !k.is_empty()) else {
        return Err(EmbeddingsError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "No access key configured for provider '{}' (passthrough_auth is not supported for embeddings)",
                provider.name
            ),
            None,
            None,
        ));
    };
    let bad_key = || {
        EmbeddingsError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Invalid access key for provider '{}'", provider.name),
            None,
            None,
        )
    };

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    if provider.provider_interface == LlmProviderType::AzureOpenAI {
        let mut value = HeaderValue::from_str(key).map_err(|_| bad_key())?;
        value.set_sensitive(true);
        headers.insert(HeaderName::from_static("api-key"), value);
    } else {
        let mut value = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| bad_key())?;
        value.set_sensitive(true);
        headers.insert(hyper::header::AUTHORIZATION, value);
    }
    if let Some(extra) = &provider.headers {
        for (name, value) in extra {
            match (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                (Ok(n), Ok(v)) => {
                    headers.insert(n, v);
                }
                _ => warn!(header = %name, "skipping invalid configured provider header"),
            }
        }
    }
    Ok(headers)
}

/// Parse the client body, resolve its model against aliases and providers, and build
/// the upstream request. Pure: no network, no globals.
pub fn build_upstream_request(
    body: &[u8],
    providers: &LlmProviders,
    aliases: &Option<HashMap<String, ModelAlias>>,
) -> Result<UpstreamRequest, EmbeddingsError> {
    let mut payload: Value = serde_json::from_slice(body).map_err(|e| {
        EmbeddingsError::new(
            StatusCode::BAD_REQUEST,
            format!("Invalid JSON body: {e}"),
            None,
            None,
        )
    })?;
    let requested = payload
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            EmbeddingsError::new(
                StatusCode::BAD_REQUEST,
                "Missing required parameter: 'model' must be a string".to_string(),
                Some("model"),
                None,
            )
        })?;

    let resolved = resolve_model_alias(&requested, aliases);
    let provider = providers.get(&resolved).ok_or_else(|| {
        warn!(model = %resolved, "embeddings model not found in configured providers");
        EmbeddingsError::new(
            StatusCode::NOT_FOUND,
            format!("The model '{resolved}' does not exist or is not configured"),
            Some("model"),
            Some("model_not_found"),
        )
    })?;

    let bare_model = provider.model.clone().unwrap_or_else(|| {
        resolved
            .split_once('/')
            .map(|(_, m)| m.to_string())
            .unwrap_or_else(|| resolved.clone())
    });

    let url = upstream_url(&provider, &bare_model)
        .map_err(|m| EmbeddingsError::new(StatusCode::INTERNAL_SERVER_ERROR, m, None, None))?;
    let headers = upstream_headers(&provider)?;

    payload["model"] = Value::String(bare_model.clone());
    let body = Bytes::from(payload.to_string());

    // Provider label: same source as chat (`split_provider_model` on the resolved
    // model id), falling back to the provider interface when the caller sent a bare
    // model name. Chat reports "unknown" in that case (see the report on KAN-533).
    let (prefix_provider, _) = bs_metrics::split_provider_model(&resolved);
    let provider_label = if prefix_provider == "unknown" {
        provider.provider_interface.to_string()
    } else {
        prefix_provider.to_string()
    };

    Ok(UpstreamRequest {
        url,
        headers,
        body,
        provider_label,
        model_label: bare_model,
    })
}

/// `usage.prompt_tokens` from an OpenAI-shaped embeddings response.
pub fn prompt_tokens_from_response(body: &[u8]) -> Option<u64> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("usage")?
        .get("prompt_tokens")?
        .as_u64()
}

/// Meter a successful embeddings response: prompt tokens, or the usage-missing counter.
/// With a usage log, a response that reports prompt tokens also writes one usage line.
pub fn record_embeddings_usage(
    provider: &str,
    model: &str,
    attribution: &Attribution,
    usage_log: Option<&crate::usage_log::UsageLog>,
    response_body: &[u8],
) {
    match prompt_tokens_from_response(response_body) {
        Some(n) => {
            bs_metrics::record_llm_tokens(
                provider,
                model,
                &attribution.org,
                &attribution.service,
                metric_labels::TOKEN_KIND_PROMPT,
                n,
            );
            if let Some(log) = usage_log {
                log.record(
                    crate::usage_log::ENDPOINT_EMBEDDINGS,
                    attribution,
                    provider,
                    model,
                    &crate::usage_log::TokenCounts {
                        prompt: Some(n),
                        ..Default::default()
                    },
                );
            }
        }
        None => bs_metrics::record_llm_tokens_usage_missing(
            provider,
            model,
            &attribution.org,
            &attribution.service,
        ),
    }
}

pub async fn embeddings(
    request: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let request_id = extract_request_id(&request);
    let request_span = info_span!(
        "llm",
        component = "llm",
        request_id = %request_id,
        http.method = %request.method(),
        http.path = %request.uri().path(),
        llm.model = tracing::field::Empty,
    );
    embeddings_inner(request, state)
        .instrument(request_span)
        .await
}

async fn embeddings_inner(
    request: Request<hyper::body::Incoming>,
    state: Arc<AppState>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    set_service_name(operation_component::LLM);

    let mut client_headers = request.headers().clone();
    let attribution =
        take_attribution_from_headers(&mut client_headers, state.attribution_header.as_deref());

    let body = request.into_body().collect().await?.to_bytes();

    let upstream = {
        let providers = state.llm_providers.read().await;
        build_upstream_request(&body, &providers, &state.model_aliases)
    };
    let upstream = match upstream {
        Ok(u) => u,
        Err(e) => return Ok(e.into_response()),
    };

    tracing::Span::current().record(tracing_llm::MODEL_NAME, upstream.model_label.as_str());
    get_active_span(|span| {
        span.set_attribute(opentelemetry::KeyValue::new(
            tracing_llm::PROVIDER,
            upstream.provider_label.clone(),
        ));
    });

    let started = std::time::Instant::now();
    let response = match state
        .http_client
        .post(&upstream.url)
        .headers(upstream.headers)
        .body(upstream.body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(err) => {
            warn!(error = %err, "embeddings upstream request failed");
            bs_metrics::record_llm_upstream(
                &upstream.provider_label,
                &upstream.model_label,
                0,
                bs_metrics::llm_error_class_from_reqwest(&err),
                started.elapsed(),
            );
            return Ok(EmbeddingsError {
                status: StatusCode::BAD_GATEWAY,
                body: json!({
                    "error": {
                        "message": format!("Failed to reach the embeddings provider: {err}"),
                        "type": "api_error",
                        "param": null,
                        "code": "upstream_unreachable",
                    }
                }),
            }
            .into_response());
        }
    };

    let status = response.status();
    let response_body = match response.bytes().await {
        Ok(b) => b,
        Err(err) => {
            warn!(error = %err, "embeddings upstream body read failed");
            bs_metrics::record_llm_upstream(
                &upstream.provider_label,
                &upstream.model_label,
                status.as_u16(),
                bs_metrics::llm_error_class_from_reqwest(&err),
                started.elapsed(),
            );
            return Ok(EmbeddingsError {
                status: StatusCode::BAD_GATEWAY,
                body: json!({
                    "error": {
                        "message": format!("Failed to read the embeddings provider response: {err}"),
                        "type": "api_error",
                        "param": null,
                        "code": "upstream_unreachable",
                    }
                }),
            }
            .into_response());
        }
    };

    // The upstream responded, so status_class alone carries the non-2xx signal.
    bs_metrics::record_llm_upstream(
        &upstream.provider_label,
        &upstream.model_label,
        status.as_u16(),
        metric_labels::LLM_ERR_NONE,
        started.elapsed(),
    );
    if status.is_success() {
        record_embeddings_usage(
            &upstream.provider_label,
            &upstream.model_label,
            &attribution,
            state.usage_log.as_ref(),
            &response_body,
        );
        if let Some(n) = prompt_tokens_from_response(&response_body) {
            get_active_span(|span| {
                span.set_attribute(opentelemetry::KeyValue::new(
                    tracing_llm::PROMPT_TOKENS,
                    n as i64,
                ));
            });
        }
    }

    Ok(json_response(status, response_body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::capture::Capture;
    use crate::usage_log::UsageLog;

    fn azure() -> LlmProvider {
        LlmProvider {
            name: "azure_openai/text-embedding-3-small".to_string(),
            provider_interface: LlmProviderType::AzureOpenAI,
            access_key: Some("azure-secret".to_string()),
            model: Some("text-embedding-3-small".to_string()),
            endpoint: Some("fikar-openai.openai.azure.com".to_string()),
            port: Some(443),
            default: None,
            ..LlmProvider::default()
        }
    }

    fn openai_v1() -> LlmProvider {
        LlmProvider {
            name: "foundry/embed-x".to_string(),
            provider_interface: LlmProviderType::OpenAI,
            access_key: Some("oai-secret".to_string()),
            model: Some("embed-x".to_string()),
            endpoint: Some("x.openai.azure.com".to_string()),
            port: Some(443),
            base_url_path_prefix: Some("/openai/v1".to_string()),
            headers: Some(HashMap::from([("x-extra".to_string(), "1".to_string())])),
            default: None,
            ..LlmProvider::default()
        }
    }

    fn providers(list: Vec<LlmProvider>) -> LlmProviders {
        LlmProviders::try_from(list).unwrap()
    }

    fn build(body: Value, p: &LlmProviders) -> Result<UpstreamRequest, EmbeddingsError> {
        build_upstream_request(body.to_string().as_bytes(), p, &None)
    }

    #[test]
    fn azure_url_and_body_rewrite() {
        let p = providers(vec![azure()]);
        let up = build(
            json!({"model":"text-embedding-3-small","input":["a","b"],"dimensions":256}),
            &p,
        )
        .unwrap();
        assert_eq!(
            up.url,
            "https://fikar-openai.openai.azure.com/openai/deployments/text-embedding-3-small/embeddings?api-version=2025-01-01-preview"
        );
        let body: Value = serde_json::from_slice(&up.body).unwrap();
        assert_eq!(
            body,
            json!({"model":"text-embedding-3-small","input":["a","b"],"dimensions":256})
        );
        assert_eq!(up.model_label, "text-embedding-3-small");
        assert_eq!(up.provider_label, "azure_openai");
    }

    #[test]
    fn prefixed_and_bare_model_both_resolve() {
        let p = providers(vec![azure()]);
        for m in [
            "text-embedding-3-small",
            "azure_openai/text-embedding-3-small",
        ] {
            let up = build(json!({"model": m, "input": "x"}), &p).unwrap();
            let body: Value = serde_json::from_slice(&up.body).unwrap();
            assert_eq!(body["model"], "text-embedding-3-small", "for {m}");
            assert_eq!(up.provider_label, "azure_openai", "for {m}");
            assert!(up.url.contains("/deployments/text-embedding-3-small/"));
        }
    }

    #[test]
    fn alias_is_resolved() {
        let p = providers(vec![azure()]);
        let aliases = Some(HashMap::from([(
            "embed".to_string(),
            ModelAlias {
                target: "azure_openai/text-embedding-3-small".to_string(),
            },
        )]));
        let up = build_upstream_request(br#"{"model":"embed","input":"x"}"#, &p, &aliases).unwrap();
        let body: Value = serde_json::from_slice(&up.body).unwrap();
        assert_eq!(body["model"], "text-embedding-3-small");
    }

    #[test]
    fn openai_interface_url_uses_base_prefix() {
        let p = providers(vec![openai_v1()]);
        let up = build(json!({"model":"embed-x","input":"x"}), &p).unwrap();
        assert_eq!(up.url, "https://x.openai.azure.com/openai/v1/embeddings");
    }

    #[test]
    fn openai_interface_defaults_to_v1_and_plain_http_ports() {
        let mut prov = openai_v1();
        prov.base_url_path_prefix = None;
        prov.port = Some(8089);
        prov.endpoint = Some("127.0.0.1".to_string());
        assert_eq!(
            upstream_url(&prov, "embed-x").unwrap(),
            "http://127.0.0.1:8089/v1/embeddings"
        );
        prov.port = None;
        prov.endpoint = None;
        assert_eq!(
            upstream_url(&prov, "embed-x").unwrap(),
            "https://api.openai.com/v1/embeddings"
        );
        prov.provider_interface = LlmProviderType::Groq;
        assert!(upstream_url(&prov, "embed-x").is_err());
    }

    #[test]
    fn azure_prefix_replaces_deployments_segment() {
        let mut prov = azure();
        prov.base_url_path_prefix = Some("/custom/azure/path/".to_string());
        assert_eq!(
            upstream_url(&prov, "m").unwrap(),
            "https://fikar-openai.openai.azure.com/custom/azure/path/m/embeddings?api-version=2025-01-01-preview"
        );
    }

    #[test]
    fn headers_carry_only_provider_credentials() {
        let p = providers(vec![azure(), openai_v1()]);
        let az = build(json!({"model":"text-embedding-3-small","input":"x"}), &p).unwrap();
        assert_eq!(az.headers.get("api-key").unwrap(), "azure-secret");
        assert!(az.headers.get("authorization").is_none());
        assert_eq!(az.headers.get("content-type").unwrap(), "application/json");

        let oa = build(json!({"model":"embed-x","input":"x"}), &p).unwrap();
        assert_eq!(
            oa.headers.get("authorization").unwrap(),
            "Bearer oai-secret"
        );
        assert!(oa.headers.get("api-key").is_none());
        assert_eq!(oa.headers.get("x-extra").unwrap(), "1");
        // Nothing is copied from the client, so the attribution header cannot appear.
        assert!(oa.headers.get("x-fikar-org").is_none());
        assert!(az.headers.get("x-fikar-org").is_none());
    }

    #[test]
    fn take_org_strips_header_and_normalizes() {
        let mut h = HeaderMap::new();
        h.insert("x-fikar-org", HeaderValue::from_static("acme"));
        h.insert("authorization", HeaderValue::from_static("Bearer client"));
        let a = take_attribution_from_headers(&mut h, Some("x-fikar-org"));
        assert_eq!(a.org, "acme");
        assert_eq!(a.service, metric_labels::SERVICE_UNATTRIBUTED);
        assert_eq!((a.user.as_str(), a.execution.as_str()), ("", ""));
        assert!(h.get("x-fikar-org").is_none());
        assert!(h.get("authorization").is_some());

        let mut h = HeaderMap::new();
        h.insert("x-fikar-org", HeaderValue::from_static("bad org!"));
        assert_eq!(
            take_attribution_from_headers(&mut h, Some("x-fikar-org")).org,
            metric_labels::ORG_INVALID
        );
    }

    #[test]
    fn every_x_fikar_header_is_stripped_even_without_attribution_header() {
        let mut h = HeaderMap::new();
        for (k, v) in [
            ("x-fikar-org", "acme"),
            ("X-Fikar-Service", "chat"),
            ("x-fikar-user", "42"),
            ("X-FIKAR-EXECUTION", "turn-1"),
            ("x-fikar-anything-else", "z"),
            ("x-other", "keep"),
        ] {
            h.insert(
                hyper::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_static(v),
            );
        }
        let a = take_attribution_from_headers(&mut h, None);
        assert_eq!(a.org, metric_labels::ORG_UNATTRIBUTED);
        assert_eq!(
            (a.service.as_str(), a.user.as_str(), a.execution.as_str()),
            ("chat", "42", "turn-1")
        );
        assert_eq!(h.len(), 1);
        assert!(h.get("x-other").is_some());
    }

    /// Wire check: the headers left after `take_attribution_from_headers` are what the
    /// handlers forward, and none of the four attribution headers reaches the provider.
    #[tokio::test]
    async fn attribution_headers_never_reach_the_provider() {
        let mut server = mockito::Server::new_async().await;
        let mut mock = server.mock("POST", "/x");
        for name in [
            "x-fikar-org",
            "x-fikar-service",
            "x-fikar-user",
            "x-fikar-execution",
        ] {
            mock = mock.match_header(name, mockito::Matcher::Missing);
        }
        let mock = mock
            .match_header("x-extra", "1")
            .with_status(200)
            .create_async()
            .await;

        let mut h = HeaderMap::new();
        h.insert("x-fikar-org", HeaderValue::from_static("acme"));
        h.insert("x-fikar-service", HeaderValue::from_static("chat"));
        h.insert("x-fikar-user", HeaderValue::from_static("42"));
        h.insert("x-fikar-execution", HeaderValue::from_static("turn-1"));
        h.insert("x-extra", HeaderValue::from_static("1"));
        take_attribution_from_headers(&mut h, Some("x-fikar-org"));
        let resp = reqwest::Client::new()
            .post(format!("{}/x", server.url()))
            .headers(h)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        mock.assert_async().await;
    }

    #[test]
    fn missing_or_bad_model_is_400() {
        let p = providers(vec![azure()]);
        for body in [
            json!({"input":"x"}),
            json!({"model":5,"input":"x"}),
            json!({"model":"","input":"x"}),
        ] {
            let err = build(body, &p).unwrap_err();
            assert_eq!(err.status, StatusCode::BAD_REQUEST);
            assert_eq!(err.body["error"]["type"], "invalid_request_error");
            assert_eq!(err.body["error"]["param"], "model");
            assert!(err.body["error"]["message"].is_string());
        }
        let err = build_upstream_request(b"not json", &p, &None).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(err.body["error"]["type"], "invalid_request_error");
    }

    #[test]
    fn unknown_model_is_404() {
        let p = providers(vec![azure()]);
        let err = build(json!({"model":"nope","input":"x"}), &p).unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(err.body["error"]["type"], "invalid_request_error");
        assert_eq!(err.body["error"]["code"], "model_not_found");
    }

    #[test]
    fn missing_access_key_is_500_not_a_leak() {
        let mut prov = azure();
        prov.access_key = None;
        let p = providers(vec![prov]);
        let err = build(json!({"model":"text-embedding-3-small","input":"x"}), &p).unwrap_err();
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    const AZURE_RESPONSE: &str = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":[0.1,0.2]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":7,"total_tokens":7}}"#;

    #[test]
    fn prompt_tokens_extraction() {
        assert_eq!(
            prompt_tokens_from_response(AZURE_RESPONSE.as_bytes()),
            Some(7)
        );
        assert_eq!(prompt_tokens_from_response(br#"{"data":[]}"#), None);
        assert_eq!(prompt_tokens_from_response(br#"{"usage":{}}"#), None);
        assert_eq!(prompt_tokens_from_response(b"garbage"), None);
    }

    fn attribution(org: &str, service: &str) -> Attribution {
        Attribution {
            org: org.into(),
            service: service.into(),
            user: "42".into(),
            execution: "turn-1".into(),
        }
    }

    #[test]
    fn usage_is_metered_or_flagged_missing() {
        let cap = Capture::default();
        let acme = attribution("acme", "chat");
        metrics::with_local_recorder(&cap, || {
            record_embeddings_usage(
                "azure_openai",
                "text-embedding-3-small",
                &acme,
                None,
                AZURE_RESPONSE.as_bytes(),
            );
            record_embeddings_usage(
                "azure_openai",
                "text-embedding-3-small",
                &acme,
                None,
                br#"{"data":[]}"#,
            );
        });
        let got = cap.0.lock().unwrap().clone();
        assert_eq!(
            got.get("brightstaff_llm_tokens_total{kind=prompt,model=text-embedding-3-small,org=acme,provider=azure_openai,service=chat}"),
            Some(&7)
        );
        assert_eq!(
            got.get("brightstaff_llm_tokens_usage_missing_total{model=text-embedding-3-small,org=acme,provider=azure_openai,service=chat}"),
            Some(&1)
        );
    }

    #[test]
    fn service_label_defaults_and_invalid() {
        let cap = Capture::default();
        metrics::with_local_recorder(&cap, || {
            for raw in [None, Some("Bad Service!")] {
                let mut h = HeaderMap::new();
                if let Some(v) = raw {
                    h.insert("x-fikar-service", HeaderValue::from_str(v).unwrap());
                }
                let a = take_attribution_from_headers(&mut h, Some("x-fikar-org"));
                record_embeddings_usage("p", "m", &a, None, AZURE_RESPONSE.as_bytes());
            }
        });
        let got = cap.0.lock().unwrap().clone();
        for svc in ["unattributed", "invalid"] {
            assert_eq!(
                got.get(&format!(
                    "brightstaff_llm_tokens_total{{kind=prompt,model=m,org=unattributed,provider=p,service={svc}}}"
                )),
                Some(&7),
                "{svc}"
            );
        }
    }

    #[tokio::test]
    async fn metered_embeddings_write_exactly_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let log = UsageLog::start(dir.path().to_path_buf(), "pinned");
        let a = attribution("3", "dictionary");
        record_embeddings_usage(
            "azure_openai",
            "te3",
            &a,
            Some(&log),
            AZURE_RESPONSE.as_bytes(),
        );
        // No usage in the response: counter only, no line.
        record_embeddings_usage("azure_openai", "te3", &a, Some(&log), br#"{"data":[]}"#);
        log.flush().await;
        let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(files.len(), 1);
        let text = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
        assert_eq!(text.lines().count(), 1);
        let v: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["endpoint"], "embeddings");
        assert_eq!(v["instance"], "pinned");
        assert_eq!(v["org"], "3");
        assert_eq!(v["service"], "dictionary");
        assert_eq!(v["user"], "42");
        assert_eq!(v["execution"], "turn-1");
        assert_eq!(v["provider"], "azure_openai");
        assert_eq!(v["model"], "te3");
        assert_eq!(v["prompt"], 7);
        assert_eq!(v["completion"], 0);
        assert_eq!(v["cache_read"], 0);
        assert_eq!(v["cache_write"], 0);
    }

    #[tokio::test]
    async fn no_log_directory_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let a = attribution("3", "chat");
        // Disabled log is `None`: metering still works and no file appears.
        record_embeddings_usage("p", "m", &a, None, AZURE_RESPONSE.as_bytes());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// Wire-level check against a local mock provider: the request the handler would
    /// send reaches the right path with provider credentials and the rewritten body.
    #[tokio::test]
    async fn upstream_request_over_the_wire() {
        let mut server = mockito::Server::new_async().await;
        let addr = server.host_with_port();
        let (host, port) = addr.split_once(':').unwrap();
        let mut prov = azure();
        prov.endpoint = Some(host.to_string());
        prov.port = Some(port.parse().unwrap());
        let p = providers(vec![prov]);
        let up = build(
            json!({"model":"azure_openai/text-embedding-3-small","input":"hello"}),
            &p,
        )
        .unwrap();
        assert!(up.url.starts_with("http://127.0.0.1:"));

        let mock = server
            .mock(
                "POST",
                "/openai/deployments/text-embedding-3-small/embeddings",
            )
            .match_query(mockito::Matcher::UrlEncoded(
                "api-version".into(),
                AZURE_API_VERSION.into(),
            ))
            .match_header("api-key", "azure-secret")
            .match_header("authorization", mockito::Matcher::Missing)
            .match_header("x-fikar-org", mockito::Matcher::Missing)
            .match_body(mockito::Matcher::Json(
                json!({"model":"text-embedding-3-small","input":"hello"}),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(AZURE_RESPONSE)
            .create_async()
            .await;

        let resp = reqwest::Client::new()
            .post(&up.url)
            .headers(up.headers)
            .body(up.body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let bytes = resp.bytes().await.unwrap();
        assert_eq!(prompt_tokens_from_response(&bytes), Some(7));
        mock.assert_async().await;
    }
}
