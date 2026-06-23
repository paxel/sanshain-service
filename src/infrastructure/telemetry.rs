use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use opentelemetry_semantic_conventions as semconv;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

/// Initializes the OpenTelemetry tracer and returns a tracing layer and the provider.
///
/// This function sets up the OTLP exporter via gRPC and configures
/// the global tracer provider and W3C propagator.
pub fn init_tracer<S>() -> (
    OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>,
    SdkTracerProvider,
)
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
{
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new(semconv::resource::SERVICE_NAME, env!("CARGO_PKG_NAME")),
            KeyValue::new(
                semconv::resource::SERVICE_VERSION,
                env!("CARGO_PKG_VERSION"),
            ),
        ])
        .build();

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .expect("Failed to create span exporter");

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .with_sampler(Sampler::AlwaysOn)
        .build();

    opentelemetry::global::set_tracer_provider(provider.clone());

    // Set global propagator for W3C context propagation
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    let layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("sanshain_service"));

    (layer, provider)
}

/// Shuts down the telemetry system and flushes remaining spans.
pub fn shutdown_telemetry(provider: SdkTracerProvider) {
    let _ = provider.shutdown();
}
