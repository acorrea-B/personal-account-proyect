//! Bootstrap of the `api` function (D6 — monolithic function).
//!
//! Startup order: config → pool → tracing → router → server.
//! This file is excluded from the coverage numerator (D17): it is startup glue
//! with no business logic to test in isolation.
//!
//! The extraction into a library (`src/lib.rs`) happens in T010, when the
//! integration harness needs to mount the app without spawning a process.

mod config;
mod router;
mod state;

use std::net::SocketAddr;
use std::process::ExitCode;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::Router;
use tokio::net::TcpListener;
use tower_http::{
    cors::CorsLayer, limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::config::Config;
use crate::router::build_router;
use crate::state::AppState;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(err) = run().await {
        eprintln!("bootstrap error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    init_tracing(&config);

    let state = AppState::build(&config).await?;
    let app = with_layers(build_router(state), &config);

    let listener = TcpListener::bind(config.bind_address()).await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, env = %config.env, "api listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

/// Cross-cutting server layers: tracing, body limit, timeout and CORS.
fn with_layers(app: Router, config: &Config) -> Router {
    app.layer(
        TraceLayer::new_for_http().make_span_with(|req: &axum::http::Request<_>| {
            tracing::info_span!(
                "http",
                method = %req.method(),
                path = %req.uri().path(),
            )
        }),
    )
    .layer(RequestBodyLimitLayer::new(config.max_body_bytes()))
    .layer(TimeoutLayer::with_status_code(
        StatusCode::REQUEST_TIMEOUT,
        config.request_timeout(),
    ))
    .layer(cors_layer(config))
}

/// CORS restricted to the known domains (plan §7, OWASP API8).
///
/// With no domains configured, no cross-origin request is allowed.
fn cors_layer(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> = config
        .allowed_origins()
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();

    if origins.is_empty() {
        return CorsLayer::new()
            .allow_origin(Vec::<HeaderValue>::new())
            .allow_methods(Vec::<axum::http::Method>::new())
            .allow_headers(Vec::<HeaderName>::new());
    }

    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            HeaderName::from_static("authorization"),
            HeaderName::from_static("content-type"),
            HeaderName::from_static("if-unmodified-since"),
            HeaderName::from_static("x-request-id"),
        ])
}

fn init_tracing(config: &Config) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,reqwest=warn"));

    let registry = tracing_subscriber::registry().with(filter);
    if config.env == "development" {
        registry.with(fmt::layer().with_target(true)).init();
    } else {
        registry.with(fmt::layer().json().with_target(true)).init();
    }
}
