use super::counters::SpendingCounters;
use crate::db::queries::upsert_spending_counters;
use crate::db::DbPool;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

/// A single usage event to be flushed to the database
#[derive(Debug, Clone)]
pub struct UsageEvent {
    pub user_id: Option<Uuid>,
    pub project_id: Uuid,
    pub pipe_id: Option<Uuid>,
    pub token_id: Option<Uuid>,
    pub provider: String,
    pub model: String,
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub cost_cents: f64,
    pub is_streaming: bool,
    pub status_code: Option<i32>,
    pub request_id: Option<String>,
    pub is_priced: bool,
}

/// Usage flusher that batches writes to PostgreSQL
pub struct UsageFlusher {
    tx: mpsc::Sender<UsageEvent>,
}

impl UsageFlusher {
    /// Start the flusher background task. Returns a sender for enqueuing events.
    pub fn start(pool: DbPool, counters: SpendingCounters, flush_interval_secs: u64) -> Self {
        let (tx, rx) = mpsc::channel::<UsageEvent>(10_000);

        tokio::spawn(flush_loop(pool, counters, rx, flush_interval_secs));

        Self { tx }
    }

    /// Enqueue a usage event for batched writing
    pub async fn enqueue(
        &self,
        event: UsageEvent,
    ) -> Result<(), mpsc::error::SendError<UsageEvent>> {
        self.tx.send(event).await
    }

    pub fn sender(&self) -> mpsc::Sender<UsageEvent> {
        self.tx.clone()
    }
}

async fn flush_loop(
    pool: DbPool,
    counters: SpendingCounters,
    mut rx: mpsc::Receiver<UsageEvent>,
    flush_interval_secs: u64,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(flush_interval_secs));
    let mut pending_events: Vec<UsageEvent> = Vec::new();

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Drain any pending events from channel
                while let Ok(event) = rx.try_recv() {
                    pending_events.push(event);
                }

                if pending_events.is_empty() {
                    // Still flush counter deltas even if no new events
                    let deltas = counters.snapshot_and_reset();
                    if !deltas.is_empty() {
                        if let Err(e) = flush_counters(&pool, &deltas).await {
                            error!(error = %e, "failed to flush spending counters");
                            counters.restore_deltas(&deltas);
                        }
                    }
                    continue;
                }

                let batch: Vec<UsageEvent> = pending_events.drain(..).collect();
                let deltas = counters.snapshot_and_reset();

                match flush_batch(&pool, &batch, &deltas).await {
                    Ok(count) => {
                        info!(records = count, "flushed usage batch");
                    }
                    Err(e) => {
                        error!(error = %e, "failed to flush usage batch, re-adding deltas");
                        counters.restore_deltas(&deltas);
                        // Re-enqueue events (best effort)
                        for event in batch {
                            pending_events.push(event);
                        }
                    }
                }
            }
            Some(event) = rx.recv() => {
                pending_events.push(event);
                // If we have a large batch, flush immediately
                if pending_events.len() >= 1000 {
                    let batch: Vec<UsageEvent> = pending_events.drain(..).collect();
                    let deltas = counters.snapshot_and_reset();
                    match flush_batch(&pool, &batch, &deltas).await {
                        Ok(count) => {
                            info!(records = count, "flushed large usage batch");
                        }
                        Err(e) => {
                            warn!(error = %e, "failed to flush large batch");
                            counters.restore_deltas(&deltas);
                            for event in batch {
                                pending_events.push(event);
                            }
                        }
                    }
                }
            }
            else => {
                // Channel closed, flush remaining
                if !pending_events.is_empty() {
                    let batch: Vec<UsageEvent> = pending_events.drain(..).collect();
                    let deltas = counters.snapshot_and_reset();
                    let _ = flush_batch(&pool, &batch, &deltas).await;
                }
                info!("usage flusher shutting down");
                break;
            }
        }
    }
}

async fn flush_batch(
    pool: &DbPool,
    events: &[UsageEvent],
    counter_deltas: &[(super::counters::CounterKey, i64)],
) -> Result<u64, Box<dyn std::error::Error>> {
    let client = pool.get_client().await?;

    // Insert events with support for optional user_id/pipe_id and is_priced
    let stmt = client
        .prepare(
            r#"
            INSERT INTO usage_log
                (user_id, project_id, pipe_id, token_id, provider, model,
                 input_tokens, output_tokens, cost_cents, is_streaming,
                 status_code, request_id, is_priced)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
        )
        .await?;

    let mut count = 0u64;
    for e in events {
        count += client
            .execute(
                &stmt,
                &[
                    &e.user_id,
                    &e.project_id,
                    &e.pipe_id,
                    &e.token_id,
                    &e.provider,
                    &e.model,
                    &e.input_tokens,
                    &e.output_tokens,
                    &e.cost_cents,
                    &e.is_streaming,
                    &e.status_code,
                    &e.request_id,
                    &e.is_priced,
                ],
            )
            .await?;
    }

    // Flush counter deltas
    let counter_records: Vec<_> = counter_deltas
        .iter()
        .map(|(key, val)| {
            (
                key.entity_type.clone(),
                key.entity_id,
                key.period_type.clone(),
                key.period_start,
                *val,
            )
        })
        .collect();

    upsert_spending_counters(&client, &counter_records).await?;

    Ok(count)
}

async fn flush_counters(
    pool: &DbPool,
    deltas: &[(super::counters::CounterKey, i64)],
) -> Result<(), Box<dyn std::error::Error>> {
    let client = pool.get_client().await?;
    let counter_records: Vec<_> = deltas
        .iter()
        .map(|(key, val)| {
            (
                key.entity_type.clone(),
                key.entity_id,
                key.period_type.clone(),
                key.period_start,
                *val,
            )
        })
        .collect();
    upsert_spending_counters(&client, &counter_records).await?;
    Ok(())
}
