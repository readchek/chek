//! Logging, request traces and, when `OTEL_EXPORTER_OTLP_ENDPOINT` is set,
//! export of both to an OpenTelemetry collector such as SigNoz.
//!
//! The exporter follows the standard `OTEL_*` variables and speaks OTLP over
//! plain HTTP, which suits a collector on the same private network.

use std::env;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, SpanExporter};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing::field::Empty;
use tracing::{Instrument, info, info_span, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::api::AppState;
use crate::limit::client_ip;

/// Keeps the exporters alive; [`Telemetry::shutdown`] flushes what's queued.
#[derive(Default)]
pub struct Telemetry {
    tracer: Option<SdkTracerProvider>,
    logger: Option<SdkLoggerProvider>,
}

impl Telemetry {
    pub fn shutdown(self) {
        if let Some(tracer) = self.tracer
            && let Err(e) = tracer.shutdown()
        {
            eprintln!("could not flush traces: {e}");
        }
        if let Some(logger) = self.logger
            && let Err(e) = logger.shutdown()
        {
            eprintln!("could not flush logs: {e}");
        }
    }
}

fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

pub fn init() -> Result<Telemetry, Box<dyn std::error::Error>> {
    let console = tracing_subscriber::fmt::layer().with_filter(filter());
    let exporting = env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok_and(|v| !v.trim().is_empty());
    if !exporting {
        tracing_subscriber::registry().with(console).init();
        return Ok(Telemetry::default());
    }

    // The resource reads OTEL_SERVICE_NAME and OTEL_RESOURCE_ATTRIBUTES;
    // without a name the collector would file everything as unknown_service.
    let mut resource = Resource::builder();
    if env::var_os("OTEL_SERVICE_NAME").is_none() {
        resource = resource.with_service_name("chek");
    }
    let resource = resource.build();
    let tracer = SdkTracerProvider::builder()
        .with_batch_exporter(SpanExporter::builder().with_http().build()?)
        .with_resource(resource.clone())
        .build();
    let logger = SdkLoggerProvider::builder()
        .with_batch_exporter(LogExporter::builder().with_http().build()?)
        .with_resource(resource)
        .build();

    // The exporter's own HTTP client logs too; sending those would feed back
    // into the exporter.
    let exported = || {
        [
            "hyper",
            "reqwest",
            "h2",
            "opentelemetry",
            "opentelemetry_sdk",
            "opentelemetry_otlp",
        ]
        .into_iter()
        .fold(filter(), |f, target| {
            f.add_directive(format!("{target}=off").parse().expect("valid directive"))
        })
    };
    tracing_subscriber::registry()
        .with(console)
        .with(
            tracing_opentelemetry::layer()
                .with_tracer(tracer.tracer("chek"))
                .with_filter(exported()),
        )
        .with(OpenTelemetryTracingBridge::new(&logger).with_filter(exported()))
        .init();
    Ok(Telemetry {
        tracer: Some(tracer),
        logger: Some(logger),
    })
}

/// Logs every request with its route, status and duration, inside a span
/// named after the route so traces group by endpoint rather than by id.
pub async fn trace_request(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let (parts, body) = request.into_parts();
    let method = parts.method.clone();
    let path = parts.uri.path().to_owned();
    let route = parts
        .extensions
        .get::<MatchedPath>()
        .map_or_else(|| "unmatched".to_owned(), |p| p.as_str().to_owned());
    let client = client_ip(&parts, &state.trusted_proxies)
        .map(|ip| ip.to_string())
        .unwrap_or_default();
    let user_agent = parts
        .headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let span = info_span!(
        "request",
        otel.name = %format!("{method} {route}"),
        otel.kind = "server",
        otel.status_code = Empty,
        http.request.method = %method,
        http.route = %route,
        url.path = %path,
        client.address = %client,
        user_agent.original = %user_agent,
        http.response.status_code = Empty,
    );
    let start = Instant::now();
    let response = next
        .run(Request::from_parts(parts, body))
        .instrument(span.clone())
        .await;
    let status = response.status().as_u16();
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    span.record("http.response.status_code", status);
    let _entered = span.enter();
    if status >= 500 {
        span.record("otel.status_code", "ERROR");
        warn!(status, latency_ms, "request failed");
    } else {
        info!(status, latency_ms, "request finished");
    }
    response
}
