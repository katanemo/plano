use super::models::{Pipe, Project, SpendingCounter, SpendingLimit, User};
use chrono::{Datelike, NaiveDate, Utc};
use tokio_postgres::Client;
use uuid::Uuid;

/// Resolved auth context from a proxy token
#[derive(Debug, Clone)]
pub struct TokenResolution {
    pub token_id: Uuid,
    pub user_id: Uuid,
    pub project_id: Uuid,
    pub user_email: String,
    pub project_name: String,
    pub pipes: Vec<PipeInfo>,
}

/// Pipe info returned from DB resolution
#[derive(Debug, Clone)]
pub struct PipeInfo {
    pub id: Uuid,
    pub name: String,
    pub provider: String,
    pub api_key_encrypted: String,
    pub model_filter: Option<String>,
}

/// Resolve a token hash to its user, project, and pipes
pub async fn resolve_token_by_hash(
    client: &Client,
    token_hash: &str,
) -> Result<Option<TokenResolution>, tokio_postgres::Error> {
    let row = client
        .query_opt(
            r#"
            SELECT
                pt.id as token_id,
                u.id as user_id,
                u.email as user_email,
                p.id as project_id,
                p.name as project_name
            FROM proxy_tokens pt
            JOIN projects p ON p.id = pt.project_id AND p.is_active = true
            JOIN users u ON u.id = p.user_id AND u.is_active = true
            WHERE pt.token_hash = $1
              AND pt.is_active = true
              AND (pt.expires_at IS NULL OR pt.expires_at > NOW())
            "#,
            &[&token_hash],
        )
        .await?;

    let row = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    let token_id: Uuid = row.get("token_id");
    let user_id: Uuid = row.get("user_id");
    let project_id: Uuid = row.get("project_id");
    let user_email: String = row.get("user_email");
    let project_name: String = row.get("project_name");

    // Update last_used_at
    let _ = client
        .execute(
            "UPDATE proxy_tokens SET last_used_at = NOW() WHERE id = $1",
            &[&token_id],
        )
        .await;

    // Fetch active pipes for the project
    let pipe_rows = client
        .query(
            r#"
            SELECT id, name, provider, api_key_encrypted, model_filter
            FROM pipes
            WHERE project_id = $1 AND is_active = true
            "#,
            &[&project_id],
        )
        .await?;

    let pipes: Vec<PipeInfo> = pipe_rows
        .iter()
        .map(|r| PipeInfo {
            id: r.get("id"),
            name: r.get("name"),
            provider: r.get("provider"),
            api_key_encrypted: r.get("api_key_encrypted"),
            model_filter: r.get("model_filter"),
        })
        .collect();

    Ok(Some(TokenResolution {
        token_id,
        user_id,
        project_id,
        user_email,
        project_name,
        pipes,
    }))
}

/// Get spending limits for a given entity
pub async fn get_spending_limits(
    client: &Client,
    entity_type: &str,
    entity_id: Uuid,
) -> Result<Vec<SpendingLimit>, tokio_postgres::Error> {
    let rows = client
        .query(
            r#"
            SELECT id, entity_type, entity_id, period_type, limit_cents,
                   is_active, created_at, updated_at
            FROM spending_limits
            WHERE entity_type = $1 AND entity_id = $2 AND is_active = true
            "#,
            &[&entity_type, &entity_id],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|r| SpendingLimit {
            id: r.get("id"),
            entity_type: r.get("entity_type"),
            entity_id: r.get("entity_id"),
            period_type: r.get("period_type"),
            limit_cents: r.get("limit_cents"),
            is_active: r.get("is_active"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

/// Batch insert usage records
pub async fn insert_usage_batch(
    client: &Client,
    records: &[(
        Uuid,
        Uuid,
        Uuid,
        Option<Uuid>,
        String,
        String,
        i32,
        i32,
        f64,
        bool,
        Option<i32>,
        Option<String>,
    )],
) -> Result<u64, tokio_postgres::Error> {
    if records.is_empty() {
        return Ok(0);
    }

    let stmt = client
        .prepare(
            r#"
            INSERT INTO usage_log
                (user_id, project_id, pipe_id, token_id, provider, model,
                 input_tokens, output_tokens, cost_cents, is_streaming,
                 status_code, request_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            "#,
        )
        .await?;

    let mut count = 0u64;
    for rec in records {
        count += client
            .execute(
                &stmt,
                &[
                    &rec.0, &rec.1, &rec.2, &rec.3, &rec.4, &rec.5, &rec.6, &rec.7, &rec.8, &rec.9,
                    &rec.10, &rec.11,
                ],
            )
            .await?;
    }
    Ok(count)
}

/// Upsert spending counters
pub async fn upsert_spending_counters(
    client: &Client,
    counters: &[(String, Uuid, String, NaiveDate, i64)],
) -> Result<(), tokio_postgres::Error> {
    if counters.is_empty() {
        return Ok(());
    }

    let stmt = client
        .prepare(
            r#"
            INSERT INTO spending_counters
                (entity_type, entity_id, period_type, period_start, spent_micro_cents, updated_at)
            VALUES ($1, $2, $3, $4, $5, NOW())
            ON CONFLICT (entity_type, entity_id, period_type, period_start)
            DO UPDATE SET
                spent_micro_cents = spending_counters.spent_micro_cents + EXCLUDED.spent_micro_cents,
                updated_at = NOW()
            "#,
        )
        .await?;

    for counter in counters {
        client
            .execute(
                &stmt,
                &[&counter.0, &counter.1, &counter.2, &counter.3, &counter.4],
            )
            .await?;
    }
    Ok(())
}

/// Load current period counters from DB (for startup hydration)
pub async fn load_current_counters(
    client: &Client,
) -> Result<Vec<SpendingCounter>, tokio_postgres::Error> {
    let today = Utc::now().date_naive();
    let month_start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);

    let rows = client
        .query(
            r#"
            SELECT id, entity_type, entity_id, period_type, period_start,
                   spent_micro_cents, updated_at
            FROM spending_counters
            WHERE (period_type = 'daily' AND period_start = $1)
               OR (period_type = 'monthly' AND period_start = $2)
            "#,
            &[&today, &month_start],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|r| SpendingCounter {
            id: r.get("id"),
            entity_type: r.get("entity_type"),
            entity_id: r.get("entity_id"),
            period_type: r.get("period_type"),
            period_start: r.get("period_start"),
            spent_micro_cents: r.get("spent_micro_cents"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

/// Create a new user
pub async fn create_user(
    client: &Client,
    email: &str,
    password_hash: &str,
    display_name: Option<&str>,
) -> Result<User, tokio_postgres::Error> {
    let row = client
        .query_one(
            r#"
            INSERT INTO users (email, password_hash, display_name)
            VALUES ($1, $2, $3)
            RETURNING id, email, password_hash, display_name, is_active, created_at, updated_at
            "#,
            &[&email, &password_hash, &display_name],
        )
        .await?;

    Ok(User {
        id: row.get("id"),
        email: row.get("email"),
        password_hash: row.get("password_hash"),
        display_name: row.get("display_name"),
        is_active: row.get("is_active"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// Get user by email
pub async fn get_user_by_email(
    client: &Client,
    email: &str,
) -> Result<Option<User>, tokio_postgres::Error> {
    let row = client
        .query_opt(
            r#"
            SELECT id, email, password_hash, display_name, is_active, created_at, updated_at
            FROM users WHERE email = $1
            "#,
            &[&email],
        )
        .await?;

    Ok(row.map(|r| User {
        id: r.get("id"),
        email: r.get("email"),
        password_hash: r.get("password_hash"),
        display_name: r.get("display_name"),
        is_active: r.get("is_active"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }))
}

/// Create a new project
pub async fn create_project(
    client: &Client,
    user_id: Uuid,
    name: &str,
    description: Option<&str>,
) -> Result<Project, tokio_postgres::Error> {
    let row = client
        .query_one(
            r#"
            INSERT INTO projects (user_id, name, description)
            VALUES ($1, $2, $3)
            RETURNING id, user_id, name, description, is_active, created_at, updated_at
            "#,
            &[&user_id, &name, &description],
        )
        .await?;

    Ok(Project {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        description: row.get("description"),
        is_active: row.get("is_active"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// List projects for a user
pub async fn list_projects(
    client: &Client,
    user_id: Uuid,
) -> Result<Vec<Project>, tokio_postgres::Error> {
    let rows = client
        .query(
            r#"
            SELECT id, user_id, name, description, is_active, created_at, updated_at
            FROM projects WHERE user_id = $1 AND is_active = true
            ORDER BY created_at DESC
            "#,
            &[&user_id],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|r| Project {
            id: r.get("id"),
            user_id: r.get("user_id"),
            name: r.get("name"),
            description: r.get("description"),
            is_active: r.get("is_active"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

/// Create a new pipe
pub async fn create_pipe(
    client: &Client,
    project_id: Uuid,
    name: &str,
    provider: &str,
    api_key_encrypted: &str,
    model_filter: Option<&str>,
) -> Result<Pipe, tokio_postgres::Error> {
    let row = client
        .query_one(
            r#"
            INSERT INTO pipes (project_id, name, provider, api_key_encrypted, model_filter)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, project_id, name, provider, api_key_encrypted, model_filter,
                      is_active, created_at, updated_at
            "#,
            &[
                &project_id,
                &name,
                &provider,
                &api_key_encrypted,
                &model_filter,
            ],
        )
        .await?;

    Ok(Pipe {
        id: row.get("id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        provider: row.get("provider"),
        api_key_encrypted: row.get("api_key_encrypted"),
        model_filter: row.get("model_filter"),
        is_active: row.get("is_active"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

/// List pipes for a project
pub async fn list_pipes(
    client: &Client,
    project_id: Uuid,
) -> Result<Vec<Pipe>, tokio_postgres::Error> {
    let rows = client
        .query(
            r#"
            SELECT id, project_id, name, provider, api_key_encrypted, model_filter,
                   is_active, created_at, updated_at
            FROM pipes WHERE project_id = $1 AND is_active = true
            ORDER BY created_at DESC
            "#,
            &[&project_id],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|r| Pipe {
            id: r.get("id"),
            project_id: r.get("project_id"),
            name: r.get("name"),
            provider: r.get("provider"),
            api_key_encrypted: r.get("api_key_encrypted"),
            model_filter: r.get("model_filter"),
            is_active: r.get("is_active"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

/// Create a proxy token (store only the hash)
pub async fn create_proxy_token(
    client: &Client,
    project_id: Uuid,
    token_hash: &str,
    name: &str,
) -> Result<Uuid, tokio_postgres::Error> {
    let row = client
        .query_one(
            r#"
            INSERT INTO proxy_tokens (project_id, token_hash, name)
            VALUES ($1, $2, $3)
            RETURNING id
            "#,
            &[&project_id, &token_hash, &name],
        )
        .await?;

    Ok(row.get("id"))
}

/// Revoke a proxy token
pub async fn revoke_proxy_token(
    client: &Client,
    token_id: Uuid,
    project_id: Uuid,
) -> Result<bool, tokio_postgres::Error> {
    let count = client
        .execute(
            "UPDATE proxy_tokens SET is_active = false WHERE id = $1 AND project_id = $2",
            &[&token_id, &project_id],
        )
        .await?;
    Ok(count > 0)
}

/// Upsert a spending limit
pub async fn upsert_spending_limit(
    client: &Client,
    entity_type: &str,
    entity_id: Uuid,
    period_type: &str,
    limit_cents: i64,
) -> Result<SpendingLimit, tokio_postgres::Error> {
    let row = client
        .query_one(
            r#"
            INSERT INTO spending_limits (entity_type, entity_id, period_type, limit_cents)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (entity_type, entity_id, period_type)
            DO UPDATE SET limit_cents = EXCLUDED.limit_cents, updated_at = NOW()
            RETURNING id, entity_type, entity_id, period_type, limit_cents,
                      is_active, created_at, updated_at
            "#,
            &[&entity_type, &entity_id, &period_type, &limit_cents],
        )
        .await?;

    Ok(SpendingLimit {
        id: row.get("id"),
        entity_type: row.get("entity_type"),
        entity_id: row.get("entity_id"),
        period_type: row.get("period_type"),
        limit_cents: row.get("limit_cents"),
        is_active: row.get("is_active"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// === Firewall mode queries ===

/// Register an API key (stores hash only)
pub async fn register_api_key(
    client: &Client,
    project_id: Uuid,
    key_hash: &str,
    provider: &str,
    upstream_url: &str,
    display_name: Option<&str>,
    egress_ip: &str,
) -> Result<Uuid, tokio_postgres::Error> {
    let row = client
        .query_one(
            r#"
            INSERT INTO registered_api_keys (project_id, key_hash, provider, upstream_url, display_name, egress_ip)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
            &[&project_id, &key_hash, &provider, &upstream_url, &display_name, &egress_ip],
        )
        .await?;
    Ok(row.get("id"))
}

/// List registered API keys for a project (no hash returned)
pub async fn list_registered_api_keys(
    client: &Client,
    project_id: Uuid,
) -> Result<Vec<RegisteredApiKeyRow>, tokio_postgres::Error> {
    let rows = client
        .query(
            r#"
            SELECT id, project_id, provider, upstream_url, display_name, is_active, created_at
            FROM registered_api_keys
            WHERE project_id = $1
            ORDER BY created_at DESC
            "#,
            &[&project_id],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|r| RegisteredApiKeyRow {
            id: r.get("id"),
            project_id: r.get("project_id"),
            provider: r.get("provider"),
            upstream_url: r.get("upstream_url"),
            display_name: r.get("display_name"),
            is_active: r.get("is_active"),
            created_at: r.get("created_at"),
        })
        .collect())
}

/// Delete a registered API key
pub async fn delete_registered_api_key(
    client: &Client,
    key_id: Uuid,
    project_id: Uuid,
) -> Result<bool, tokio_postgres::Error> {
    let count = client
        .execute(
            "DELETE FROM registered_api_keys WHERE id = $1 AND project_id = $2",
            &[&key_id, &project_id],
        )
        .await?;
    Ok(count > 0)
}

/// Row type for registered API key listings
#[derive(Debug, Clone)]
pub struct RegisteredApiKeyRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub provider: String,
    pub upstream_url: String,
    pub display_name: Option<String>,
    pub is_active: bool,
    pub created_at: chrono::DateTime<Utc>,
}

/// Upsert custom model pricing
pub async fn upsert_custom_pricing(
    client: &Client,
    project_id: Uuid,
    provider: &str,
    model: &str,
    input_price_per_million: f64,
    output_price_per_million: f64,
) -> Result<Uuid, tokio_postgres::Error> {
    let row = client
        .query_one(
            r#"
            INSERT INTO custom_model_pricing (project_id, provider, model, input_price_per_million, output_price_per_million)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (project_id, provider, model)
            DO UPDATE SET
                input_price_per_million = EXCLUDED.input_price_per_million,
                output_price_per_million = EXCLUDED.output_price_per_million
            RETURNING id
            "#,
            &[&project_id, &provider, &model, &input_price_per_million, &output_price_per_million],
        )
        .await?;
    Ok(row.get("id"))
}

/// List custom pricing for a project (or global if project_id is None)
pub async fn list_custom_pricing(
    client: &Client,
    project_id: Option<Uuid>,
) -> Result<Vec<CustomPricingRow>, tokio_postgres::Error> {
    let rows = match project_id {
        Some(pid) => {
            client
                .query(
                    r#"
                    SELECT id, project_id, provider, model, input_price_per_million, output_price_per_million, created_at
                    FROM custom_model_pricing
                    WHERE project_id = $1
                    ORDER BY provider, model
                    "#,
                    &[&pid],
                )
                .await?
        }
        None => {
            client
                .query(
                    r#"
                    SELECT id, project_id, provider, model, input_price_per_million, output_price_per_million, created_at
                    FROM custom_model_pricing
                    WHERE project_id IS NULL
                    ORDER BY provider, model
                    "#,
                    &[],
                )
                .await?
        }
    };

    Ok(rows
        .iter()
        .map(|r| CustomPricingRow {
            id: r.get("id"),
            project_id: r.get("project_id"),
            provider: r.get("provider"),
            model: r.get("model"),
            input_price_per_million: r.get("input_price_per_million"),
            output_price_per_million: r.get("output_price_per_million"),
            created_at: r.get("created_at"),
        })
        .collect())
}

/// Row type for custom pricing
#[derive(Debug, Clone)]
pub struct CustomPricingRow {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub provider: String,
    pub model: String,
    pub input_price_per_million: f64,
    pub output_price_per_million: f64,
    pub created_at: chrono::DateTime<Utc>,
}

/// Lookup custom pricing for a specific model (project override first, then global)
pub async fn get_custom_pricing(
    client: &Client,
    project_id: Uuid,
    provider: &str,
    model: &str,
) -> Result<Option<CustomPricingRow>, tokio_postgres::Error> {
    // Try project-specific pricing first
    let row = client
        .query_opt(
            r#"
            SELECT id, project_id, provider, model, input_price_per_million, output_price_per_million, created_at
            FROM custom_model_pricing
            WHERE project_id = $1 AND provider = $2 AND model = $3
            "#,
            &[&project_id, &provider, &model],
        )
        .await?;

    if let Some(r) = row {
        return Ok(Some(CustomPricingRow {
            id: r.get("id"),
            project_id: r.get("project_id"),
            provider: r.get("provider"),
            model: r.get("model"),
            input_price_per_million: r.get("input_price_per_million"),
            output_price_per_million: r.get("output_price_per_million"),
            created_at: r.get("created_at"),
        }));
    }

    // Fall back to global pricing (project_id IS NULL)
    let row = client
        .query_opt(
            r#"
            SELECT id, project_id, provider, model, input_price_per_million, output_price_per_million, created_at
            FROM custom_model_pricing
            WHERE project_id IS NULL AND provider = $1 AND model = $2
            "#,
            &[&provider, &model],
        )
        .await?;

    Ok(row.map(|r| CustomPricingRow {
        id: r.get("id"),
        project_id: r.get("project_id"),
        provider: r.get("provider"),
        model: r.get("model"),
        input_price_per_million: r.get("input_price_per_million"),
        output_price_per_million: r.get("output_price_per_million"),
        created_at: r.get("created_at"),
    }))
}

/// Insert a firewall-mode usage record (no user_id or pipe_id required)
pub async fn insert_firewall_usage_batch(
    client: &Client,
    records: &[(
        Uuid,
        String,
        String,
        i32,
        i32,
        bool,
        Option<i32>,
        Option<String>,
    )],
) -> Result<u64, tokio_postgres::Error> {
    if records.is_empty() {
        return Ok(0);
    }

    let stmt = client
        .prepare(
            r#"
            INSERT INTO usage_log
                (project_id, provider, model,
                 input_tokens, output_tokens, is_streaming,
                 status_code, request_id, is_priced)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, false)
            "#,
        )
        .await?;

    let mut count = 0u64;
    for rec in records {
        count += client
            .execute(
                &stmt,
                &[
                    &rec.0, &rec.1, &rec.2, &rec.3, &rec.4, &rec.5, &rec.6, &rec.7,
                ],
            )
            .await?;
    }
    Ok(count)
}

/// Fetch unpriced usage records for background pricing
pub async fn get_unpriced_usage(
    client: &Client,
    limit: i64,
) -> Result<Vec<UnpricedUsageRow>, tokio_postgres::Error> {
    let rows = client
        .query(
            r#"
            SELECT id, project_id, provider, model, input_tokens, output_tokens
            FROM usage_log
            WHERE is_priced = false
            ORDER BY created_at ASC
            LIMIT $1
            "#,
            &[&limit],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|r| UnpricedUsageRow {
            id: r.get("id"),
            project_id: r.get("project_id"),
            provider: r.get("provider"),
            model: r.get("model"),
            input_tokens: r.get("input_tokens"),
            output_tokens: r.get("output_tokens"),
        })
        .collect())
}

/// Row type for unpriced usage
#[derive(Debug, Clone)]
pub struct UnpricedUsageRow {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub provider: String,
    pub model: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
}

/// Mark usage records as priced with a calculated cost
pub async fn mark_usage_priced(
    client: &Client,
    updates: &[(Uuid, f64)],
) -> Result<(), tokio_postgres::Error> {
    if updates.is_empty() {
        return Ok(());
    }

    let stmt = client
        .prepare("UPDATE usage_log SET cost_cents = $1, is_priced = true WHERE id = $2")
        .await?;

    for (id, cost_cents) in updates {
        client.execute(&stmt, &[cost_cents, id]).await?;
    }
    Ok(())
}

/// Get all active spending limits (for budget checker background task)
/// Get the current cumulative spending for an entity/period from the DB.
pub async fn get_current_spending(
    client: &Client,
    entity_type: &str,
    entity_id: Uuid,
    period_type: &str,
) -> Result<i64, tokio_postgres::Error> {
    let today = chrono::Utc::now().date_naive();
    let period_start = match period_type {
        "daily" => today,
        "monthly" => {
            chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today)
        }
        _ => today,
    };

    let row = client
        .query_opt(
            r#"
            SELECT spent_micro_cents FROM spending_counters
            WHERE entity_type = $1 AND entity_id = $2
              AND period_type = $3 AND period_start = $4
            "#,
            &[&entity_type, &entity_id, &period_type, &period_start],
        )
        .await?;

    Ok(row
        .map(|r| r.get::<_, i64>("spent_micro_cents"))
        .unwrap_or(0))
}

pub async fn get_all_active_spending_limits(
    client: &Client,
) -> Result<Vec<SpendingLimit>, tokio_postgres::Error> {
    let rows = client
        .query(
            r#"
            SELECT id, entity_type, entity_id, period_type, limit_cents,
                   is_active, created_at, updated_at
            FROM spending_limits
            WHERE is_active = true
            "#,
            &[],
        )
        .await?;

    Ok(rows
        .iter()
        .map(|r| SpendingLimit {
            id: r.get("id"),
            entity_type: r.get("entity_type"),
            entity_id: r.get("entity_id"),
            period_type: r.get("period_type"),
            limit_cents: r.get("limit_cents"),
            is_active: r.get("is_active"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}
