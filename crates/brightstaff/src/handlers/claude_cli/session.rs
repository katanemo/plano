//! Session manager for the claude-cli bridge. Maps a stable session id (taken
//! from a client-provided header or hashed from the conversation prefix) to a
//! long-lived `ClaudeProcess`. Enforces an idle TTL and a hard cap on the
//! number of concurrent sessions.

use std::collections::HashMap;
use std::sync::Arc;

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
        // Build a deterministic seed from (model, system_prompt, first user
        // message) so a retried conversation lands on the same session.
        let mut seed = String::new();
        seed.push_str(&req.model);
        seed.push('\u{1f}');
        if let Some(system) = &req.system {
            seed.push_str(&system_text(system));
        }
        seed.push('\u{1f}');
        if let Some(first) = first_user_message_text(req) {
            seed.push_str(&first);
        }
        uuid_from_seed(&seed)
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

        // Single lock acquisition for the whole get-or-spawn path. `last_used`
        // is now a sync mutex on `ClaudeProcess`, so iterating to find the
        // LRU victim does not block other tasks across an `.await`.
        let mut map = self.inner.lock().await;

        if let Some(existing) = map.get(session_id) {
            debug!(session = %session_id, "reusing claude-cli session");
            return Ok(Arc::clone(existing));
        }

        // If we are at the cap, take an LRU victim out of the map first so
        // its slot is freed before we insert. We drop the lock for the
        // shutdown await (killing a child can take a tick), accepting that
        // the cap can drift by one if a concurrent task spawns in that
        // window — the next reap will catch it.
        let victim = if map.len() >= self.config.max_sessions {
            let victim_key = lru_session_id(&map);
            victim_key.and_then(|k| map.remove(&k).map(|v| (k, v)))
        } else {
            None
        };

        // Spawn outside of any lock if we have to wait on a victim shutdown.
        let process = if let Some((victim_key, victim_proc)) = victim {
            drop(map);
            info!(session = %victim_key, "evicting LRU claude-cli session to make room");
            victim_proc.shutdown().await;
            let process = ClaudeProcess::spawn(
                session_id.to_string(),
                model,
                system_prompt,
                cwd,
                self.config.process.clone(),
            )
            .await?;
            self.inner
                .lock()
                .await
                .insert(session_id.to_string(), Arc::clone(&process));
            process
        } else {
            // No eviction needed — keep holding the map lock across spawn so
            // we don't race with another caller resolving the same id.
            let process = ClaudeProcess::spawn(
                session_id.to_string(),
                model,
                system_prompt,
                cwd,
                self.config.process.clone(),
            )
            .await?;
            map.insert(session_id.to_string(), Arc::clone(&process));
            process
        };

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

        // Collect victims under a single lock acquisition; `last_used()` is
        // sync, so the iteration never crosses an `.await`.
        let to_kill: Vec<(String, Arc<ClaudeProcess>)> = {
            let mut map = self.inner.lock().await;
            let keys: Vec<String> = map
                .iter()
                .filter(|(_, v)| now.duration_since(v.last_used()) > ttl)
                .map(|(k, _)| k.clone())
                .collect();
            keys.into_iter()
                .filter_map(|k| map.remove(&k).map(|v| (k, v)))
                .collect()
        };

        for (k, proc) in to_kill {
            info!(session = %k, "evicting idle claude-cli session");
            proc.shutdown().await;
        }
    }
}

/// Pick the least-recently-used session id from the map. Sync because
/// `ClaudeProcess::last_used` is sync.
fn lru_session_id(map: &HashMap<String, Arc<ClaudeProcess>>) -> Option<String> {
    map.iter()
        .min_by_key(|(_, v)| v.last_used())
        .map(|(k, _)| k.clone())
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

/// Deterministic UUIDv5 derived from an arbitrary seed string. The `claude`
/// CLI requires `--session-id` to be a valid UUID; v5 (SHA-1 based) gives
/// us a stable mapping across Rust toolchain versions, unlike `DefaultHasher`.
/// We use the OID namespace because the seed isn't a DNS or URL name.
fn uuid_from_seed(seed: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, seed.as_bytes()).to_string()
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
