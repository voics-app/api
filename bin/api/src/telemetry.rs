//! Structured JSON logs to stdout, optional OpenTelemetry trace export over
//! OTLP/HTTP, and the Prometheus metrics recorder (Global Constraints).

use std::error::Error;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::TracerProvider;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};
use url::Url;

/// Keeps the tracer provider alive so buffered spans are flushed on shutdown.
pub struct Telemetry {
    provider: Option<TracerProvider>,
}

impl Telemetry {
    pub fn install(level: &str, otlp_endpoint: Option<&Url>) -> Result<Self, Box<dyn Error>> {
        let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
        let db = DbLatency.with_filter(filter_fn(|meta| meta.target() == "sqlx::query"));
        let logs = tracing_subscriber::fmt::layer()
            .json()
            .with_target(false)
            .with_filter(filter);

        match otlp_endpoint {
            Some(endpoint) => {
                let exporter = SpanExporter::builder()
                    .with_http()
                    .with_endpoint(endpoint.join("v1/traces")?.to_string())
                    .build()?;
                let provider = TracerProvider::builder()
                    .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
                    .build();
                let tracer = provider.tracer("api");
                tracing_subscriber::registry()
                    .with(logs)
                    .with(db)
                    .with(tracing_opentelemetry::layer().with_tracer(tracer))
                    .init();
                Ok(Self {
                    provider: Some(provider),
                })
            }
            None => {
                tracing_subscriber::registry().with(logs).with(db).init();
                Ok(Self { provider: None })
            }
        }
    }

    pub fn shutdown(self) {
        if let Some(provider) = self.provider {
            let _ = provider.shutdown();
        }
    }
}

/// Histogram buckets (seconds) shared by request and DB query latency.
const LATENCY_BUCKETS: [f64; 12] = [
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 10.0,
];

/// Installs the global Prometheus recorder; the handle renders `/metrics`.
pub fn prometheus() -> Result<PrometheusHandle, Box<dyn Error>> {
    Ok(PrometheusBuilder::new()
        .set_buckets(&LATENCY_BUCKETS)?
        .install_recorder()?)
}

/// Records sqlx's per-statement `elapsed_secs` events (target `sqlx::query`) as the
/// `db_query_duration_seconds` histogram, labelled by SQL verb. No call-site changes needed.
pub struct DbLatency;

impl<S: tracing::Subscriber> Layer<S> for DbLatency {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = QueryVisitor::default();
        event.record(&mut visitor);
        if let Some(seconds) = visitor.elapsed_secs {
            metrics::histogram!(
                "db_query_duration_seconds",
                "db" => "postgres",
                "operation" => sql_operation(visitor.summary.as_deref().unwrap_or(""))
            )
            .record(seconds);
        }
    }
}

#[derive(Default)]
struct QueryVisitor {
    elapsed_secs: Option<f64>,
    summary: Option<String>,
}

impl tracing::field::Visit for QueryVisitor {
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        if field.name() == "elapsed_secs" {
            self.elapsed_secs = Some(value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "summary" {
            self.summary = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "summary" {
            self.summary = Some(format!("{value:?}"));
        }
    }
}

/// Low-cardinality label: the leading SQL verb.
fn sql_operation(summary: &str) -> &'static str {
    let verb = summary
        .trim_start_matches(|c: char| !c.is_ascii_alphabetic())
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match verb.as_str() {
        "select" | "with" => "select",
        "insert" => "insert",
        "update" => "update",
        "delete" => "delete",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_is_the_sql_verb() {
        assert_eq!(sql_operation("SELECT id FROM users"), "select");
        assert_eq!(sql_operation("\"insert into x\""), "insert");
        assert_eq!(sql_operation("UPDATE t SET a = 1"), "update");
        assert_eq!(sql_operation("DELETE FROM t"), "delete");
        assert_eq!(sql_operation("BEGIN"), "other");
        assert_eq!(sql_operation(""), "other");
    }

    #[test]
    fn layer_records_elapsed_into_histogram() {
        let recorder = PrometheusBuilder::new()
            .set_buckets(&LATENCY_BUCKETS)
            .unwrap()
            .build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let subscriber = tracing_subscriber::registry().with(DbLatency);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "sqlx::query", summary = "SELECT 1", elapsed_secs = 0.004_f64);
        });
        let text = handle.render();
        assert!(
            text.contains("db_query_duration_seconds_bucket{db=\"postgres\",operation=\"select\""),
            "{text}"
        );
        assert!(
            text.contains(
                "db_query_duration_seconds_count{db=\"postgres\",operation=\"select\"} 1"
            ),
            "{text}"
        );
    }
}
