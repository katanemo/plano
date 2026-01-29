use std::fmt;
use std::sync::OnceLock;

use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use time::macros::format_description;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::{format, time::FormatTime, FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::EnvFilter;

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
    S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
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
            "[{}] ",
            event.metadata().level().to_string().to_lowercase()
        )?;

        ctx.field_format().format_fields(writer.by_ref(), event)?;

        writeln!(writer)
    }
}

static INIT_LOGGER: OnceLock<SdkTracerProvider> = OnceLock::new();

pub fn init_tracer() -> &'static SdkTracerProvider {
    INIT_LOGGER.get_or_init(|| {
        global::set_text_map_propagator(TraceContextPropagator::new());

        // Get OTEL collector URL from environment
        let otel_endpoint = std::env::var("OTEL_COLLECTOR_URL")
            .unwrap_or_else(|_| "http://localhost:4317".to_string());

        let tracing_enabled = std::env::var("OTEL_TRACING_ENABLED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(false);

        // Create OTLP exporter to send spans to collector
        if tracing_enabled {
            // Set service name via environment if not already set
            if std::env::var("OTEL_SERVICE_NAME").is_err() {
                std::env::set_var("OTEL_SERVICE_NAME", "brightstaff");
            }

            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(&otel_endpoint)
                .build()
                .expect("Failed to create OTLP span exporter");

            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .build();

            global::set_tracer_provider(provider.clone());

            // Create OpenTelemetry tracing layer using TracerProvider trait
            use opentelemetry::trace::TracerProvider as _;
            let telemetry_layer =
                tracing_opentelemetry::layer().with_tracer(provider.tracer("brightstaff"));

            // Combine the OpenTelemetry layer with fmt layer using the registry
            let subscriber = tracing_subscriber::registry()
                .with(telemetry_layer)
                .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
                .with(tracing_subscriber::fmt::layer().event_format(BracketedFormatter));

            tracing::subscriber::set_global_default(subscriber)
                .expect("Failed to set tracing subscriber");

            provider
        } else {
            // Tracing disabled - use no-op provider
            let provider = SdkTracerProvider::builder().build();
            global::set_tracer_provider(provider.clone());

            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
                )
                .event_format(BracketedFormatter)
                .init();

            provider
        }
    })
}
