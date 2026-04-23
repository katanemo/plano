//! Prometheus metrics for brightstaff.
//!
//! Installs the `metrics` global recorder backed by
//! `metrics-exporter-prometheus` and exposes a `/metrics` HTTP endpoint on a
//! dedicated admin port (default `0.0.0.0:9092`, overridable via
//! `METRICS_BIND_ADDRESS`).
//!
//! Emitted metric families (see `describe_all` for full list):
//! - HTTP RED: `brightstaff_http_requests_total`,
//!   `brightstaff_http_request_duration_seconds`,
//!   `brightstaff_http_in_flight_requests`.
//! - LLM upstream: `brightstaff_llm_upstream_requests_total`,
//!   `brightstaff_llm_upstream_duration_seconds`,
//!   `brightstaff_llm_time_to_first_token_seconds`,
//!   `brightstaff_llm_tokens_total`,
//!   `brightstaff_llm_tokens_usage_missing_total`.
//! - Routing: `brightstaff_router_decisions_total`,
//!   `brightstaff_router_decision_duration_seconds`,
//!   `brightstaff_routing_service_requests_total`,
//!   `brightstaff_session_cache_events_total`.
//! - Process: via `metrics-process`.
//! - Build: `brightstaff_build_info`.

use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use tracing::{info, warn};

pub mod labels;

/// Guard flag so tests don't re-install the global recorder.
static INIT: OnceLock<()> = OnceLock::new();

const DEFAULT_METRICS_BIND: &str = "0.0.0.0:9092";

/// HTTP request duration buckets (seconds). Capped at 60s.
const HTTP_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

/// LLM upstream / TTFT buckets (seconds). Capped at 120s because provider
/// completions routinely run that long.
const LLM_BUCKETS: &[f64] = &[0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0];

/// Router decision buckets (seconds). The orchestrator call itself is usually
/// sub-second but bucketed generously in case of upstream slowness.
const ROUTER_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

/// Install the global recorder and spawn the `/metrics` HTTP listener.
///
/// Safe to call more than once; subsequent calls are no-ops so tests that
/// construct their own recorder still work.
pub fn init() {
    if INIT.get().is_some() {
        return;
    }

    let bind: SocketAddr = std::env::var("METRICS_BIND_ADDRESS")
        .unwrap_or_else(|_| DEFAULT_METRICS_BIND.to_string())
        .parse()
        .unwrap_or_else(|err| {
            warn!(error = %err, default = DEFAULT_METRICS_BIND, "invalid METRICS_BIND_ADDRESS, falling back to default");
            DEFAULT_METRICS_BIND.parse().expect("default bind parses")
        });

    let builder = PrometheusBuilder::new()
        .with_http_listener(bind)
        .set_buckets_for_metric(
            Matcher::Full("brightstaff_http_request_duration_seconds".to_string()),
            HTTP_BUCKETS,
        )
        .and_then(|b| {
            b.set_buckets_for_metric(Matcher::Prefix("brightstaff_llm_".to_string()), LLM_BUCKETS)
        })
        .and_then(|b| {
            b.set_buckets_for_metric(
                Matcher::Full("brightstaff_router_decision_duration_seconds".to_string()),
                ROUTER_BUCKETS,
            )
        });

    let builder = match builder {
        Ok(b) => b,
        Err(err) => {
            warn!(error = %err, "failed to configure metrics buckets, using defaults");
            PrometheusBuilder::new().with_http_listener(bind)
        }
    };

    if let Err(err) = builder.install() {
        warn!(error = %err, "failed to install Prometheus recorder; metrics disabled");
        return;
    }

    let _ = INIT.set(());

    describe_all();
    emit_build_info();

    // Register process-level collector (RSS, CPU, FDs).
    let collector = metrics_process::Collector::default();
    collector.describe();
    // Prime once at startup; subsequent scrapes refresh via the exporter's
    // per-scrape render, so we additionally refresh on a short interval to
    // keep gauges moving between scrapes without requiring client pull.
    collector.collect();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            collector.collect();
        }
    });

    info!(address = %bind, "metrics listener started");
}

fn describe_all() {
    describe_counter!(
        "brightstaff_http_requests_total",
        "Total HTTP requests served by brightstaff, by handler and status class."
    );
    describe_histogram!(
        "brightstaff_http_request_duration_seconds",
        "Wall-clock duration of HTTP requests served by brightstaff, by handler."
    );
    describe_gauge!(
        "brightstaff_http_in_flight_requests",
        "Number of HTTP requests currently being served by brightstaff, by handler."
    );

    describe_counter!(
        "brightstaff_llm_upstream_requests_total",
        "LLM upstream request outcomes, by provider, model, status class and error class."
    );
    describe_histogram!(
        "brightstaff_llm_upstream_duration_seconds",
        "Wall-clock duration of LLM upstream calls (stream close for streaming), by provider and model."
    );
    describe_histogram!(
        "brightstaff_llm_time_to_first_token_seconds",
        "Time from request start to first streamed byte, by provider and model (streaming only)."
    );
    describe_counter!(
        "brightstaff_llm_tokens_total",
        "Tokens reported in the provider `usage` field, by provider, model and kind (prompt/completion)."
    );
    describe_counter!(
        "brightstaff_llm_tokens_usage_missing_total",
        "LLM responses that completed without a usable `usage` block (so token counts are unknown)."
    );

    describe_counter!(
        "brightstaff_router_decisions_total",
        "Routing decisions made by the orchestrator, by route, selected model, and whether a fallback was used."
    );
    describe_histogram!(
        "brightstaff_router_decision_duration_seconds",
        "Time spent in the orchestrator deciding a route, by route."
    );
    describe_counter!(
        "brightstaff_routing_service_requests_total",
        "Outcomes of /routing/* decision requests: decision_served, no_candidates, policy_error."
    );
    describe_counter!(
        "brightstaff_session_cache_events_total",
        "Session affinity cache lookups and stores, by outcome."
    );

    describe_gauge!(
        "brightstaff_build_info",
        "Build metadata. Always 1; labels carry version and git SHA."
    );
}

fn emit_build_info() {
    let version = env!("CARGO_PKG_VERSION");
    let git_sha = option_env!("GIT_SHA").unwrap_or("unknown");
    gauge!(
        "brightstaff_build_info",
        "version" => version.to_string(),
        "git_sha" => git_sha.to_string(),
    )
    .set(1.0);
}

/// Split a provider-qualified model id like `"openai/gpt-4o"` into
/// `(provider, model)`. Returns `("unknown", raw)` when there is no `/`.
pub fn split_provider_model(full: &str) -> (&str, &str) {
    match full.split_once('/') {
        Some((p, m)) => (p, m),
        None => ("unknown", full),
    }
}

/// Bucket an HTTP status code into `"2xx"` / `"4xx"` / `"5xx"` / `"1xx"` / `"3xx"`.
pub fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

// ---------------------------------------------------------------------------
// HTTP RED helpers
// ---------------------------------------------------------------------------

/// RAII guard that increments the in-flight gauge on construction and
/// decrements on drop. Pair with [`HttpTimer`] in the `route()` wrapper so the
/// gauge drops even on error paths.
pub struct InFlightGuard {
    handler: &'static str,
}

impl InFlightGuard {
    pub fn new(handler: &'static str) -> Self {
        gauge!(
            "brightstaff_http_in_flight_requests",
            "handler" => handler,
        )
        .increment(1.0);
        Self { handler }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        gauge!(
            "brightstaff_http_in_flight_requests",
            "handler" => self.handler,
        )
        .decrement(1.0);
    }
}

/// Record the HTTP request counter + duration histogram.
pub fn record_http(handler: &'static str, method: &'static str, status: u16, started: Instant) {
    let class = status_class(status);
    counter!(
        "brightstaff_http_requests_total",
        "handler" => handler,
        "method" => method,
        "status_class" => class,
    )
    .increment(1);
    histogram!(
        "brightstaff_http_request_duration_seconds",
        "handler" => handler,
    )
    .record(started.elapsed().as_secs_f64());
}

// ---------------------------------------------------------------------------
// LLM upstream helpers
// ---------------------------------------------------------------------------

/// Classify an outcome of an LLM upstream call for the `error_class` label.
pub fn llm_error_class_from_reqwest(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "timeout"
    } else if err.is_connect() {
        "connect"
    } else if err.is_decode() {
        "parse"
    } else {
        "other"
    }
}

/// Record the outcome of an LLM upstream call. `status` is the HTTP status
/// the upstream returned (0 if the call never produced one, e.g. send failure).
/// `error_class` is `"none"` on success, or a discriminated error label.
pub fn record_llm_upstream(
    provider: &str,
    model: &str,
    status: u16,
    error_class: &str,
    duration: Duration,
) {
    let class = if status == 0 {
        "error"
    } else {
        status_class(status)
    };
    counter!(
        "brightstaff_llm_upstream_requests_total",
        "provider" => provider.to_string(),
        "model" => model.to_string(),
        "status_class" => class,
        "error_class" => error_class.to_string(),
    )
    .increment(1);
    histogram!(
        "brightstaff_llm_upstream_duration_seconds",
        "provider" => provider.to_string(),
        "model" => model.to_string(),
    )
    .record(duration.as_secs_f64());
}

pub fn record_llm_ttft(provider: &str, model: &str, ttft: Duration) {
    histogram!(
        "brightstaff_llm_time_to_first_token_seconds",
        "provider" => provider.to_string(),
        "model" => model.to_string(),
    )
    .record(ttft.as_secs_f64());
}

pub fn record_llm_tokens(provider: &str, model: &str, kind: &'static str, count: u64) {
    counter!(
        "brightstaff_llm_tokens_total",
        "provider" => provider.to_string(),
        "model" => model.to_string(),
        "kind" => kind,
    )
    .increment(count);
}

pub fn record_llm_tokens_usage_missing(provider: &str, model: &str) {
    counter!(
        "brightstaff_llm_tokens_usage_missing_total",
        "provider" => provider.to_string(),
        "model" => model.to_string(),
    )
    .increment(1);
}

// ---------------------------------------------------------------------------
// Router helpers
// ---------------------------------------------------------------------------

pub fn record_router_decision(
    route: &'static str,
    selected_model: &str,
    fallback: bool,
    duration: Duration,
) {
    counter!(
        "brightstaff_router_decisions_total",
        "route" => route,
        "selected_model" => selected_model.to_string(),
        "fallback" => if fallback { "true" } else { "false" },
    )
    .increment(1);
    histogram!(
        "brightstaff_router_decision_duration_seconds",
        "route" => route,
    )
    .record(duration.as_secs_f64());
}

pub fn record_routing_service_outcome(outcome: &'static str) {
    counter!(
        "brightstaff_routing_service_requests_total",
        "outcome" => outcome,
    )
    .increment(1);
}

pub fn record_session_cache_event(outcome: &'static str) {
    counter!(
        "brightstaff_session_cache_events_total",
        "outcome" => outcome,
    )
    .increment(1);
}
