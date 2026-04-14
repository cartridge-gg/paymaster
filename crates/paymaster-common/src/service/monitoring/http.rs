//! HTTP middleware for inbound distributed tracing.
//!
//! Extracts the W3C trace context from incoming HTTP headers and makes it
//! the parent of the request's root span, so spans emitted by `#[instrument]`
//! RPC methods chain to the caller's trace in the collector.

use opentelemetry_http::HeaderExtractor;
use tower_http::classify::{ServerErrorsAsFailures, SharedClassifier};
use tower_http::trace::{MakeSpan, TraceLayer};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// `tower-http` `MakeSpan` that extracts the W3C trace context from inbound
/// HTTP headers and sets it as the parent of the new request span.
///
/// If no propagator is globally installed or the headers contain no
/// `traceparent`, the extracted context is empty and the span starts as
/// a fresh root — no panic, no-op.
#[derive(Debug, Clone, Default)]
pub struct OtelMakeSpan;

impl<B> MakeSpan<B> for OtelMakeSpan {
    fn make_span(&mut self, request: &http::Request<B>) -> Span {
        let cx = opentelemetry::global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(request.headers())));
        let span = tracing::info_span!(
            "http_request",
            method = %request.method(),
            uri = %request.uri(),
        );
        span.set_parent(cx);
        span
    }
}

/// Convenience constructor for a `tower-http` `TraceLayer` pre-wired with
/// [`OtelMakeSpan`]. Drop this into a `ServiceBuilder` before the rest of
/// the middleware stack so every inbound request gets a root span with the
/// caller's remote parent context.
pub fn trace_layer() -> TraceLayer<SharedClassifier<ServerErrorsAsFailures>, OtelMakeSpan> {
    TraceLayer::new_for_http().make_span_with(OtelMakeSpan)
}

#[cfg(test)]
mod tests {
    use opentelemetry::trace::TraceContextExt;
    use opentelemetry::Context;
    use opentelemetry_sdk::propagation::TraceContextPropagator;

    use super::*;

    fn ensure_propagator() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        });
    }

    fn extract_parent<B>(req: &http::Request<B>) -> Context {
        opentelemetry::global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(req.headers())))
    }

    #[test]
    fn make_span_without_traceparent_is_root() {
        ensure_propagator();
        let req = http::Request::builder().uri("/health").body(()).unwrap();

        let _ = OtelMakeSpan.make_span(&req);

        let cx = extract_parent(&req);
        let otel_span = cx.span();
        assert!(!otel_span.span_context().is_valid());
    }

    #[test]
    fn make_span_with_valid_traceparent_is_child() {
        ensure_propagator();
        let trace_id_hex = "0af7651916cd43dd8448eb211c80319c";
        let req = http::Request::builder()
            .uri("/paymaster_buildTransaction")
            .header("traceparent", format!("00-{trace_id_hex}-b7ad6b7169203331-01"))
            .body(())
            .unwrap();

        let _ = OtelMakeSpan.make_span(&req);

        let cx = extract_parent(&req);
        let otel_span = cx.span();
        let sc = otel_span.span_context();
        assert!(sc.is_valid(), "remote span context should be valid");
        assert_eq!(sc.trace_id().to_string(), trace_id_hex);
    }

    #[test]
    fn make_span_with_malformed_traceparent_does_not_panic() {
        ensure_propagator();
        let req = http::Request::builder()
            .uri("/health")
            .header("traceparent", "garbage-not-a-valid-header")
            .body(())
            .unwrap();

        let _ = OtelMakeSpan.make_span(&req);

        let cx = extract_parent(&req);
        let otel_span = cx.span();
        assert!(!otel_span.span_context().is_valid());
    }
}
