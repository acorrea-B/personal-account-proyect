//! Route mapping and the explicit list of public routes (FR-011, deny-by-default).
//!
//! The public/protected split is explicit: public routes are mounted on their
//! own router; protected ones hang from the router with the JWT gate applied.
//! Any route missing from `PUBLIC_ROUTES` requires a valid credential.

use axum::{routing::get, Router};

use crate::state::AppState;

/// Public API routes (FR-011). Anything not listed here requires a credential.
// Consumed by the JWT gate in T006.
#[allow(dead_code)]
pub const PUBLIC_ROUTES: &[&str] = &[
    "POST /v1/auth/register",
    "POST /v1/auth/resend-verification",
    "POST /v1/auth/login",
    "POST /v1/auth/refresh",
];

/// Health probe for the post-deploy smoke test (plan §12.2).
async fn health() -> &'static str {
    "ok"
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .with_state(state)
}
