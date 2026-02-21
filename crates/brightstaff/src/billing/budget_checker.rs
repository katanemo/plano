use crate::db::queries::get_all_active_spending_limits;
use crate::db::DbPool;
use dashmap::DashSet;
use std::sync::Arc;
use tracing::{debug, error};
use uuid::Uuid;

/// Background task that checks spending limits and maintains a set of blocked projects.
/// WASM filters poll GET /budget/blocked to get this set.
#[derive(Clone)]
pub struct BudgetChecker {
    blocked_projects: Arc<DashSet<Uuid>>,
}

impl Default for BudgetChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl BudgetChecker {
    pub fn new() -> Self {
        Self {
            blocked_projects: Arc::new(DashSet::new()),
        }
    }

    /// Get the list of currently blocked project IDs
    pub fn get_blocked_projects(&self) -> Vec<Uuid> {
        self.blocked_projects.iter().map(|r| *r).collect()
    }

    /// Check if a project is blocked
    pub fn is_blocked(&self, project_id: &Uuid) -> bool {
        self.blocked_projects.contains(project_id)
    }

    /// Start the background checking task
    pub fn start(self, pool: DbPool, interval_secs: u64) {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

            loop {
                interval.tick().await;

                match self.check_budgets(&pool).await {
                    Ok(blocked_count) => {
                        if blocked_count > 0 {
                            debug!(blocked = blocked_count, "budget check complete");
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "budget checker error");
                    }
                }
            }
        });
    }

    async fn check_budgets(&self, pool: &DbPool) -> Result<usize, Box<dyn std::error::Error>> {
        let client = pool.get_client().await?;
        let limits = get_all_active_spending_limits(&client).await?;

        let newly_blocked: DashSet<Uuid> = DashSet::new();

        for limit in &limits {
            let limit_micro_cents = limit.limit_cents * 10_000; // cents -> micro-cents

            // Query cumulative spending from DB (not in-memory deltas)
            let spent = crate::db::queries::get_current_spending(
                &client,
                &limit.entity_type,
                limit.entity_id,
                &limit.period_type,
            )
            .await
            .unwrap_or(0);

            let is_within_budget = spent < limit_micro_cents;

            if !is_within_budget && limit.entity_type == "project" {
                newly_blocked.insert(limit.entity_id);
            }
        }

        // Replace the blocked set atomically
        self.blocked_projects.clear();
        for id in newly_blocked.iter() {
            self.blocked_projects.insert(*id);
        }

        Ok(self.blocked_projects.len())
    }
}
