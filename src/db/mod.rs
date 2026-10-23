//! Repositories of the persistence layer (D3): every query is ALWAYS
//! filtered by `user_id` (FR-019 anti-BOLA) — the RLS deny-all is the
//! security net and the filter by caller is the only authorized path.

pub mod cv_texts;
pub mod profiles;

/// The errors of the repositories: the SQL of the server (`Sql`) and the
/// collision of the optimistic write (Q2 — the transaction is the final
/// referee and returns the current version).
#[derive(Debug)]
pub enum RepoError {
    Sql(sqlx::Error),
    /// The mirror row does not exist (unverified account): the handler
    /// answers not-found / the profile does not exist yet.
    Missing,
    /// `stale-version`: the resource changed under the feet of the client;
    /// the current version travels to name it in the document.
    Stale(chrono::DateTime<chrono::Utc>),
}
#[cfg(test)]
pub(crate) mod test_support {
    use sqlx::PgPool;
    use uuid::Uuid;

    /// The pool of the local DB of tests (Supabase local — §12.3): the
    /// migrations of T003 applied with idempotence. `None`: no local DB —
    /// the test is skipped instead of failing (the CI always has it).
    ///
    /// Guard against the cloud: the tests NEVER run against the project
    /// real (the database of tests is only local).
    pub async fn pool() -> Option<PgPool> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let local = ["localhost", "127.0.0.1"]
            .iter()
            .any(|host| url.contains(host));
        if !local {
            tracing::warn!("the tests are skipped: DATABASE_URL is not local");
            return None;
        }

        // The pool of the test: TINY on purpose — every #[tokio::test]
        // opens its OWN pool and the suites run in parallel: the default
        // 10 connections per pool starves the shared local postgres (the
        // goTruth of the same machine exhausts first — the CI's flakes of
        // the T021 branch traced back to this). Two connections solve the
        // whole test's work.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(&url)
            .await
            .ok()?;

        // The migrations of T003 resolved at RUNTIME (the macro of embed
        // does not accept routes of this crate here): the same `Migrator`
        // of sqlx that the CI applies.
        let migrations_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(migrations_dir))
            .await
            .inspect_err(|error| tracing::warn!(%error, "the migrations could not be read"))
            .ok()?;
        migrator
            .run(&pool)
            .await
            .inspect_err(|error| tracing::warn!(%error, "the migrations of tests failed"))
            .ok()?;

        Some(pool)
    }

    /// One test user in auth.users (the FKs of the schema point there):
    /// unique email per user of test. For cleanup: the rows of the user
    /// cascade with their deletion (D12).
    pub async fn create_user(pool: &PgPool) -> Uuid {
        let id = uuid::Uuid::new_v4();
        let email = format!("test-{id}@test.local");

        sqlx::query(
            "insert into auth.users (id, aud, role, email, encrypted_password, \
             email_confirmed_at, raw_app_meta_data, raw_user_meta_data) \
             values ($1, 'authenticated', 'authenticated', $2, '', now(), '{}', '{}')",
        )
        .bind(id)
        .bind(&email)
        .execute(pool)
        .await
        .expect("the test user is created");

        id
    }
}
