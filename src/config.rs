//! Configuration read from the environment (secrets only via env — plan §12.2).
//!
//! Deliberate centralization: a single struct holding every operational knob.

use std::time::Duration;

use thiserror::Error;

/// Startup error: the configuration is invalid or a required variable is
/// missing. Distinct from the API errors (`errors.rs`), which are HTTP
/// responses.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("missing required environment variable: {0}")]
    MissingEnv(String),
    #[error("invalid value for environment variable: {0}")]
    InvalidValue(String),
}

#[derive(Clone, Debug)]
// The rate-limit knobs (R2) and timeouts are consumed by the middleware and
// the clients added in later tasks; they are centralized here from the scaffold.
#[allow(dead_code)]
pub struct Config {
    pub env: String,
    pub port: u16,
    pub host: String,
    pub database_url: String,
    pub supabase_url: String,
    pub supabase_jwks_url: String,
    pub service_role_key: String,
    /// Public key of the new Supabase system (sb_publishable_...) for the
    /// public flows of Auth (the legacy 'anon' occupies the same slot).
    pub supabase_publishable_key: String,
    pub supabase_jwt_issuer: String,
    pub supabase_jwt_audience: String,
    pub max_body_bytes: usize,
    pub request_timeout: Duration,
    pub db_max_connections: u32,
    pub db_acquire_timeout: Duration,
    pub query_timeout: Duration,
    pub supabase_timeout: Duration,
    pub allowed_origins: Vec<String>,
    pub rate_limit_login_per_min: u32,
    pub rate_limit_resend_per_10min: u32,
    pub rate_limit_cv_write_per_min: u32,
    pub rate_limit_request_per_min: u32,
    pub cv_max_bytes: usize,
    pub cv_versions_retention: i64,
}

impl Config {
    /// Reads the configuration from the environment. Fails fast when anything
    /// required is missing.
    pub fn from_env() -> Result<Self, ConfigError> {
        let env = var("ENV").unwrap_or_else(|| "development".to_string());
        let port = parse_or("PORT", 8080u16)?;
        let host = var("HOST").unwrap_or_else(|| "0.0.0.0".to_string());

        let database_url = required("DATABASE_URL")?;
        let supabase_url = required("SUPABASE_URL")?;
        let supabase_jwks_url = required("SUPABASE_JWKS_URL")?;
        let service_role_key = required("SERVICE_ROLE_KEY")?;
        let supabase_publishable_key = required("SUPABASE_PUBLISHABLE_KEY")?;

        Ok(Self {
            env,
            port,
            host,
            database_url,
            supabase_url: supabase_url.trim_end_matches('/').to_string(),
            supabase_jwks_url,
            service_role_key,
            supabase_publishable_key,
            supabase_jwt_issuer: var("SUPABASE_JWT_ISSUER")
                .unwrap_or_else(|| "supabase".to_string()),
            supabase_jwt_audience: var("SUPABASE_JWT_AUDIENCE")
                .unwrap_or_else(|| "authenticated".to_string()),
            max_body_bytes: parse_or("MAX_BODY_BYTES", 65_536usize)?,
            request_timeout: Duration::from_secs(parse_or("REQUEST_TIMEOUT_SECS", 10u64)?),
            db_max_connections: parse_or("DB_MAX_CONNECTIONS", 5u32)?,
            db_acquire_timeout: Duration::from_secs(parse_or("DB_ACQUIRE_TIMEOUT_SECS", 5u64)?),
            query_timeout: Duration::from_secs(parse_or("QUERY_TIMEOUT_SECS", 5u64)?),
            supabase_timeout: Duration::from_secs(parse_or("SUPABASE_TIMEOUT_SECS", 3u64)?),
            allowed_origins: var("ALLOWED_ORIGINS")
                .map(|raw| {
                    raw.split(',')
                        .map(|o| o.trim().to_string())
                        .filter(|o| !o.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            rate_limit_login_per_min: parse_or("RATE_LIMIT_LOGIN_PER_MIN", 5u32)?,
            rate_limit_resend_per_10min: parse_or("RATE_LIMIT_RESEND_PER_10MIN", 3u32)?,
            rate_limit_cv_write_per_min: parse_or("RATE_LIMIT_CV_WRITE_PER_MIN", 10u32)?,
            rate_limit_request_per_min: parse_or("RATE_LIMIT_REQUEST_PER_MIN", 120u32)?,
            cv_max_bytes: parse_or("CV_MAX_BYTES", 51_200usize)?,
            cv_versions_retention: parse_or("CV_VERSIONS_RETENTION", 10i64)?,
        })
    }

    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }

    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }

    /// The issuer to enforce on the credentials: `None` when the SUPABASE
    /// does not emit it (legacy gotrue; the verification skips it).
    pub fn issuer_to_verify(&self) -> Option<&str> {
        let issuer = self.supabase_jwt_issuer.trim();
        if issuer.is_empty() {
            None
        } else {
            Some(issuer)
        }
    }
}

fn var(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn required(key: &str) -> Result<String, ConfigError> {
    var(key).ok_or_else(|| {
        tracing::error!(key, "missing required environment variable");
        ConfigError::MissingEnv(key.to_string())
    })
}

fn parse_or<T>(key: &str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
{
    match var(key) {
        None => Ok(default),
        Some(raw) => raw.parse::<T>().map_err(|_| {
            tracing::error!(key, "invalid value for environment variable");
            ConfigError::InvalidValue(key.to_string())
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_required_variable_is_a_config_error() {
        // Without touching the environment: `from_env` must fail on
        // `DATABASE_URL`.
        let err = Config::from_env().unwrap_err();
        assert!(matches!(err, ConfigError::MissingEnv(_)));
    }

    #[test]
    fn secrets_are_never_printed_in_the_error() {
        let err = ConfigError::MissingEnv("SERVICE_ROLE_KEY".to_string());
        assert_eq!(
            err.to_string(),
            "missing required environment variable: SERVICE_ROLE_KEY"
        );
    }
}
