//! Shared application state: database pool and Supabase credentials.

use sqlx::{postgres::PgPoolOptions, PgPool};

use crate::config::Config;

/// Dependencies shared by the handlers.
///
/// The Supabase Auth client and the rate-limit store are wired into the state
/// by the tasks that implement them (T007, T008).
#[derive(Clone)]
#[allow(dead_code)]
pub struct AppState {
    pub pool: PgPool,
    pub supabase_url: String,
    pub service_role_key: String,
    pub jwt_secret: String,
    pub jwt_issuer: String,
    pub jwt_audience: String,
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
        })
    }
}
