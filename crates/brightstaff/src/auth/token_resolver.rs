use crate::db::queries::{resolve_token_by_hash, PipeInfo};
use crate::db::DbPool;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Context returned after successful token resolution
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user_id: Uuid,
    pub project_id: Uuid,
    pub token_id: Uuid,
    pub user_email: String,
    pub project_name: String,
    pub pipes: Vec<PipeInfo>,
}

/// Hash a bearer token to its SHA-256 hex digest
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Resolve a bearer token to an AuthContext
pub async fn resolve_token(pool: &DbPool, token: &str) -> Result<AuthContext, AuthError> {
    let token_hash = hash_token(token);
    let client = pool
        .get_client()
        .await
        .map_err(|e| AuthError::Internal(format!("failed to get db connection: {}", e)))?;

    let resolution = resolve_token_by_hash(&client, &token_hash)
        .await
        .map_err(|e| AuthError::Internal(format!("db query failed: {}", e)))?;

    match resolution {
        Some(res) => Ok(AuthContext {
            user_id: res.user_id,
            project_id: res.project_id,
            token_id: res.token_id,
            user_email: res.user_email,
            project_name: res.project_name,
            pipes: res.pipes,
        }),
        None => Err(AuthError::InvalidToken),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid or expired token")]
    InvalidToken,
    #[error("no pipe found for model: {0}")]
    NoPipeForModel(String),
    #[error("spending limit exceeded")]
    SpendingLimitExceeded,
    #[error("internal error: {0}")]
    Internal(String),
}
