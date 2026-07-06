//! Implicit session-affinity key derivation.
//!
//! When a request carries no explicit `X-Model-Affinity` header, Plano derives a
//! stable session key from the parts of the prompt that repeat verbatim at the head
//! of every turn — the same bytes the provider's prompt cache is keyed on:
//!
//! ```text
//! session_key  = hash(system + tools + first_user_message)
//! prefix_hash  = hash(system + tools)
//! ```
//!
//! The session key is constant for the life of a conversation (history grows at the
//! tail, not the head), so turns 2+ reuse the same pin without any client changes.
//! The prefix hash covers only the fully-stable segment and is stored with the pin
//! for drift detection: if it changes, the provider cache is already lost and
//! re-routing fresh is safe.
//!
//! Only salted hashes are ever stored — never prompt content.

use hermesllm::apis::openai::{Message, Role};

/// Salt folded into every hash so stored keys can't be trivially correlated with
/// prompt content across systems. Deterministic across processes/replicas so a
/// shared Redis session cache keys consistently.
const HASH_SALT: &str = "plano-affinity-v1";

/// Prefix distinguishing derived keys from client-supplied `X-Model-Affinity` ids.
const IMPLICIT_KEY_PREFIX: &str = "implicit:";

/// Derived affinity identifiers for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplicitAffinity {
    /// Session-cache key: `implicit:{hex}` over system + tools + first user message.
    pub session_key: String,
    /// Hash of the stable prefix only (system + tools), for drift detection.
    pub prefix_hash: u64,
}

/// FNV-1a 64-bit — stable across processes and Rust versions (unlike `DefaultHasher`),
/// dependency-free, and plenty for cache keying (collisions merely over-pin, which is
/// cache-friendly).
fn fnv1a64(chunks: &[&str]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(PRIME);
        }
        // Field separator so ("ab","c") and ("a","bc") hash differently.
        hash ^= 0x1f;
        hash = hash.wrapping_mul(PRIME);
    };
    feed(HASH_SALT.as_bytes());
    for chunk in chunks {
        feed(chunk.as_bytes());
    }
    hash
}

/// Derive the implicit affinity key from parsed request messages and tool names.
///
/// Returns `None` when there is no user message to anchor on (nothing distinguishes
/// the conversation, so pinning would be meaningless).
pub fn derive_implicit_affinity(
    messages: &[Message],
    tool_names: Option<&[String]>,
    tenant_id: Option<&str>,
) -> Option<ImplicitAffinity> {
    let system_text: String = messages
        .iter()
        .filter(|m| matches!(m.role, Role::System | Role::Developer))
        .filter_map(|m| m.content.as_ref().map(|c| c.to_string()))
        .collect::<Vec<_>>()
        .join("\n");

    let first_user = messages
        .iter()
        .find(|m| matches!(m.role, Role::User))
        .and_then(|m| m.content.as_ref().map(|c| c.to_string()))?;

    let tools_text = tool_names.map(|names| names.join(",")).unwrap_or_default();
    let tenant = tenant_id.unwrap_or_default();

    let prefix_hash = fnv1a64(&[tenant, &system_text, &tools_text]);
    let session_hash = fnv1a64(&[tenant, &system_text, &tools_text, &first_user]);

    Some(ImplicitAffinity {
        session_key: format!("{IMPLICIT_KEY_PREFIX}{session_hash:016x}"),
        prefix_hash,
    })
}

/// Estimate the token count of the stable prompt prefix (system + tools + all but the
/// final message) using a chars/4 heuristic. Used by cache-aware ranking to weigh the
/// cost of re-sending vs re-reading the prefix; precision is not required.
pub fn estimate_prefix_tokens(messages: &[Message]) -> u64 {
    let upto = messages.len().saturating_sub(1);
    let chars: usize = messages[..upto]
        .iter()
        .filter_map(|m| m.content.as_ref().map(|c| c.to_string().len()))
        .sum();
    (chars / 4) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermesllm::apis::openai::MessageContent;

    fn msg(role: Role, text: &str) -> Message {
        Message {
            role,
            content: Some(MessageContent::Text(text.to_string())),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    #[test]
    fn key_is_stable_as_history_grows() {
        let turn1 = vec![
            msg(Role::System, "You are a coding agent."),
            msg(Role::User, "Fix the bug in main.rs"),
        ];
        let turn5 = vec![
            msg(Role::System, "You are a coding agent."),
            msg(Role::User, "Fix the bug in main.rs"),
            msg(Role::Assistant, "Done. Anything else?"),
            msg(Role::User, "Now add tests"),
            msg(Role::Assistant, "Added."),
        ];

        let a1 = derive_implicit_affinity(&turn1, None, None).unwrap();
        let a5 = derive_implicit_affinity(&turn5, None, None).unwrap();
        assert_eq!(a1.session_key, a5.session_key);
        assert_eq!(a1.prefix_hash, a5.prefix_hash);
        assert!(a1.session_key.starts_with("implicit:"));
    }

    #[test]
    fn different_first_user_message_yields_different_session() {
        let base = msg(Role::System, "You are a coding agent.");
        let a = derive_implicit_affinity(
            &[base.clone(), msg(Role::User, "conversation A")],
            None,
            None,
        )
        .unwrap();
        let b = derive_implicit_affinity(&[base, msg(Role::User, "conversation B")], None, None)
            .unwrap();
        // Same stable prefix, different conversations.
        assert_eq!(a.prefix_hash, b.prefix_hash);
        assert_ne!(a.session_key, b.session_key);
    }

    #[test]
    fn changed_system_prompt_changes_prefix_hash() {
        let a = derive_implicit_affinity(
            &[msg(Role::System, "v1 prompt"), msg(Role::User, "hi")],
            None,
            None,
        )
        .unwrap();
        let b = derive_implicit_affinity(
            &[msg(Role::System, "v2 prompt"), msg(Role::User, "hi")],
            None,
            None,
        )
        .unwrap();
        assert_ne!(a.prefix_hash, b.prefix_hash);
        assert_ne!(a.session_key, b.session_key);
    }

    #[test]
    fn tools_and_tenant_are_part_of_the_key() {
        let messages = [msg(Role::System, "s"), msg(Role::User, "u")];
        let plain = derive_implicit_affinity(&messages, None, None).unwrap();
        let with_tools =
            derive_implicit_affinity(&messages, Some(&["get_weather".to_string()]), None).unwrap();
        let with_tenant = derive_implicit_affinity(&messages, None, Some("acme")).unwrap();
        assert_ne!(plain.session_key, with_tools.session_key);
        assert_ne!(plain.session_key, with_tenant.session_key);
    }

    #[test]
    fn no_user_message_yields_none() {
        assert!(derive_implicit_affinity(&[msg(Role::System, "s")], None, None).is_none());
        assert!(derive_implicit_affinity(&[], None, None).is_none());
    }

    #[test]
    fn prefix_token_estimate_excludes_final_turn() {
        let messages = [
            msg(Role::System, &"x".repeat(4000)),
            msg(Role::User, &"y".repeat(400)),
            msg(Role::User, "new turn"),
        ];
        assert_eq!(estimate_prefix_tokens(&messages), 1100);
        assert_eq!(estimate_prefix_tokens(&[]), 0);
    }
}
