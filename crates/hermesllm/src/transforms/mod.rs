//! API transformation modules
//!
//! This module provides organized transformations between the two main LLM API formats:
//! - `/v1/chat/completions` (OpenAI format)
//! - `/v1/messages` (Anthropic format)
//!
//! Provider-specific transformations (Bedrock, Groq, etc.) are handled internally
//! by the gateway, but the external API surface remains these two standard formats.
//! The transformations are split into logical modules for maintainability.

pub mod lib;
pub mod request;
pub mod response;
pub mod response_streaming;

// Re-export commonly used items for convenience
pub use lib::*;
#[allow(ambiguous_glob_reexports)]
pub use request::*;
pub use response::*;
pub use response_streaming::*;

// ============================================================================
// CONSTANTS
// ============================================================================

/// Default maximum tokens when converting from OpenAI to Anthropic and no max_tokens is specified
pub const DEFAULT_MAX_TOKENS: u32 = 4096;
