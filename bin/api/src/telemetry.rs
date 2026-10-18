//! Structured JSON logs to stdout, optional OpenTelemetry trace export over
//! OTLP/HTTP, and the Prometheus metrics recorder (Global Constraints).

use std::error::Error;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::TracerProvider;
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
                    .with(tracing_opentelemetry::layer().with_tracer(tracer))
                    .init();
                Ok(Self {
                    provider: Some(provider),
                })
            }
            None => {
                tracing_subscriber::registry().with(logs).init();
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

/// Installs the global Prometheus recorder; the handle renders `/metrics`.
pub fn prometheus() -> Result<PrometheusHandle, Box<dyn Error>> {
    Ok(PrometheusBuilder::new().install_recorder()?)
}
