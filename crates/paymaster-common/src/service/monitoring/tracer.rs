use std::sync::OnceLock;

use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{Protocol, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing::Subscriber;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::service::monitoring::Configuration;

const SERVICE_NAME: &str = "paymaster";

/// Global handle to the tracer provider, held so the process-wide shutdown
/// hook can flush buffered spans before exit.
static PROVIDER: OnceLock<SdkTracerProvider> = OnceLock::new();

pub struct Tracer;

impl Tracer {
    /// Build the `tracing_opentelemetry` subscriber layer and install the
    /// global tracer provider plus a W3C `TraceContext` text-map propagator.
    ///
    /// The propagator is required for inbound `traceparent` headers extracted
    /// by `OtelMakeSpan` to connect to exported spans — without it, spans are
    /// orphan roots in the collector.
    pub fn layer<S>(configuration: &Configuration) -> impl Layer<S>
    where
        S: Subscriber,
        S: for<'span> LookupSpan<'span>,
    {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(format!("{}/v1/traces", configuration.endpoint))
            .with_protocol(Protocol::HttpBinary)
            .with_headers(configuration.headers())
            .build()
            .unwrap();

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(Resource::builder().with_service_name(SERVICE_NAME).build())
            .build();

        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let tracer = provider.tracer(SERVICE_NAME);
        opentelemetry::global::set_tracer_provider(provider.clone());
        let _ = PROVIDER.set(provider);

        tracing_opentelemetry::layer().with_tracer(tracer)
    }
}

/// Flush and shut down the global tracer provider, if one was installed.
///
/// Call this on graceful shutdown so the batch exporter flushes tail spans
/// before the process exits. No-op when `Tracer::layer` was never called.
pub fn shutdown() {
    if let Some(provider) = PROVIDER.get() {
        let _ = provider.shutdown();
    }
}
