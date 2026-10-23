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

use axum::body::{to_bytes, Body};
use axum::http::Request;
use tower::ServiceExt;

/// Loads the dotenv files (the envs of the process always win): `.env` of
/// the repo and `.env.test` of the local stack (the latter is the one of
/// the tests).
///
/// The merge resolves BY LAST WRITE: `.env.test` over `.env` — the bug of
/// the first write (set-if-absent, `.env` first) let the cloud pooler of
/// `.env` win and the locality guard failed: all the integration tests
/// skipped silently (there and in the CI, which does not export
/// `SERVICE_ROLE_KEY` anyway).
fn load_env_files() {
    let mut values = std::collections::HashMap::new();
    for file in [".env", ".env.test"] {
        let Ok(content) = std::fs::read_to_string(file) else {
            continue;
        };
        for line in content.lines() {
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                values.insert(key.to_string(), value.trim().to_string());
            }
        }
    }
    // The process environment keeps the last word over every file.
    for (key, value) in values {
        if std::env::var(&key).is_err() {
            std::env::set_var(key, value);
        }
    }

    // The aliases of the platform: the CI exports the SUPABASE_* names and
    // the config of the app reads SERVICE_ROLE_KEY — the alias makes the
    // exported stack usable without touching the config (the harness is the
    // adapter, R7's one vocabulary).
    if std::env::var("SERVICE_ROLE_KEY").is_err() {
        if let Ok(service) = std::env::var("SUPABASE_SERVICE_ROLE_KEY") {
            std::env::set_var("SERVICE_ROLE_KEY", service);
        }
    }

    // The JWKS URL derives from the base when it does not travel — the
    // ending rule again: the auth-v1-rooted base is the raw gotrue; the
    // platform base gains the gateway prefix.
    if std::env::var("SUPABASE_JWKS_URL").is_err() {
        if let Ok(base) = std::env::var("SUPABASE_URL") {
            let root = base.trim_end_matches('/');
            if root.ends_with("/auth/v1") {
                std::env::set_var("SUPABASE_JWKS_URL", format!("{root}/.well-known/jwks.json"));
            } else {
                std::env::set_var(
                    "SUPABASE_JWKS_URL",
                    format!("{root}/auth/v1/.well-known/jwks.json"),
                );
            }
        }
    }
}

/// The guard of locality: the local DB and the local Supabase; anything
/// else (the real project is in the env!) does not touch the tests.
fn local_env_ok() -> bool {
    let db_ok = std::env::var("DATABASE_URL")
        .map(|url| {
            ["localhost", "127.0.0.1"]
                .iter()
                .any(|host| url.contains(host))
        })
        .unwrap_or(false);
    let auth_ok = std::env::var("SUPABASE_URL")
        .map(|url| {
            ["localhost", "127.0.0.1"]
                .iter()
                .any(|host| url.contains(host))
        })
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
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("complete");
    serde_json::from_slice(&body).expect("the integration answers are always valid JSON")
}

/// The fixture user (via the admin API, R8): the credentials belong to the
/// test that created them — the id of the flow (the mirror, the admin
/// confirmations) and the email/password of the flows of login.
pub struct TestUser {
    /// The mirror and the security-matrix tasks (T013/T022) consume it.
    #[allow(dead_code)]
    pub id: String,
    pub email: String,
    pub password: String,
    #[allow(dead_code)] // the flows assert it when they land
    pub verified: bool,
}

/// Fixture via the admin API (R8): one account verificado/no verificado —
/// returns the whole fixture (the credentials live only in the test).
pub async fn admin_create_user(verified: bool) -> TestUser {
    let base = std::env::var("SUPABASE_URL").expect("the guard checked it");
    let service = std::env::var("SERVICE_ROLE_KEY").expect("the guard checked it");

    // The same ending rule of the app's client — and the prefix flips: the
    // raw gotrue (the base rooted in /auth/v1) serves /admin/users at its
    // root; the standard deployment (the base of the platform) gates the
    // route with /auth/v1.
    let root = base.trim_end_matches('/');
    let root = root.strip_suffix("/auth/v1").unwrap_or(root);
    let url = if base.trim_end_matches('/').ends_with("/auth/v1") {
        format!("{root}/admin/users") // raw gotrue: the route at its root
    } else {
        format!("{root}/auth/v1/admin/users") // the standard gateway
    };

    let email = format!("it-{}@test.local", uuid::Uuid::new_v4());
    let password = format!("ItPass{}a", uuid::Uuid::new_v4().simple());

    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header("apikey", &service)
        .header("authorization", format!("Bearer {service}"))
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
    TestUser {
        id: document["id"]
            .as_str()
            .expect("the admin fixture always carries the id")
            .to_string(),
        email,
        password,
        verified,
    }
}

/// Fixture via the admin API (R8): confirm the email of AN EXISTING user
/// (the UPDATE of `email_confirmed_at` — the trigger of the mirror
/// T003/FR-004 fires on the update, not on the creation) — the second
/// phase the mirror tests drive.
pub async fn admin_confirm_user(user_id: &str) {
    let base = std::env::var("SUPABASE_URL").expect("the guard checked it");
    let service = std::env::var("SERVICE_ROLE_KEY").expect("the guard checked it");
    let root = base.trim_end_matches('/');
    let root = root.strip_suffix("/auth/v1").unwrap_or(root);
    let url = if base.trim_end_matches('/').ends_with("/auth/v1") {
        format!("{root}/admin/users/{user_id}") // raw gotrue
    } else {
        format!("{root}/auth/v1/admin/users/{user_id}") // the gateway
    };

    let client = reqwest::Client::new();
    let response = client
        .put(url)
        .header("apikey", &service)
        .header("authorization", format!("Bearer {service}"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({ "email_confirm": true }))
        .send()
        .await
        .expect("the admin API answers");
    assert!(
        response.status().is_success(),
        "the admin confirm of the fixture succeeds"
    );
}

/// The pool of the LOCAL DB of the tests (the migrations applied with
/// idempotence — the same `Migrator` the app runs): `None` = no local DB
/// (the test skips instead of failing; the CI always has it). The guard of
/// the cloud: NEVER against the project real.
pub async fn db_pool() -> Option<sqlx::PgPool> {
    load_env_files();
    let url = std::env::var("DATABASE_URL").ok()?;
    if !["localhost", "127.0.0.1"]
        .iter()
        .any(|host| url.contains(host))
    {
        tracing::warn!("the tests are skipped: DATABASE_URL is not local");
        return None;
    }
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
        .ok()?;

    let migrations_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
    let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(migrations_dir))
        .await
        .ok()?;
    migrator.run(&pool).await.ok()?;
    Some(pool)
}

/// The login of a fixture email+password (the harness proves flows end to
/// end); it answers the pair or the error code of Supabase. Unconsumed
/// until its flow lands (T011+).
#[expect(dead_code)]
pub async fn login(email: &str, password: &str) -> serde_json::Value {
    let base = std::env::var("SUPABASE_URL").expect("the guard checked it");
    let service = std::env::var("SERVICE_ROLE_KEY").expect("the guard checked it");

    let root = base.trim_end_matches('/');
    let root = root.strip_suffix("/auth/v1").unwrap_or(root);
    let url = if base.trim_end_matches('/').ends_with("/auth/v1") {
        format!("{root}/token?grant_type=password") // raw gotrue at its root
    } else {
        format!("{root}/auth/v1/token?grant_type=password") // the gateway
    };

    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header("apikey", &service)
        .header("authorization", format!("Bearer {service}"))
        .header("content-type", "application/json")
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .await
        .expect("the auth API answers");
    response.json().await.expect("the auth JSON")
}
