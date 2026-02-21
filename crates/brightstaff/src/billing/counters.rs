use chrono::{Datelike, NaiveDate, Utc};
use dashmap::DashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// Key for spending counter lookup
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct CounterKey {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub period_type: String,
    pub period_start: NaiveDate,
}

/// In-memory spending counters backed by DashMap + atomic values
#[derive(Clone)]
pub struct SpendingCounters {
    counters: Arc<DashMap<CounterKey, AtomicI64>>,
}

impl SpendingCounters {
    pub fn new() -> Self {
        Self {
            counters: Arc::new(DashMap::new()),
        }
    }

    /// Check if a budget would be exceeded.
    /// Returns true if within budget, false if would exceed.
    pub fn check_budget(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        period_type: &str,
        limit_micro_cents: i64,
    ) -> bool {
        let today = Utc::now().date_naive();
        let period_start = match period_type {
            "daily" => today,
            "monthly" => NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today),
            _ => return true,
        };

        let key = CounterKey {
            entity_type: entity_type.to_string(),
            entity_id,
            period_type: period_type.to_string(),
            period_start,
        };

        match self.counters.get(&key) {
            Some(val) => val.load(Ordering::Relaxed) < limit_micro_cents,
            None => true, // no spending yet
        }
    }

    /// Record usage in micro-cents. Returns the new total.
    pub fn record_usage(
        &self,
        entity_type: &str,
        entity_id: Uuid,
        period_type: &str,
        micro_cents: i64,
    ) -> i64 {
        let today = Utc::now().date_naive();
        let period_start = match period_type {
            "daily" => today,
            "monthly" => NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today),
            _ => today,
        };

        let key = CounterKey {
            entity_type: entity_type.to_string(),
            entity_id,
            period_type: period_type.to_string(),
            period_start,
        };

        self.counters
            .entry(key)
            .or_insert_with(|| AtomicI64::new(0))
            .fetch_add(micro_cents, Ordering::Relaxed)
            + micro_cents
    }

    /// Get current spending for an entity/period
    pub fn get_current(&self, entity_type: &str, entity_id: Uuid, period_type: &str) -> i64 {
        let today = Utc::now().date_naive();
        let period_start = match period_type {
            "daily" => today,
            "monthly" => NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today),
            _ => today,
        };

        let key = CounterKey {
            entity_type: entity_type.to_string(),
            entity_id,
            period_type: period_type.to_string(),
            period_start,
        };

        self.counters
            .get(&key)
            .map(|v| v.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Load counters from DB records (for startup hydration)
    pub fn hydrate(&self, records: &[(String, Uuid, String, NaiveDate, i64)]) {
        for (entity_type, entity_id, period_type, period_start, spent) in records {
            let key = CounterKey {
                entity_type: entity_type.clone(),
                entity_id: *entity_id,
                period_type: period_type.clone(),
                period_start: *period_start,
            };
            self.counters
                .entry(key)
                .or_insert_with(|| AtomicI64::new(0))
                .fetch_add(*spent, Ordering::Relaxed);
        }
    }

    /// Snapshot all counter deltas and reset to 0.
    /// Returns the deltas for flushing to DB.
    pub fn snapshot_and_reset(&self) -> Vec<(CounterKey, i64)> {
        let mut deltas = Vec::new();
        for entry in self.counters.iter() {
            let val = entry.value().swap(0, Ordering::Relaxed);
            if val > 0 {
                deltas.push((entry.key().clone(), val));
            }
        }
        deltas
    }

    /// Re-add deltas back to counters (on flush failure)
    pub fn restore_deltas(&self, deltas: &[(CounterKey, i64)]) {
        for (key, val) in deltas {
            self.counters
                .entry(key.clone())
                .or_insert_with(|| AtomicI64::new(0))
                .fetch_add(*val, Ordering::Relaxed);
        }
    }
}
