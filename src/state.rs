//! Shared application state: database pool and Supabase credentials.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sqlx::{postgres::PgPoolOptions, PgPool};

use crate::clients::supabase_auth::SupabaseAuthClient;
use crate::config::Config;
use crate::middleware::auth::SignatureVerifier;
#[cfg(test)]
use crate::middleware::hs256::Hs256Verifier;
use crate::middleware::jwks::JwksVerifier;

/// The short cache of the session state per credential (the option b of
/// the plan §16.6 — the operator's ruling: the rotation is mandatory): the
/// alive-answer of goTruth within the TTL avoids the round trip of every
/// protected request; the window of the revocation visibility = the TTL,
/// and the `invalidate` of the revoke closes it in the very request.
#[derive(Clone, Default)]
pub struct SessionCheck {
    alive: Arc<Mutex<HashMap<String, std::time::Instant>>>,
    /// The HS256 test path: the credentials it signs carry no Supabase
    /// session behind — the network check would be pure noise.
    #[cfg(test)]
    trusted: bool,
}

#[cfg(test)]
impl SessionCheck {
    /// The always-alive of the test path.
    pub fn trusted() -> Self {
        Self {
            alive: Arc::new(Mutex::new(HashMap::new())),
            trusted: true,
        }
    }
}

impl SessionCheck {
    /// The TTL of the cached-alive answer: revoked/deleted-user becomes
    /// visible in at most this — and the revoke's own `invalidate`
    /// unconditionally.
    const TTL: std::time::Duration = std::time::Duration::from_secs(60);

    /// The credential cached-alive within the TTL? The trusted states skip
    /// the question (the HS256 test path — no session behind).
    pub(crate) fn alive_within_ttl(&self, token: &str) -> bool {
        #[cfg(test)]
        if self.trusted {
            return true;
        }
        let alive = self.alive.lock().expect("the session cache never poisons");
        alive
            .get(token)
            .is_some_and(|seen| seen.elapsed() <= Self::TTL)
    }

    /// Records an alive answer (after the goTruth confirmed the session).
    pub(crate) fn record(&self, token: &str) {
        let mut alive = self.alive.lock().expect("the session cache never poisons");
        // The sweep of the stale: the entries older than the TTL leave
        // (the map cannot grow unbounded — the scale of the credentials).
        alive.retain(|_, seen| seen.elapsed() <= Self::TTL);
        alive.insert(token.to_string(), std::time::Instant::now());
    }

    /// The revocation closes the window at once (the revoke handler's
    /// cleanup).
    pub(crate) fn invalidate(&self, token: &str) {
        self.alive
            .lock()
            .expect("the session cache never poisons")
            .remove(token);
    }
}

/// Dependencies shared by the handlers.
///
/// The Supabase Auth client and the rate-limit store are wired into the state
/// by the tasks that implement them (T007, T008).
#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub pool: PgPool,
    /// The REST client of Supabase Auth (T008).
    pub supabase_auth: SupabaseAuthClient,
    pub supabase_url: String,
    pub service_role_key: String,
    pub jwt_secret: String,
    pub jwt_issuer: String,
    pub jwt_audience: String,
    /// Verification of the credentials (R1): HS256 behind the trait, the
    /// JWKS alternative plugs in the same point (auth.rs).
    pub signature_verifier: Arc<dyn SignatureVerifier>,
    /// The session-state cache of the credentials (plan §16.6, option b —
    /// the operator's ruling: the rotation is mandatory): the extractor
    /// checks it after the signature.
    pub session_check: SessionCheck,
}

impl AppState {
    /// Builds the state: connection pool + Supabase credentials.
    ///
    /// Fails fast when the database is unreachable (Principle III) — the
    /// eager `connect` IS the startup probe: the first physical connection
    /// opens here or the cold start fails (`StartupError::Database`).
    pub async fn build(config: &Config) -> Result<Self, sqlx::Error> {
        let started = std::time::Instant::now();
        let pool = PgPoolOptions::new()
            .max_connections(config.db_max_connections)
            .acquire_timeout(config.db_acquire_timeout)
            .connect(&config.database_url)
            .await?;

        // The startup log of the connection (operator's request, plan §16.2):
        // a function instance opens and closes its connections — this line
        // proves the pooler answered BEFORE the first real request; the
        // failure of this path travels one step up as `StartupError` and
        // `lib.rs` names it in the ERROR log of the cold start.
        tracing::info!(
            latency_ms = started.elapsed().as_millis() as u64,
            db_max_connections = config.db_max_connections,
            "the database pool responds — the startup connection succeeded"
        );

        Ok(Self {
            pool,
            session_check: SessionCheck::default(),
            supabase_url: config.supabase_url.clone(),
            service_role_key: config.service_role_key.clone(),
            // Only the test path signs tokens with it (test_token): the
            // verification runs against the JWKS of the platform since the
            // asymmetric signing-key rollout of the platform.
            jwt_secret: String::new(),
            jwt_issuer: config.supabase_jwt_issuer.clone(),
            jwt_audience: config.supabase_jwt_audience.clone(),
            supabase_auth: SupabaseAuthClient::new(
                &config.supabase_url,
                &config.supabase_publishable_key,
                &config.service_role_key,
                config.supabase_timeout,
            ),
            signature_verifier: Arc::new(JwksVerifier::new(
                &config.supabase_jwks_url,
                config.issuer_to_verify(),
                &config.supabase_jwt_audience,
                config.supabase_timeout,
            )),
        })
    }
}

#[cfg(test)]
impl AppState {
    /// A state for the tests of the router and the middlewares: lazy pool
    /// (no DB is ever touched in these tests) and the HS256 verifier of the
    /// test secrets.
    pub fn for_tests() -> Self {
        use sqlx::postgres::PgPoolOptions;

        Self {
            pool: PgPoolOptions::new()
                .connect_lazy("postgres://postgres:postgres@127.0.0.1:5432/postgres")
                .expect("the connection string is valid"),
            // The test path: trusted (the HS256 credentials carry no
            // Supabase session behind them).
            session_check: SessionCheck::trusted(),
            supabase_url: "http://127.0.0.1:54321".to_string(),
            service_role_key: "test-service-role-key".to_string(),
            jwt_secret: "test-jwt-secret".to_string(),
            jwt_issuer: "supabase".to_string(),
            jwt_audience: "authenticated".to_string(),
            supabase_auth: SupabaseAuthClient::new(
                "http://127.0.0.1:54321",
                "test-publishable",
                "test-service-role-key",
                std::time::Duration::from_secs(3),
            ),
            signature_verifier: std::sync::Arc::new(Hs256Verifier::new(
                "test-jwt-secret",
                Some("supabase"),
                "authenticated",
            )),
        }
    }
}
