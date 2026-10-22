//! Harness of the integration tests (T010): the full application over the
//! LOCAL Supabase of R8 (§12.3), fixtures of verified/unverified users via
//! the admin API, and the request helper with a request id.
//!
//! The tests skip silently when the local Supabase is not up (`supabase
//! start`): they never run against the real project (guard).
//!
//! Environment: the process env wins; below, the repo `.env` and the
//! `.env.test` (the values of the local stack, gitignored — they are set
//! before the first `app()` of the test binary).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::Request;
use tower::ServiceExt;

/// Loads the dotenv files (the envs of the process always win): `.env` of
/// the repo first, `.env.test` of the local stack after (the latter is the
/// one of the tests).
fn load_env_files() {
    // Later files win over the earlier ones (the test stack has priority).
    for file in [".env", ".env.test"] {
        let Ok(content) = std::fs::read_to_string(file) else { continue };
        for line in content.lines() {
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                if std::env::var(key).is_err() {
                    std::env::set_var(key, value.trim());
                }
            }
        }
    }
}

/// The guard of locality: the local DB and the local Supabase; anything
/// else (the real project is in the env!) does not touch the tests.
fn local_env_ok() -> bool {
    let db_ok = std::env::var("DATABASE_URL")
        .map(|url| ["localhost", "127.0.0.1"].iter().any(|host| url.contains(host)))
        .unwrap_or(false);
    let auth_ok = std::env::var("SUPABASE_URL")
        .map(|url| ["localhost", "127.0.0.1"].iter().any(|host| url.contains(host)))
        .unwrap_or(false);
    let credentials_ok = std::env::var("SERVICE_ROLE_KEY")
        .map(|key| !key.trim().is_empty())
        .unwrap_or(false);
    db_ok && auth_ok && credentials_ok
}

/// The application of the test process: built once (the static of
/// `lib.APPLICATION`) with the env of the harness already loaded.
pub async fn app() -> Option<perfil_api::Application> {
    load_env_files();
    if !local_env_ok() {
        eprintln!("skipped: no local Supabase (supabase start) or no credentials");
        return None;
    }
    perfil_api::application().await.ok()
}

/// The counter of the request id per call (visible in the logs).
pub fn new_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// One request via the router: method + path (+ JSON body optative) and the
/// id of correlation always placed (`x-request-id`, D18 rule 3).
pub async fn call(
    application: &perfil_api::Application,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(path);
    builder = builder.header("x-request-id", new_request_id());
    let request = match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .expect("the request is valid"),
        None => builder.body(Body::empty()).expect("the request is valid"),
    };

    application
        .router
        .clone()
        .oneshot(request)
        .await
        .expect("the router never fails")
}

/// The body of the answer as JSON (the problem documents carry one).
pub async fn json_of(response: axum::response::Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.expect("complete");
    serde_json::from_slice(&body).expect("the integration answers are always valid JSON")
}

/// Fixture via the admin API (R8): one account verificado/no verificado —
/// returns the id of the user.
pub async fn admin_create_user(verified: bool) -> String {
    let base = std::env::var("SUPABASE_URL").expect("the guard checked it");
    let service = std::env::var("SERVICE_ROLE_KEY").expect("the guard checked it");

    // Direct gotrue (the local stack, without a gateway): the admin routes
    // go without the `/auth/v1` prefix; with a gateway, with it.
    let prefix = if base.contains("127.0.0.1") || base.contains("localhost") {
        ""
    } else {
        "/auth/v1"
    };

    let email = format!("it-{}@test.local", uuid::Uuid::new_v4());
    let password = format!("ItPass{}a", uuid::Uuid::new_v4().simple());

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}{prefix}/admin/users"))
        .header("apikey", &service)
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "email": email,
            "password": password,
            "email_confirm": verified,
        }))
        .send()
        .await
        .expect("the admin API answers");

    let document: serde_json::Value = response.json().await.expect("the admin JSON");
    document["id"]
        .as_str()
        .expect("the admin fixture always carries the id")
        .to_string()
}

/// The login of a fixture email+password (the harness proves flows end to
/// end); it answers the pair or the error code of Supabase.
pub async fn login(email: &str, password: &str) -> serde_json::Value {
    let base = std::env::var("SUPABASE_URL").expect("the guard checked it");
    let service = std::env::var("SERVICE_ROLE_KEY").expect("the guard checked it");

    let prefix = if base.contains("127.0.0.1") || base.contains("localhost") {
        ""
    } else {
        "/auth/v1"
    };

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}{prefix}/token?grant_type=password"))
        .header("apikey", &service)
        .header("content-type", "application/json")
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await
        .expect("the auth API answers");
    response.json().await.expect("the auth JSON")
}

