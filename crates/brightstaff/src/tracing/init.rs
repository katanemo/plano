use std::fmt;
use std::sync::OnceLock;

use opentelemetry::global;
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use time::macros::format_description;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::{format, time::FormatTime, FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use super::ServiceNameOverrideExporter;
use common::configuration::Tracing;

type ReloadHandle = reload::Handle<EnvFilter, tracing_subscriber::Registry>;

static LOG_LEVEL_HANDLE: OnceLock<ReloadHandle> = OnceLock::new();

/// Dynamically change the log level filter at runtime.
///
/// Accepts any valid `RUST_LOG` / `EnvFilter` syntax, e.g. `"debug"`,
/// `"brightstaff=trace,info"`.
pub fn set_log_level(new_filter: &str) -> Result<(), String> {
    let handle = LOG_LEVEL_HANDLE
        .get()
        .ok_or_else(|| "tracer not initialized".to_string())?;
    let filter = EnvFilter::try_new(new_filter)
        .map_err(|e| format!("invalid filter '{new_filter}': {e}"))?;
    handle
        .reload(filter)
        .map_err(|e| format!("failed to reload filter: {e}"))
}

/// Returns the current log level filter string, if the tracer is initialized.
pub fn get_log_level() -> Option<String> {
    let handle = LOG_LEVEL_HANDLE.get()?;
    handle.with_current(|f| f.to_string()).ok()
}

struct BracketedTime;

impl FormatTime for BracketedTime {
    fn format_time(&self, w: &mut format::Writer<'_>) -> fmt::Result {
        let now = time::OffsetDateTime::now_utc();
        write!(
            w,
            "[{}]",
            now.format(&format_description!(
                "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
            ))
            .unwrap()
        )
    }
}

struct BracketedFormatter;

impl<S, N> FormatEvent<S, N> for BracketedFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: format::Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let timer = BracketedTime;
        timer.format_time(&mut writer)?;

        write!(
            writer,
            "[{}]",
            event.metadata().level().to_string().to_lowercase()
        )?;

        // Extract request_id from span context if present
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                let extensions = span.extensions();
                if let Some(fields) = extensions.get::<FormattedFields<N>>() {
                    let fields_str = fields.fields.as_str();
                    // Look for request_id in the formatted fields
                    if let Some(start) = fields_str.find("request_id=") {
                        let rest = &fields_str[start + 11..]; // Skip "request_id="
                        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
                        let rid = &rest[..end];
                        write!(writer, " request_id={}", rid)?;
                        break;
                    }
                }
            }
        }

        write!(writer, " ")?;
        ctx.field_format().format_fields(writer.by_ref(), event)?;

        writeln!(writer)
    }
}

use tracing_subscriber::fmt::FormattedFields;

static INIT_LOGGER: OnceLock<SdkTracerProvider> = OnceLock::new();

pub fn init_tracer(tracing_config: Option<&Tracing>) -> &'static SdkTracerProvider {
    INIT_LOGGER.get_or_init(|| {
        global::set_text_map_propagator(TraceContextPropagator::new());

        // Get OTEL endpoint and sampling from config.yaml tracing section
        let otel_endpoint = tracing_config.and_then(|t| t.opentracing_grpc_endpoint.clone());

        let random_sampling = tracing_config.and_then(|t| t.random_sampling).unwrap_or(0);

        let tracing_enabled = random_sampling > 0 && otel_endpoint.is_some();
        eprintln!(
            "initializing tracing: tracing_enabled={}, otel_endpoint={:?}, random_sampling={}",
            tracing_enabled, otel_endpoint, random_sampling
        );

        // Create OTLP exporter to send spans to collector.
        // Use `if let` to destructure the endpoint, avoiding an unwrap.
        if let Some(endpoint) = otel_endpoint.as_deref().filter(|_| tracing_enabled) {
            if std::env::var("OTEL_SERVICE_NAME").is_err() {
                std::env::set_var("OTEL_SERVICE_NAME", "plano");
            }
            // Create ServiceNameOverrideExporter to support per-span service names
            // This allows spans to have different service names (e.g., plano(orchestrator),
            // plano(filter), plano(llm)) by setting the "service.name.override" attribute
            let exporter = ServiceNameOverrideExporter::new(endpoint);

            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .build();

            global::set_tracer_provider(provider.clone());

            // Create OpenTelemetry tracing layer using TracerProvider trait
            use opentelemetry::trace::TracerProvider as _;
            let telemetry_layer =
                tracing_opentelemetry::layer().with_tracer(provider.tracer("brightstaff"));

            let env_filter =
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
            let (filter_layer, reload_handle) = reload::Layer::new(env_filter);
            LOG_LEVEL_HANDLE.set(reload_handle).ok();

            // Create fmt layer with span field formatting enabled (no ANSI to keep fields parseable)
            let fmt_layer = tracing_subscriber::fmt::layer()
                .event_format(BracketedFormatter)
                .fmt_fields(format::DefaultFields::new())
                .with_ansi(false);

            let subscriber = tracing_subscriber::registry()
                .with(filter_layer)
                .with(telemetry_layer)
                .with(fmt_layer);

            tracing::subscriber::set_global_default(subscriber)
                .expect("Failed to set tracing subscriber");

            provider
        } else {
            // Tracing disabled - use no-op provider
            let provider = SdkTracerProvider::builder().build();
            global::set_tracer_provider(provider.clone());

            let env_filter =
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
            let (filter_layer, reload_handle) = reload::Layer::new(env_filter);
            LOG_LEVEL_HANDLE.set(reload_handle).ok();

            // Create fmt layer with span field formatting enabled (no ANSI to keep fields parseable)
            let fmt_layer = tracing_subscriber::fmt::layer()
                .event_format(BracketedFormatter)
                .fmt_fields(format::DefaultFields::new())
                .with_ansi(false);

            tracing_subscriber::registry()
                .with(filter_layer)
                .with(fmt_layer)
                .init();

            provider
        }
    })
}
