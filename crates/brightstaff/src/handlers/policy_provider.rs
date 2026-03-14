use std::sync::Arc;
use std::time::Duration;

use common::configuration::{ModelUsagePreference, RoutingPolicyProvider};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use tracing::warn;

use crate::state::policy_cache::PolicyCache;

const DEFAULT_POLICY_TTL_SECONDS: u64 = 300;

#[derive(Debug, Deserialize)]
struct ExternalPolicyResponse {
    policy_id: String,
    routing_preferences: Vec<ModelUsagePreference>,
}

#[derive(Debug)]
pub enum PolicyFetchError {
    Transient(String),
    Invalid(String),
}

impl PolicyFetchError {
    pub fn is_transient(&self) -> bool {
        matches!(self, PolicyFetchError::Transient(_))
    }

    pub fn message(&self) -> &str {
        match self {
            PolicyFetchError::Transient(msg) | PolicyFetchError::Invalid(msg) => msg,
        }
    }
}

pub struct PolicyProviderClient {
    client: reqwest::Client,
    config: RoutingPolicyProvider,
    cache: Arc<PolicyCache>,
    ttl: Duration,
}

impl PolicyProviderClient {
    pub fn new(config: RoutingPolicyProvider, cache: Arc<PolicyCache>) -> Self {
        let ttl = Duration::from_secs(config.ttl_seconds.unwrap_or(DEFAULT_POLICY_TTL_SECONDS));
        Self {
            client: reqwest::Client::new(),
            config,
            cache,
            ttl,
        }
    }

    pub async fn fetch_policy(
        &self,
        policy_id: &str,
    ) -> Result<Vec<ModelUsagePreference>, PolicyFetchError> {
        if let Some(cached) = self.cache.get_valid(policy_id).await {
            return Ok(cached);
        }

        let headers = self.build_headers()?;
        let response = self
            .client
            .get(&self.config.url)
            .query(&[("policy_id", policy_id)])
            .headers(headers)
            .send()
            .await
            .map_err(|err| PolicyFetchError::Transient(format!("policy fetch failed: {}", err)))?;

        if !response.status().is_success() {
            return if response.status().is_server_error() {
                Err(PolicyFetchError::Transient(format!(
                    "policy provider returned {}",
                    response.status()
                )))
            } else {
                Err(PolicyFetchError::Invalid(format!(
                    "policy provider returned non-success status {}",
                    response.status()
                )))
            };
        }

        let payload: ExternalPolicyResponse = response
            .json()
            .await
            .map_err(|err| PolicyFetchError::Invalid(format!("invalid policy payload: {}", err)))?;

        if payload.policy_id != policy_id {
            return Err(PolicyFetchError::Invalid(format!(
                "policy_id mismatch in provider response: expected '{}', got '{}'",
                policy_id, payload.policy_id
            )));
        }

        if payload.routing_preferences.is_empty() {
            warn!(
                policy_id,
                "policy provider returned empty routing preferences"
            );
        }

        self.cache
            .insert(
                policy_id.to_string(),
                payload.routing_preferences.clone(),
                self.ttl,
            )
            .await;
        Ok(payload.routing_preferences)
    }

    fn build_headers(&self) -> Result<HeaderMap, PolicyFetchError> {
        let mut headers = HeaderMap::new();
        if let Some(configured_headers) = &self.config.headers {
            for (name, value) in configured_headers {
                let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
                    PolicyFetchError::Invalid(format!(
                        "invalid header name '{}' in routing.policy_provider.headers: {}",
                        name, err
                    ))
                })?;
                let header_value = HeaderValue::from_str(value).map_err(|err| {
                    PolicyFetchError::Invalid(format!(
                        "invalid header value for '{}' in routing.policy_provider.headers: {}",
                        name, err
                    ))
                })?;
                headers.insert(header_name, header_value);
            }
        }
        Ok(headers)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use common::configuration::RoutingPolicyProvider;
    use mockito::{Matcher, Server};

    use crate::handlers::policy_provider::{PolicyFetchError, PolicyProviderClient};
    use crate::state::policy_cache::PolicyCache;

    fn provider_config(url: String, ttl_seconds: Option<u64>) -> RoutingPolicyProvider {
        RoutingPolicyProvider {
            url,
            headers: None,
            ttl_seconds,
        }
    }

    #[tokio::test]
    async fn fetches_policy_and_populates_cache() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/routing-policy")
            .match_query(Matcher::UrlEncoded(
                "policy_id".to_string(),
                "customer-abc".to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "policy_id":"customer-abc",
                    "routing_preferences":[
                        {
                            "model":"openai/gpt-4o",
                            "routing_preferences":[{"name":"quick response","description":"fast"}]
                        }
                    ]
                }"#,
            )
            .expect(1)
            .create_async()
            .await;

        let cache = Arc::new(PolicyCache::new());
        let client = PolicyProviderClient::new(
            provider_config(format!("{}/v1/routing-policy", server.url()), Some(300)),
            cache,
        );

        let first = client.fetch_policy("customer-abc").await.unwrap();
        let second = client.fetch_policy("customer-abc").await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second[0].model, "openai/gpt-4o");
    }

    #[tokio::test]
    async fn returns_invalid_on_policy_id_mismatch() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/routing-policy")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "policy_id":"different-id",
                    "routing_preferences":[]
                }"#,
            )
            .create_async()
            .await;

        let cache = Arc::new(PolicyCache::new());
        let client = PolicyProviderClient::new(
            provider_config(format!("{}/v1/routing-policy", server.url()), Some(300)),
            cache,
        );

        let err = client.fetch_policy("customer-abc").await.unwrap_err();
        assert!(matches!(err, PolicyFetchError::Invalid(_)));
    }

    #[tokio::test]
    async fn returns_transient_on_server_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/routing-policy")
            .match_query(Matcher::Any)
            .with_status(500)
            .create_async()
            .await;

        let cache = Arc::new(PolicyCache::new());
        let client = PolicyProviderClient::new(
            provider_config(format!("{}/v1/routing-policy", server.url()), Some(300)),
            cache,
        );

        let err = client.fetch_policy("customer-abc").await.unwrap_err();
        assert!(err.is_transient());
    }

    #[tokio::test]
    async fn returns_invalid_on_client_error_status() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/routing-policy")
            .match_query(Matcher::Any)
            .with_status(404)
            .create_async()
            .await;

        let cache = Arc::new(PolicyCache::new());
        let client = PolicyProviderClient::new(
            provider_config(format!("{}/v1/routing-policy", server.url()), Some(300)),
            cache,
        );

        let err = client.fetch_policy("customer-abc").await.unwrap_err();
        assert!(matches!(err, PolicyFetchError::Invalid(_)));
    }

    #[tokio::test]
    async fn supports_headers() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/routing-policy")
            .match_header("authorization", "Bearer token")
            .match_query(Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"policy_id":"customer-abc","routing_preferences":[]}"#)
            .create_async()
            .await;

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token".to_string());
        let cache = Arc::new(PolicyCache::new());
        let client = PolicyProviderClient::new(
            RoutingPolicyProvider {
                url: format!("{}/v1/routing-policy", server.url()),
                headers: Some(headers),
                ttl_seconds: Some(Duration::from_secs(300).as_secs()),
            },
            cache,
        );

        let _ = client.fetch_policy("customer-abc").await.unwrap();
    }
}
