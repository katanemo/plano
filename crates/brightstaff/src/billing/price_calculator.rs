use crate::billing::counters::SpendingCounters;
use crate::db::queries::{get_unpriced_usage, mark_usage_priced};
use crate::db::DbPool;
use crate::pricing::PricingRegistry;
use tracing::{debug, error, info};

/// Background task that prices unpriced usage records.
/// Runs every `interval_secs` seconds, fetches unpriced rows, calculates cost,
/// and updates them as priced.
pub struct PriceCalculator;

impl PriceCalculator {
    pub fn start(
        pool: DbPool,
        pricing: PricingRegistry,
        counters: SpendingCounters,
        interval_secs: u64,
    ) {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

            loop {
                interval.tick().await;

                match price_batch(&pool, &pricing, &counters).await {
                    Ok(0) => {}
                    Ok(count) => {
                        info!(records = count, "priced usage records");
                    }
                    Err(e) => {
                        error!(error = %e, "price calculator error");
                    }
                }
            }
        });
    }
}

async fn price_batch(
    pool: &DbPool,
    pricing: &PricingRegistry,
    counters: &SpendingCounters,
) -> Result<usize, Box<dyn std::error::Error>> {
    let client = pool.get_client().await?;

    let unpriced = get_unpriced_usage(&client, 1000).await?;
    if unpriced.is_empty() {
        return Ok(0);
    }

    let mut updates = Vec::with_capacity(unpriced.len());

    for row in &unpriced {
        let cost_cents = if let Some(project_id) = row.project_id {
            pricing
                .calculate_cost_with_custom(
                    pool,
                    project_id,
                    &row.provider,
                    &row.model,
                    row.input_tokens,
                    row.output_tokens,
                )
                .await
        } else {
            pricing
                .calculate_cost(
                    &row.provider,
                    &row.model,
                    row.input_tokens,
                    row.output_tokens,
                )
                .await
        };

        updates.push((row.id, cost_cents));

        // Update in-memory counters with the calculated cost
        if let Some(project_id) = row.project_id {
            let cost_micro_cents = (cost_cents * 10_000.0) as i64;
            if cost_micro_cents > 0 {
                counters.record_usage("project", project_id, "daily", cost_micro_cents);
                counters.record_usage("project", project_id, "monthly", cost_micro_cents);
            }
        }

        debug!(
            id = %row.id,
            provider = %row.provider,
            model = %row.model,
            cost_cents = cost_cents,
            "priced usage record"
        );
    }

    let count = updates.len();
    mark_usage_priced(&client, &updates).await?;

    Ok(count)
}
