use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub display_name: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipe {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub provider: String,
    pub api_key_encrypted: String,
    pub model_filter: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyToken {
    pub id: Uuid,
    pub project_id: Uuid,
    pub token_hash: String,
    pub name: String,
    pub is_active: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingLimit {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub period_type: String,
    pub limit_cents: i64,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub project_id: Uuid,
    pub pipe_id: Uuid,
    pub token_id: Option<Uuid>,
    pub provider: String,
    pub model: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cost_cents: f64,
    pub is_streaming: bool,
    pub status_code: Option<i32>,
    pub request_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingCounter {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub period_type: String,
    pub period_start: NaiveDate,
    pub spent_micro_cents: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub id: Uuid,
    pub provider: String,
    pub model: String,
    pub input_price_per_token: f64,
    pub output_price_per_token: f64,
    pub source: String,
    pub updated_at: DateTime<Utc>,
}
