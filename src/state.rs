//! Shared application state: database pool and Supabase credentials.

use std::sync::Arc;

use sqlx::{postgres::PgPoolOptions, PgPool};

use crate::clients::supabase_auth::SupabaseAuthClient;
use crate::config::Config;
use crate::middleware::auth::SignatureVerifier;
use crate::middleware::hs256::Hs256Verifier;

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
}

impl AppState {
    /// Builds the state: connection pool + Supabase credentials.
    ///
    /// Fails fast when the database is unreachable (Principle III).
    pub async fn build(config: &Config) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(config.db_max_connections)
            .acquire_timeout(config.db_acquire_timeout)
            .connect(&config.database_url)
            .await?;

        Ok(Self {
            pool,
            supabase_url: config.supabase_url.clone(),
            service_role_key: config.service_role_key.clone(),
            jwt_secret: config.supabase_jwt_secret.clone(),
            jwt_issuer: config.supabase_jwt_issuer.clone(),
            jwt_audience: config.supabase_jwt_audience.clone(),
            supabase_auth: SupabaseAuthClient::new(
                &config.supabase_url,
                &config.supabase_publishable_key,
                &config.service_role_key,
                config.supabase_timeout,
            ),
            signature_verifier: Arc::new(Hs256Verifier::new(
                &config.supabase_jwt_secret,
                &config.supabase_jwt_issuer,
                &config.supabase_jwt_audience,
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
                "supabase",
                "authenticated",
            )),
        }
    }
}
