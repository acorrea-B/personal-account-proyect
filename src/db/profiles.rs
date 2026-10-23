//! Repository of the identity mirror (D3/FR-019): the row is created by the
//! trigger of the email confirmation (T003, FR-004) — the API never writes
//! the email of the mirror, only the profile fields (PUT /v1/profile).
//!
//! Every query is filtered by `user_id`: the isolation lives in Rust and
//! the RLS deny-all (D3) is the safety net.

use chrono::Timelike;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::RepoError;

/// The one row per account (1:1 with `auth.users`, trigger of T003).
#[derive(Debug, sqlx::FromRow)]
pub struct ProfileRow {
    pub id: Uuid,
    pub email: String,
    pub full_name: String,
    pub headline: Option<String>,
    pub bio: Option<String>,
    pub location: Option<String>,
    /// jsonb of the profile; validated structurally in Rust (R3).
    pub links: Option<serde_json::Value>,
    pub skills: Option<serde_json::Value>,
    pub processing_consent: Option<bool>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// The mirror of the user (or `None` while the account is unverified).
pub async fn get(pool: &PgPool, user_id: Uuid) -> Result<Option<ProfileRow>, RepoError> {
    let row = sqlx::query_as::<_, ProfileRow>(
        "select id, email, full_name, headline, bio, location, links, skills, \
         processing_consent, created_at, updated_at \
         from profiles where id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(RepoError::Sql)?;

    Ok(row)
}

/// The optimistic update (Q2): when the client declares a version
/// (`If-Unmodified-Since`), the comparison happens against the live row
/// locked in the transaction.
pub struct ProfileUpdate {
    pub full_name: String,
    pub headline: Option<String>,
    pub bio: Option<String>,
    pub location: Option<String>,
    pub links: Option<serde_json::Value>,
    pub skills: Option<serde_json::Value>,
    pub processing_consent: Option<bool>,
}

/// Upserts the fields of the profile of the user and returns the row with
/// its new version.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &PgPool,
    user_id: Uuid,
    expected_version: Option<chrono::DateTime<chrono::Utc>>,
    patch: ProfileUpdate,
) -> Result<ProfileRow, RepoError> {
    // The transaction arbitrates the optimistic conflict (Q2): the lock of
    // the row + the comparison of the version inside it.
    let mut transaction = pool.begin().await.map_err(RepoError::Sql)?;

    let current: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("select created_at, updated_at from profiles where id = $1 for update")
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(RepoError::Sql)?;

    let (_, live_updated_at) = current.ok_or(RepoError::Missing)?;

    if let Some(expected) = expected_version {
        // The contract's instant travels truncated to seconds (the
        // response's format): the DB's precision (micros) never denies an
        // otherwise equal second — both sides truncate to the contract's
        // precision for the comparison.
        let live = live_updated_at
            .with_nanosecond(0)
            .unwrap_or(live_updated_at);
        let expected = expected.with_nanosecond(0).unwrap_or(expected);
        if live != expected {
            // Nothing written: the transaction rolls back on return.
            return Err(RepoError::Stale(live));
        }
    }

    let row = sqlx::query_as::<_, ProfileRow>(
        "update profiles set \
           full_name = $2, headline = $3, bio = $4, location = $5, \
           links = $6, skills = $7, processing_consent = $8, updated_at = now() \
         where id = $1 \
         returning id, email, full_name, headline, bio, location, links, skills, \
           processing_consent, created_at, updated_at",
    )
    .bind(user_id)
    .bind(&patch.full_name)
    .bind(&patch.headline)
    .bind(&patch.bio)
    .bind(&patch.location)
    .bind(&patch.links)
    .bind(&patch.skills)
    .bind(patch.processing_consent)
    .fetch_one(&mut *transaction)
    .await
    .map_err(RepoError::Sql)?;

    transaction.commit().await.map_err(RepoError::Sql)?;
    Ok(row)
}
#[cfg(test)]
mod tests {
    use super::*;

    /// The mirror without the account verified: `get` answers `None` and
    /// the update rejects with `Missing` (the handler answers not-found).
    #[tokio::test]
    async fn the_mirror_without_verification_does_not_exist() {
        let Some(pool) = crate::db::test_support::pool().await else {
            eprintln!("skipped: no local DATABASE_URL (supabase start)");
            return;
        };
        let user = crate::db::test_support::create_user(&pool).await;
        assert!(get(&pool, user).await.unwrap().is_none());

        let patch = ProfileUpdate {
            full_name: "Alguien".to_string(),
            headline: None,
            bio: None,
            location: None,
            links: None,
            skills: None,
            processing_consent: None,
        };
        let update = update(&pool, user, None, patch).await;
        assert!(matches!(update, Err(RepoError::Missing)));
    }

    /// The upsert of the fields does not touch the mirror (email intact,
    /// created_at intact) and rounds correctly the typed jsonbs.
    #[tokio::test]
    async fn the_upsert_rounds_the_fields_without_touching_the_mirror() {
        let Some(pool) = crate::db::test_support::pool().await else {
            eprintln!("skipped: no local DATABASE_URL (supabase start)");
            return;
        };
        let user = crate::db::test_support::create_user(&pool).await;
        let email = format!("mirror-{user}@test.local");

        // The trigger of T003 created it: the test replicates its effect.
        sqlx::query("insert into profiles (id, email, full_name) values ($1, $2, 'User')")
            .bind(user)
            .bind(&email)
            .execute(&pool)
            .await
            .unwrap();

        let patch = ProfileUpdate {
            full_name: "Alguien Importante".to_string(),
            headline: Some("Backend".to_string()),
            bio: None,
            location: None,
            links: Some(serde_json::json!([
                {"type": "linkedin", "url": "https://www.linkedin.com/in/alguien"}
            ])),
            skills: Some(serde_json::json!(["rust", "aws"])),
            processing_consent: None,
        };
        let row = update(&pool, user, None, patch).await.unwrap();

        assert_eq!(row.email, email, "the mirror keeps the email");
        assert_eq!(row.full_name, "Alguien Importante");
        assert_eq!(row.headline.as_deref(), Some("Backend"));
        assert_eq!(
            row.links,
            Some(
                serde_json::json!([{"type": "linkedin", "url": "https://www.linkedin.com/in/alguien"}])
            )
        );
        assert_eq!(row.skills, Some(serde_json::json!(["rust", "aws"])));

        // The update is idempotent: the mirror keeps being 1:1.
        let count: i64 = sqlx::query_scalar("select count(*) from profiles where id = $1")
            .bind(user)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    /// Q2: the profile update is also optimistic — a version older answers
    /// `stale-version` and touches nothing.
    #[tokio::test]
    async fn the_profile_is_also_optimistic() {
        let Some(pool) = crate::db::test_support::pool().await else {
            eprintln!("skipped: no local DATABASE_URL (supabase start)");
            return;
        };
        let user = crate::db::test_support::create_user(&pool).await;
        let email = format!("optimista-{user}@test.local");

        sqlx::query("insert into profiles (id, email, full_name) values ($1, $2, 'User')")
            .bind(user)
            .bind(&email)
            .execute(&pool)
            .await
            .unwrap();

        let patch = ProfileUpdate {
            full_name: "Primero".to_string(),
            headline: None,
            bio: None,
            location: None,
            links: None,
            skills: None,
            processing_consent: None,
        };
        let first = update(&pool, user, None, patch).await.unwrap();
        let version = first.updated_at;

        // The contract's precision = seconds: the clock leaves the second
        // of the first write, so the next write REALLY rotates the
        // version (the truncation is faithful to the response's format).
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        let patch = ProfileUpdate {
            full_name: "Con la version correcta".to_string(),
            headline: None,
            bio: None,
            location: None,
            links: None,
            skills: None,
            processing_consent: None,
        };
        let updated = update(&pool, user, Some(version), patch).await.unwrap();
        assert_eq!(updated.full_name, "Con la version correcta");

        // The version older conflicts without writing.
        let patch = ProfileUpdate {
            full_name: "Con la version vieja".to_string(),
            headline: None,
            bio: None,
            location: None,
            links: None,
            skills: None,
            processing_consent: None,
        };
        let conflict = update(&pool, user, Some(version), patch).await;
        assert!(matches!(conflict, Err(RepoError::Stale(_))));

        let untouched = get(&pool, user).await.unwrap().unwrap();
        assert_eq!(untouched.full_name, "Con la version correcta");
    }

    /// FR-019: the query filtered by user_id — the profile of another user
    /// never travels.
    #[tokio::test]
    async fn the_queries_never_cross_users() {
        let Some(pool) = crate::db::test_support::pool().await else {
            eprintln!("skipped: no local DATABASE_URL (supabase start)");
            return;
        };
        let alice = crate::db::test_support::create_user(&pool).await;
        let bob = crate::db::test_support::create_user(&pool).await;

        sqlx::query("insert into profiles (id, email, full_name) values ($1, $2, 'Alice')")
            .bind(alice)
            .bind(format!("alice-{alice}@test.local"))
            .execute(&pool)
            .await
            .unwrap();

        assert!(get(&pool, bob).await.unwrap().is_none());
        assert!(get(&pool, alice).await.unwrap().is_some());
    }
}
