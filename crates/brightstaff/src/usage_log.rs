//! Per-response usage log (KAN-551, format v1).
//!
//! One JSON line per metered upstream response is appended to
//! `<FIKAR_USAGE_LOG_DIR>/usage-YYYY-MM-DD.jsonl` (UTC date of `ts`). Lines are handed
//! to a single writer task over a bounded channel, so the request path never touches
//! the disk. Prompt and response text never enter a line.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use opentelemetry::trace::TraceContextExt;
use opentelemetry::KeyValue;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};
use tracing::warn;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::handlers::Attribution;

pub const DIR_ENV: &str = "FIKAR_USAGE_LOG_DIR";
pub const INSTANCE_ENV: &str = "FIKAR_USAGE_INSTANCE";
pub const CALL_ID_ATTR: &str = "fikar.call_id";
const CHANNEL_CAPACITY: usize = 8192;

pub const ENDPOINT_CHAT: &str = "chat";
pub const ENDPOINT_EMBEDDINGS: &str = "embeddings";

/// Token counts of one metered response, as recorded on the counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenCounts {
    pub prompt: Option<u64>,
    pub completion: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
}

impl TokenCounts {
    /// A response with neither prompt nor completion usage is "usage missing": no line.
    pub fn has_usage(&self) -> bool {
        self.prompt.is_some() || self.completion.is_some()
    }
}

#[derive(Serialize)]
struct Line<'a> {
    v: u8,
    ts: String,
    call_id: String,
    instance: &'a str,
    endpoint: &'a str,
    org: &'a str,
    service: &'a str,
    user: &'a str,
    execution: &'a str,
    provider: &'a str,
    model: &'a str,
    prompt: u64,
    completion: u64,
    cache_read: u64,
    cache_write: u64,
}

enum Msg {
    Line { date: String, line: String },
    Flush(oneshot::Sender<()>),
}

/// Cheap-to-clone handle to the writer task.
#[derive(Debug, Clone)]
pub struct UsageLog {
    tx: mpsc::Sender<Msg>,
    instance: Arc<str>,
}

impl UsageLog {
    /// Read `FIKAR_USAGE_LOG_DIR` / `FIKAR_USAGE_INSTANCE`. Unset or empty directory:
    /// `None`, no task, nothing is written. Must be called inside a tokio runtime.
    pub fn from_env() -> Option<Self> {
        let dir = std::env::var(DIR_ENV)
            .ok()
            .filter(|d| !d.trim().is_empty())?;
        let instance = std::env::var(INSTANCE_ENV).unwrap_or_default();
        Some(Self::start(PathBuf::from(dir), &instance))
    }

    /// Spawn the writer task for `dir`. Must be called inside a tokio runtime.
    pub fn start(dir: PathBuf, instance: &str) -> Self {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        tokio::spawn(writer(dir, rx));
        Self {
            tx,
            instance: Arc::from(instance),
        }
    }

    /// Append one line for a metered response and stamp `fikar.call_id` on the current
    /// span. Never blocks and never fails the request.
    pub fn record(
        &self,
        endpoint: &str,
        attribution: &Attribution,
        provider: &str,
        model: &str,
        tokens: &TokenCounts,
    ) {
        self.record_at(Utc::now(), endpoint, attribution, provider, model, tokens);
    }

    fn record_at(
        &self,
        now: DateTime<Utc>,
        endpoint: &str,
        attribution: &Attribution,
        provider: &str,
        model: &str,
        tokens: &TokenCounts,
    ) {
        if !tokens.has_usage() {
            return;
        }
        let call_id = uuid::Uuid::new_v4().to_string();
        let line = Line {
            v: 1,
            ts: now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            call_id: call_id.clone(),
            instance: &self.instance,
            endpoint,
            org: &attribution.org,
            service: &attribution.service,
            user: &attribution.user,
            execution: &attribution.execution,
            provider,
            model,
            prompt: tokens.prompt.unwrap_or(0),
            completion: tokens.completion.unwrap_or(0),
            cache_read: tokens.cache_read.unwrap_or(0),
            cache_write: tokens.cache_write.unwrap_or(0),
        };
        let mut text = match serde_json::to_string(&line) {
            Ok(t) => t,
            Err(err) => {
                warn!(error = %err, "usage log: could not serialize line, dropped");
                return;
            }
        };
        text.push('\n');
        tracing::Span::current()
            .context()
            .span()
            .set_attribute(KeyValue::new(CALL_ID_ATTR, call_id));
        let msg = Msg::Line {
            date: now.format("%Y-%m-%d").to_string(),
            line: text,
        };
        if let Err(err) = self.tx.try_send(msg) {
            warn!(error = %err, "usage log: channel full or closed, line dropped");
        }
    }

    /// Resolves once every line queued before this call has been written.
    pub async fn flush(&self) {
        let (done, wait) = oneshot::channel();
        if self.tx.send(Msg::Flush(done)).await.is_ok() {
            let _ = wait.await;
        }
    }
}

async fn writer(dir: PathBuf, mut rx: mpsc::Receiver<Msg>) {
    let mut open: Option<(String, tokio::fs::File)> = None;
    while let Some(msg) = rx.recv().await {
        match msg {
            Msg::Flush(done) => {
                let _ = done.send(());
            }
            Msg::Line { date, line } => {
                if open.as_ref().map(|(d, _)| d != &date).unwrap_or(true) {
                    match open_file(&dir, &date).await {
                        Ok(f) => open = Some((date, f)),
                        Err(err) => {
                            warn!(error = %err, dir = %dir.display(), "usage log: cannot open file, line dropped");
                            open = None;
                            continue;
                        }
                    }
                }
                if let Some((_, file)) = open.as_mut() {
                    // tokio's File hands the write to a blocking thread; flush waits for
                    // it, so a line is on disk before the next message (or a flush ack).
                    let res = match file.write_all(line.as_bytes()).await {
                        Ok(()) => file.flush().await,
                        Err(e) => Err(e),
                    };
                    if let Err(err) = res {
                        warn!(error = %err, "usage log: write failed, line dropped");
                        open = None;
                    }
                }
            }
        }
    }
}

async fn open_file(dir: &Path, date: &str) -> std::io::Result<tokio::fs::File> {
    tokio::fs::create_dir_all(dir).await?;
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("usage-{date}.jsonl")))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn attr() -> Attribution {
        Attribution {
            org: "3".into(),
            service: "chat".into(),
            user: "42".into(),
            execution: "turn-8c1d2e".into(),
        }
    }

    #[tokio::test]
    async fn line_format_and_utc_date_files() {
        let dir = tempfile::tempdir().unwrap();
        let log = UsageLog::start(dir.path().join("nested"), "pinned");
        let t = Utc.with_ymd_and_hms(2026, 10, 16, 23, 59, 59).unwrap()
            + chrono::Duration::milliseconds(123);
        let tokens = TokenCounts {
            prompt: Some(1200),
            completion: Some(300),
            ..Default::default()
        };
        log.record_at(t, "chat", &attr(), "azure_openai", "gpt-5.6-terra", &tokens);
        log.record_at(
            t + chrono::Duration::seconds(1),
            "embeddings",
            &attr(),
            "azure_openai",
            "m",
            &tokens,
        );
        log.flush().await;
        let a = std::fs::read_to_string(dir.path().join("nested/usage-2026-10-16.jsonl")).unwrap();
        let b = std::fs::read_to_string(dir.path().join("nested/usage-2026-10-17.jsonl")).unwrap();
        assert_eq!(a.lines().count(), 1);
        assert_eq!(b.lines().count(), 1);
        assert!(a.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(a.trim_end()).unwrap();
        let id = v["call_id"].as_str().unwrap();
        assert_eq!(
            a.trim_end().replace(id, "ID"),
            r#"{"v":1,"ts":"2026-10-16T23:59:59.123Z","call_id":"ID","instance":"pinned","endpoint":"chat","org":"3","service":"chat","user":"42","execution":"turn-8c1d2e","provider":"azure_openai","model":"gpt-5.6-terra","prompt":1200,"completion":300,"cache_read":0,"cache_write":0}"#
        );
    }

    #[tokio::test]
    async fn no_usage_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let log = UsageLog::start(dir.path().to_path_buf(), "");
        log.record("chat", &attr(), "p", "m", &TokenCounts::default());
        log.record(
            "chat",
            &attr(),
            "p",
            "m",
            &TokenCounts {
                cache_read: Some(5),
                ..Default::default()
            },
        );
        log.flush().await;
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn dir_unset_or_empty_means_disabled() {
        // from_env is only reached with the variable absent or empty in this test process.
        std::env::remove_var(DIR_ENV);
        assert!(UsageLog::from_env().is_none());
        std::env::set_var(DIR_ENV, "  ");
        assert!(UsageLog::from_env().is_none());
        std::env::remove_var(DIR_ENV);
    }
}
