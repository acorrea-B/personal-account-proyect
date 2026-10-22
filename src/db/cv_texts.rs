//! Repository of the CV (live version + history, ADR 0013): the save is
//! the sole multi-table point of write — the everything-or-nothing (FR-016/
//! 017, SC-005): archive → upsert → purge ≤10, one transaction.
//!
//! The optimistic arbitration (Q2) happens INSIDE the transaction (`FOR
//! UPDATE` on the live row): in disagreement nothing is archived nor
//! overwritten — `stale-version` with the current version.

use sqlx::PgPool;
use uuid::Uuid;

use crate::db::RepoError;
use crate::events::{Event, Outcome, Taxonomy};

/// The live version of the CV: 1 row per user (unique, T003).
#[derive(Debug, sqlx::FromRow)]
pub struct CvTextRow {
    pub user_id: Uuid,
    pub content: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// The result of a save: the accounting of the purge (the emission of
/// `cv.versions_purged` only when versions fell — one event per
/// significant outcome, D18).
#[derive(Debug)]
pub struct SaveOutcome {
    pub purged_count: u64,
}

/// The live version of the user (or `None` without a saved CV yet).
///
/// The filter by user_id is the isolated access path (FR-019, D3).
pub async fn get(pool: &PgPool, user_id: Uuid) -> Result<Option<CvTextRow>, RepoError> {
    let row = sqlx::query_as::<_, CvTextRow>(
        "select user_id, content, updated_at from cv_texts where user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(RepoError::Sql)?;

    Ok(row)
}

/// The atomic save (Q2/FR-016/017): the tx arbitrates the version; it
/// archives the anterior content (if there was one), upserts the live one
/// and purges the surplus of the retention window; only when versions fell
/// does it emit `cv.versions_purged` with `purged_count` (D18, emitter: the
/// repository in the tx).
pub async fn save(
    pool: &PgPool,
    user_id: Uuid,
    content: &str,
    request_id: &str,
    expected_version: Option<chrono::DateTime<chrono::Utc>>,
    retention: i64,
) -> Result<SaveOutcome, RepoError> {
    let mut transaction = pool.begin().await.map_err(RepoError::Sql)?;

    // The final referee (Q2): the live row locked before any write.
    let live: Option<CvTextRow> = sqlx::query_as::<_, CvTextRow>(
        "select user_id, content, updated_at from cv_texts where user_id = $1 for update",
    )
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(RepoError::Sql)?;

    if let Some(expected) = expected_version {
        if live
            .as_ref()
            .is_some_and(|live| live.updated_at != expected)
        {
            // Nothing was written: the tx rolls back on the return.
            return Err(RepoError::Stale(live.expect("locked above").updated_at));
        }
    }

    if let Some(live) = &live {
        // The archive of the anterior content (FR-016), before the upsert,
        // inside the same tx: everything-or-nothing.
        sqlx::query("insert into cv_versions (user_id, content) values ($1, $2)")
            .bind(user_id)
            .bind(&live.content)
            .execute(&mut *transaction)
            .await
            .map_err(RepoError::Sql)?;
    }

    sqlx::query(
        "insert into cv_texts (user_id, content) values ($1, $2) \
         on conflict (user_id) do update set content = $2, updated_at = now()",
    )
    .bind(user_id)
    .bind(content)
    .execute(&mut *transaction)
    .await
    .map_err(RepoError::Sql)?;

    // The retention (FR-017/SC-005): keep the last N (archived_at desc) and
    // purge the surplus in the same tx.
    let purged: u64 = sqlx::query_scalar::<_, Uuid>(
        "with doomed as ( \
           select id from cv_versions where user_id = $1 \
           order by archived_at desc, id desc offset $2 \
         ) \
         delete from cv_versions where id in (select id from doomed) \
         returning id",
    )
    .bind(user_id)
    .bind(retention)
    .fetch_all(&mut *transaction)
    .await
    .map_err(RepoError::Sql)?
    .len() as u64;

    transaction.commit().await.map_err(RepoError::Sql)?;

    if purged > 0 {
        let mut meta = Event::no_meta();
        meta.insert("purged_count".to_string(), serde_json::Value::from(purged));
        meta.insert("kept".to_string(), serde_json::Value::from(retention));

        Event::new(
            Taxonomy::CvVersionsPurged,
            Outcome::Accepted,
            request_id,
            Some(user_id.to_string()),
            None,
            meta,
        )
        .emit();
    }

    Ok(SaveOutcome {
        purged_count: purged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests of the repositories run against one ephemeral DB per test
    /// (SQLx) with the schema of T003 applied: `supabase start` locally or
    /// the service of the CI (§12.3).
    #[tokio::test]
    async fn the_first_save_upserts_without_archive() {
        let Some(pool) = crate::db::test_support::pool().await else {
            eprintln!("skipped: no local DATABASE_URL (supabase start)");
            return;
        };
        let user = crate::db::test_support::create_user(&pool).await;
        let outcome = save(&pool, user, "primer CV", "req-1", None, 10)
            .await
            .expect("the first save succeeds");
        assert_eq!(outcome.purged_count, 0);

        assert!(get(&pool, user).await.unwrap().is_some());

        // A second save archives the first content.
        save(&pool, user, "segundo CV", "req-1", None, 10)
            .await
            .expect("the second save succeeds");

        let live = get(&pool, user).await.unwrap().expect("the live version");
        assert_eq!(live.content, "segundo CV");

        let archived: i64 =
            sqlx::query_scalar("select count(*) from cv_versions where user_id = $1")
                .bind(user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(archived, 1);
    }

    /// SC-005 / FR-017: after 16 replacements, exactly 10 versions live in
    /// the history and the rest were purged in the same tx.
    #[tokio::test]
    async fn the_retention_keeps_ten_versions() {
        let Some(pool) = crate::db::test_support::pool().await else {
            eprintln!("skipped: no local DATABASE_URL (supabase start)");
            return;
        };
        let user = crate::db::test_support::create_user(&pool).await;

        for round in 0..17 {
            let content = format!("CV #{round}");
            save(&pool, user, &content, "req-1", None, 10)
                .await
                .expect("the save succeeds");
        }

        let live = get(&pool, user).await.unwrap().expect("the live version");
        assert_eq!(live.content, "CV #16");

        let archived: i64 =
            sqlx::query_scalar("select count(*) from cv_versions where user_id = $1")
                .bind(user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(archived, 10, "the retention keeps exactly 10");
    }

    /// Q2: the optimistic conflict answers `stale-version` WITHOUT writing
    /// anything (nothing archived, nothing overwritten).
    #[tokio::test]
    async fn the_stale_version_blocks_and_writes_nothing() {
        let Some(pool) = crate::db::test_support::pool().await else {
            eprintln!("skipped: no local DATABASE_URL (supabase start)");
            return;
        };
        let user = crate::db::test_support::create_user(&pool).await;

        save(&pool, user, "CV A", "req-2", None, 10).await.unwrap();
        let live = get(&pool, user).await.unwrap().unwrap();
        let live_version = live.updated_at;

        // The client declares an older version: conflict.
        let old = live_version - chrono::Duration::milliseconds(1);
        let conflict = save(&pool, user, "CV B", "req-2", Some(old), 10).await;
        assert!(matches!(conflict, Err(RepoError::Stale(_))));

        // Nothing travelled: the content remains and without archive.
        let after = get(&pool, user).await.unwrap().unwrap();
        assert_eq!(after.content, "CV A");
        let archived: i64 =
            sqlx::query_scalar("select count(*) from cv_versions where user_id = $1")
                .bind(user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(archived, 0);
    }

    /// FR-019: the query never crosses users — the CV of another user does
    /// not travel by any filter.
    #[tokio::test]
    async fn the_queries_never_cross_users() {
        let Some(pool) = crate::db::test_support::pool().await else {
            eprintln!("skipped: no local DATABASE_URL (supabase start)");
            return;
        };
        let alice = crate::db::test_support::create_user(&pool).await;
        let bob = crate::db::test_support::create_user(&pool).await;

        save(&pool, alice, "CV de alice", "req-3", None, 10)
            .await
            .unwrap();

        assert!(get(&pool, bob).await.unwrap().is_none());
    }
}
