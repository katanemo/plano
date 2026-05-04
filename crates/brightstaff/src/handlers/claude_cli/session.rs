//! Session manager for the claude-cli bridge. Maps a stable session id (taken
//! from a client-provided header or hashed from the conversation prefix) to a
//! long-lived `ClaudeProcess`. Enforces an idle TTL and a hard cap on the
//! number of concurrent sessions.

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use hermesllm::apis::anthropic::{
    MessagesContentBlock, MessagesMessageContent, MessagesRequest, MessagesRole,
    MessagesSystemPrompt,
};
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::{debug, info};

use super::process::{ClaudeCliConfig, ClaudeProcess, ProcessError};

/// Optional client header that pins a request to a specific session id.
pub const SESSION_HEADER: &str = "x-arch-claude-cli-session";

/// Default cap. The bridge is local and per-developer; this is a guard
/// against runaway memory if a client bug churns through unique session ids.
pub const DEFAULT_MAX_SESSIONS: usize = 64;

/// Tunables for the session manager.
#[derive(Debug, Clone)]
pub struct SessionManagerConfig {
    pub max_sessions: usize,
    pub process: ClaudeCliConfig,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            max_sessions: DEFAULT_MAX_SESSIONS,
            process: ClaudeCliConfig::default(),
        }
    }
}

/// Holds active `ClaudeProcess` handles keyed by session id.
pub struct SessionManager {
    inner: Mutex<HashMap<String, Arc<ClaudeProcess>>>,
    config: SessionManagerConfig,
}

impl SessionManager {
    pub fn new(config: SessionManagerConfig) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(HashMap::new()),
            config,
        })
    }

    /// Pick (or fabricate) the session id for a given request.
    ///
    /// Strategy (in order):
    /// 1. Honor the `x-arch-claude-cli-session` header if it's a non-empty
    ///    valid UUID-shaped string.
    /// 2. Otherwise hash `(model, system_prompt_text, first_user_message_text)`
    ///    and produce a deterministic UUID-shaped id so retries of the same
    ///    conversation reuse the same process.
    pub fn resolve_session_id(client_header: Option<&str>, req: &MessagesRequest) -> String {
        if let Some(raw) = client_header {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                // Accept any opaque token; the CLI requires UUID format, so
                // we hash unknown shapes into one.
                if uuid::Uuid::parse_str(trimmed).is_ok() {
                    return trimmed.to_string();
                }
                return uuid_from_seed(trimmed);
            }
        }
        let mut hasher = DefaultHasher::new();
        req.model.hash(&mut hasher);
        if let Some(system) = &req.system {
            system_text(system).hash(&mut hasher);
        }
        if let Some(first) = first_user_message_text(req) {
            first.hash(&mut hasher);
        }
        uuid_from_seed(&hasher.finish().to_string())
    }

    /// Get the existing session's process or spawn a new one.
    pub async fn get_or_spawn(
        &self,
        session_id: &str,
        model: &str,
        system_prompt: Option<&str>,
        cwd: Option<&std::path::Path>,
    ) -> Result<Arc<ClaudeProcess>, ProcessError> {
        // Reap idle sessions on the read path so we don't need a separate
        // background task for the common one-developer-one-laptop deployment.
        self.evict_idle().await;

        {
            let map = self.inner.lock().await;
            if let Some(existing) = map.get(session_id) {
                debug!(session = %session_id, "reusing claude-cli session");
                return Ok(Arc::clone(existing));
            }
        }

        let mut map = self.inner.lock().await;
        if let Some(existing) = map.get(session_id) {
            return Ok(Arc::clone(existing));
        }

        if map.len() >= self.config.max_sessions {
            // Evict the least-recently-used session to keep the cap honest.
            if let Some(victim_key) = lru_session_id(&map).await {
                if let Some(victim) = map.remove(&victim_key) {
                    info!(session = %victim_key, "evicting LRU claude-cli session to make room");
                    drop(map);
                    victim.shutdown().await;
                    map = self.inner.lock().await;
                }
            }
        }

        let process = ClaudeProcess::spawn(
            session_id.to_string(),
            model,
            system_prompt,
            cwd,
            self.config.process.clone(),
        )
        .await?;
        map.insert(session_id.to_string(), Arc::clone(&process));
        Ok(process)
    }

    /// Drop and kill all sessions. Called on graceful shutdown.
    pub async fn shutdown_all(&self) {
        let mut map = self.inner.lock().await;
        let drained: Vec<_> = map.drain().collect();
        drop(map);
        info!(count = drained.len(), "draining claude-cli sessions");
        for (_, proc) in drained {
            proc.shutdown().await;
        }
    }

    async fn evict_idle(&self) {
        let ttl = self.config.process.session_ttl;
        if ttl.is_zero() {
            return;
        }
        let now = Instant::now();
        let mut to_kill: Vec<(String, Arc<ClaudeProcess>)> = Vec::new();
        {
            let map = self.inner.lock().await;
            for (k, v) in map.iter() {
                if now.duration_since(v.last_used().await) > ttl {
                    to_kill.push((k.clone(), Arc::clone(v)));
                }
            }
        }
        if to_kill.is_empty() {
            return;
        }
        let mut map = self.inner.lock().await;
        for (k, _) in &to_kill {
            map.remove(k);
        }
        drop(map);
        for (k, proc) in to_kill {
            info!(session = %k, "evicting idle claude-cli session");
            proc.shutdown().await;
        }
    }
}

async fn lru_session_id(map: &HashMap<String, Arc<ClaudeProcess>>) -> Option<String> {
    let mut oldest: Option<(String, Instant)> = None;
    for (k, v) in map.iter() {
        let used = v.last_used().await;
        match &oldest {
            Some((_, t)) if *t < used => {}
            _ => oldest = Some((k.clone(), used)),
        }
    }
    oldest.map(|(k, _)| k)
}

fn first_user_message_text(req: &MessagesRequest) -> Option<String> {
    for msg in &req.messages {
        if msg.role != MessagesRole::User {
            continue;
        }
        return Some(match &msg.content {
            MessagesMessageContent::Single(s) => s.clone(),
            MessagesMessageContent::Blocks(blocks) => blocks
                .iter()
                .filter_map(|b| match b {
                    MessagesContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }
    None
}

fn system_text(system: &MessagesSystemPrompt) -> String {
    match system {
        MessagesSystemPrompt::Single(s) => s.clone(),
        MessagesSystemPrompt::Blocks(blocks) => blocks
            .iter()
            .filter_map(|b| match b {
                MessagesContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Deterministic v5-style UUID derived from an arbitrary seed string. The
/// `claude` CLI requires `--session-id` to be a valid UUID; we use the DNS
/// namespace constant as a stable salt so the same conversation always maps
/// to the same id without us pulling in the v5 feature of the `uuid` crate.
fn uuid_from_seed(seed: &str) -> String {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    let h1 = hasher.finish();
    let mut hasher2 = DefaultHasher::new();
    h1.hash(&mut hasher2);
    seed.hash(&mut hasher2);
    let h2 = hasher2.finish();
    let bytes = [
        (h1 >> 56) as u8,
        (h1 >> 48) as u8,
        (h1 >> 40) as u8,
        (h1 >> 32) as u8,
        (h1 >> 24) as u8,
        (h1 >> 16) as u8,
        (h1 >> 8) as u8,
        h1 as u8,
        (h2 >> 56) as u8,
        (h2 >> 48) as u8,
        (h2 >> 40) as u8,
        (h2 >> 32) as u8,
        (h2 >> 24) as u8,
        (h2 >> 16) as u8,
        (h2 >> 8) as u8,
        h2 as u8,
    ];
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}

/// `Duration::is_zero` shim — `Duration` exposes `is_zero` only on stable
/// 1.53+, but our MSRV already covers that. Re-exporting keeps call sites
/// terse if we ever need to swap implementations.
#[allow(dead_code)]
fn is_zero(d: Duration) -> bool {
    d.is_zero()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermesllm::apis::anthropic::MessagesMessage;

    fn req(model: &str, user: &str, system: Option<&str>) -> MessagesRequest {
        MessagesRequest {
            model: model.to_string(),
            messages: vec![MessagesMessage {
                role: MessagesRole::User,
                content: MessagesMessageContent::Single(user.to_string()),
            }],
            max_tokens: 1024,
            container: None,
            mcp_servers: None,
            system: system.map(|s| MessagesSystemPrompt::Single(s.to_string())),
            metadata: None,
            service_tier: None,
            thinking: None,
            temperature: None,
            top_p: None,
            top_k: None,
            stream: Some(true),
            stop_sequences: None,
            tools: None,
            tool_choice: None,
        }
    }

    #[test]
    fn header_uuid_is_used_as_is() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let r = req("sonnet", "hi", None);
        assert_eq!(SessionManager::resolve_session_id(Some(id), &r), id);
    }

    #[test]
    fn header_non_uuid_is_normalized_to_uuid() {
        let r = req("sonnet", "hi", None);
        let id = SessionManager::resolve_session_id(Some("my-token"), &r);
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        let id2 = SessionManager::resolve_session_id(Some("my-token"), &r);
        assert_eq!(id, id2);
    }

    #[test]
    fn empty_header_falls_back_to_hash() {
        let r = req("sonnet", "hi", Some("you are helpful"));
        let id = SessionManager::resolve_session_id(Some(""), &r);
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        let id2 = SessionManager::resolve_session_id(None, &r);
        assert_eq!(id, id2);
    }

    #[test]
    fn hash_is_stable_across_repeats_and_distinct_across_inputs() {
        let r1 = req("sonnet", "hello", None);
        let r2 = req("sonnet", "hello", None);
        let r3 = req("sonnet", "different", None);
        let r4 = req("opus", "hello", None);
        assert_eq!(
            SessionManager::resolve_session_id(None, &r1),
            SessionManager::resolve_session_id(None, &r2)
        );
        assert_ne!(
            SessionManager::resolve_session_id(None, &r1),
            SessionManager::resolve_session_id(None, &r3)
        );
        assert_ne!(
            SessionManager::resolve_session_id(None, &r1),
            SessionManager::resolve_session_id(None, &r4)
        );
    }
}
