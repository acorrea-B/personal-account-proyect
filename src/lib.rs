//! Library entry point and handler of the `api` function (D6).
//!
//! The Rust runtime of Scaleway invokes one exported async function per
//! request (`pub async fn handle(req: Request<Body>) -> Response<Body>`), so
//! the axum router remains the internal core (plan §13.2): the handler hands
//! the request over and returns the response unchanged.
//!
//! The configuration and the connection pool are built once per cold start
//! and reused by the invocations of that instance.

pub mod config;
pub mod errors;
pub mod events;
pub mod router;
pub mod state;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tokio::sync::OnceCell;
use tower::ServiceExt;
use tower_http::{
    cors::CorsLayer, limit::RequestBodyLimitLayer, timeout::TimeoutLayer, trace::TraceLayer,
};

use crate::config::Config;
use crate::router::build_router;
use crate::state::AppState;

/// Failure during the startup of the function: the configuration is invalid or
/// the database is unreachable.
///
/// Distinct from the API errors (`src/errors.rs`), which are HTTP responses.
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("invalid configuration: {0}")]
    Config(#[from] config::ConfigError),
    #[error("database unavailable: {0}")]
    Database(#[from] sqlx::Error),
}

/// The application of an instance: configuration and router.
#[derive(Clone)]
pub struct Application {
    pub config: Config,
    pub router: Router,
}

/// The application of this instance, built once per cold start.
static APPLICATION: OnceCell<Application> = OnceCell::const_new();

/// Builds the application: configuration → pool → tracing → router.
pub async fn application() -> Result<Application, StartupError> {
    APPLICATION
        .get_or_try_init(|| async {
            let config = Config::from_env()?;
            init_tracing(&config);
            let state = AppState::build(&config).await?;
            let router = with_layers(build_router(state), &config);
            Ok(Application { config, router })
        })
        .await
        .cloned()
}

/// The handler of the function: one invocation per request.
pub async fn handle(req: Request<Body>) -> Response<Body> {
    let application = match application().await {
        Ok(application) => application,
        Err(err) => {
            tracing::error!(error = %err, "the function could not start");
            return internal_error();
        }
    };

    match application.router.clone().oneshot(req).await {
        Ok(response) => response,
        Err(err) => {
            tracing::error!(error = %err, "the request could not be served");
            internal_error()
        }
    }
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

/// Initializes the tracing subscriber: JSON in the deployed environments,
/// human readable in development.
///
/// The subscriber is global and can only be installed once, so a second call
/// (another instance of the process, a test) is ignored instead of panicking.
fn init_tracing(config: &Config) {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,reqwest=warn"));

    let registry = tracing_subscriber::registry().with(filter);
    if config.env == "development" {
        let _ = registry.with(fmt::layer().with_target(true)).try_init();
    } else {
        let _ = registry
            .with(fmt::layer().json().with_target(true))
            .try_init();
    }
}

/// RFC 7807 answer for the failures that happen outside the request
/// lifecycle (the function could not start): the canonical catalogue of
/// `src/errors.rs` builds the body.
fn internal_error() -> Response<Body> {
    errors::Problem::new(errors::ApiError::Internal).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use sqlx::postgres::PgPoolOptions;

    /// A state with a lazy pool: the test never queries the database.
    fn test_state() -> AppState {
        AppState {
            pool: PgPoolOptions::new()
                .connect_lazy("postgres://postgres:postgres@127.0.0.1:5432/postgres")
                .expect("the connection string is valid"),
            supabase_url: "http://127.0.0.1:54321".to_string(),
            service_role_key: "test-service-role-key".to_string(),
            jwt_secret: "test-jwt-secret".to_string(),
            jwt_issuer: "supabase".to_string(),
            jwt_audience: "authenticated".to_string(),
        }
    }

    #[tokio::test]
    async fn the_health_route_answers_ok() {
        let router = build_router(test_state());

        let response = router
            .oneshot(
                Request::get("/health")
                    .body(Body::empty())
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");

        assert_eq!(response.status(), StatusCode::OK);
    }
}
