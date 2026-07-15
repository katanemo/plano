use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use common::configuration::Configuration;
use std::time::Duration;
use tracing::{debug, info};

pub mod memory;
pub mod redis;

/// A conversation's binding to a model, plus the state the session router needs to
/// reason about cache warmth and switch affordability across turns.
///
/// Warmth is no longer derived from the cache's own expiry — the entry is kept alive
/// as a plain KV value (subject only to a GC bound) and the router decides warmth from
/// [`SessionBinding::last_used`] against the provider's cache window. This is what lets
/// the decision path reason about warmth without ever seeing a provider response.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SessionBinding {
    /// Provider-qualified model that handled the latest request (e.g. `openai/gpt-4o`).
    /// This is what the session is currently *warm on*; a proposed switch's cost is
    /// measured against reading the context at this model's cached rate. It tracks the
    /// last dispatched model and changes whenever a switch is honored.
    pub anchor_model: String,
    /// Provider-qualified model the session started on this warm episode — the model it
    /// would have stayed on had it *never switched*. The never-switch baseline is priced
    /// against this (not `anchor_model`, which drifts as switches happen). Set when a warm
    /// episode begins and preserved across its turns.
    #[serde(default)]
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_name: Option<String>,
    /// Hash of the stable prompt prefix (system + tools) observed when the binding was
    /// stored. Used to detect prefix drift: if a later request's prefix hash differs,
    /// the provider cache is already lost so a switch is free.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_hash: Option<u64>,
    /// When this session was last dispatched. Warmth = `now - last_used` compared
    /// against the provider's idle/hard cache window.
    #[serde(default = "SystemTime::now", with = "epoch_secs")]
    pub last_used: SystemTime,
    /// Best estimate of the cacheable context size (input tokens) — the tokens a switch
    /// would have to re-ingest at the uncached rate. Refined from real usage on the
    /// full-proxy path; the tokenizer estimate on the decision path.
    #[serde(default)]
    pub cached_tokens: u64,
    /// Cumulative *never-switch* baseline (USD) for this warm episode: the running cost
    /// the session would have paid by staying on its `default_model`. Grows each warm
    /// turn. This is the denominator the percentage overhead cap is measured against.
    #[serde(default)]
    pub baseline_usd: f64,
    /// Cumulative overhead (USD) actually spent on paid switches this warm episode.
    /// Monotonic: paid switches add to it, free/cheaper switches never subtract. A paid
    /// switch is allowed only while `switch_spend_usd + cost <= pct * baseline_usd`.
    #[serde(default)]
    pub switch_spend_usd: f64,
    /// Number of model switches taken during this warm session (observability).
    #[serde(default)]
    pub switches: u32,
    /// Bounded most-recent-first-ish record of the models this session has been
    /// dispatched to, each with when it was last used and how large the context was
    /// then. Lets the switch-cost estimate credit a *return* to a still-warm model (it
    /// re-reads only the tokens appended since, not the whole context) and gives future
    /// routing policies per-model recency to reason about. Capped at
    /// [`MAX_ROUTE_HISTORY`] distinct models (LRU-evicted).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<RouteVisit>,
    /// Cumulative *actual* cost (USD, input + output) of the whole conversation, priced
    /// from the configured catalog rates and refined from real usage each turn on the
    /// full-proxy path. Conversation-level (not per warm episode): it persists across
    /// cold re-binds and is best-effort (resets only if the binding is evicted).
    #[serde(default)]
    pub session_cost_usd: f64,
}

/// One model this session has been dispatched to, with the recency and context size
/// needed to estimate whether its provider cache is still warm on a later return.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RouteVisit {
    /// Provider-qualified model id (e.g. `openai/gpt-4o`).
    pub model: String,
    /// When this model last handled a turn in this session.
    #[serde(with = "epoch_secs")]
    pub last_used: SystemTime,
    /// Context size (input tokens) that model saw on its last turn — the prefix it may
    /// still have cached if it's revisited before its cache window elapses.
    #[serde(default)]
    pub cached_tokens: u64,
}

/// Max distinct models retained in [`SessionBinding::history`]. Small: real sessions
/// touch a handful of models, and the entry has to stay compact on the Redis wire.
pub const MAX_ROUTE_HISTORY: usize = 8;

/// Record (or refresh) a model visit in `history`, then LRU-evict down to
/// [`MAX_ROUTE_HISTORY`]. Refreshing an existing model updates its recency and context
/// size in place rather than appending a duplicate.
pub fn record_route_visit(
    history: &mut Vec<RouteVisit>,
    model: &str,
    last_used: SystemTime,
    cached_tokens: u64,
) {
    if let Some(entry) = history.iter_mut().find(|e| e.model == model) {
        entry.last_used = last_used;
        entry.cached_tokens = cached_tokens;
    } else {
        history.push(RouteVisit {
            model: model.to_string(),
            last_used,
            cached_tokens,
        });
    }
    while history.len() > MAX_ROUTE_HISTORY {
        if let Some(idx) = history
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| e.last_used)
            .map(|(i, _)| i)
        {
            history.remove(idx);
        } else {
            break;
        }
    }
}

/// Serde helper: persist `SystemTime` as whole epoch seconds so the Redis wire format
/// is stable and compact (the default `SystemTime` representation is version-fragile).
mod epoch_secs {
    use super::{Duration, SystemTime, UNIX_EPOCH};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let secs = t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        s.serialize_u64(secs)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(UNIX_EPOCH + Duration::from_secs(secs))
    }
}

#[async_trait]
pub trait SessionCache: Send + Sync {
    /// Look up a session binding by key. `None` when absent or GC-evicted. Warmth is
    /// the caller's concern (time since `last_used`), not the cache's.
    async fn get(&self, key: &str) -> Option<SessionBinding>;

    /// Store a session binding with the given GC TTL. The TTL is only a memory bound
    /// (keep the entry around at least as long as it could plausibly be warm); it does
    /// not define warmth.
    async fn put(&self, key: &str, binding: SessionBinding, ttl: Duration);

    /// Remove a session binding by key.
    async fn remove(&self, key: &str);
}

/// Initialize the session cache backend from config.
/// Defaults to the in-memory backend when no `session_cache` block is configured.
pub async fn init_session_cache(
    config: &Configuration,
) -> Result<Arc<dyn SessionCache>, Box<dyn std::error::Error + Send + Sync>> {
    use common::configuration::SessionCacheType;

    let session_max_entries = config.routing.as_ref().and_then(|r| r.session_max_entries);

    const DEFAULT_SESSION_MAX_ENTRIES: usize = 10_000;
    const MAX_SESSION_MAX_ENTRIES: usize = 10_000;

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
            Ok(Arc::new(memory::MemorySessionCache::new(max_entries)))
        }
        SessionCacheType::Redis => {
            let url = cache_config
                .and_then(|c| c.url.as_ref())
                .ok_or("session_cache.url is required when type is redis")?;
            debug!(storage_type = "redis", url = %url, "initializing session cache");
            let cache = redis::RedisSessionCache::new(url)
                .await
                .map_err(|e| format!("failed to connect to Redis session cache: {e}"))?;
            Ok(Arc::new(cache))
        }
    }
}
