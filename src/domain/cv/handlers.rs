//! The handlers of the CV (T021, US4): the save (the fail fast in the
//! contract's order — UTF-8 → bytes — the optimistic precondition Q2
//! INSIDE the repository's transaction by way of the repo), the live read
//! and the history of the versions. The D3 underneath: the user_id ONLY
//! from the credential — every query filters it (FR-019, anti-BOLA).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;

use crate::db::cv_texts::{self, VersionMetaRow, VersionRow};
use crate::domain::cv::dto::{
    CvResponse, CvSaved, CvText, SaveCvRequest, SaveCvResponse, Version, VersionMeta,
    VersionResponse, VersionsResponse,
};
use crate::errors::{ApiError, Problem};
use crate::events::{Event, Outcome, Taxonomy};
use crate::state::AppState;

fn iso_of(instant: chrono::DateTime<chrono::Utc>) -> String {
    instant.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The request id the access log will correlate (D18) — the same
/// resolution the layer does.
fn request_id_of(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

/// The body not processable: the canonical 400 of the shape (the UTF-8 of
/// the BODY travels apart — the 422 `cv-invalid-encoding` — ; a valid
/// UTF-8 body that is not the JSON of the contract is the 400).
fn bad_request(reason: &'static str, path: &str) -> Response {
    tracing::warn!(reason = reason, path = %path);
    Problem::new(ApiError::BadRequest {
        detail: "The body is not the JSON of the contract.".to_string(),
    })
    .on(path)
    .into_response()
}

/// `POST /v1/cv/text` (US4, FR-015/016/017): the validation fails fast in
/// the contract's order — the encoding (the body carries the CV's bytes)
/// → the size (≤ `cv_max_bytes`, 51200 exact accepted) → the PRECONDITION
/// `If-Unmodified-Since` (Q2) which the repository's transaction
/// arbitrates at the last instant (`FOR UPDATE`) — and the save lands
/// everything-or-nothing (ADR 0013): archive → upsert → purge.
pub async fn post_text(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: crate::middleware::auth::AuthenticatedUser,
    body: axum::body::Bytes,
) -> Response {
    const PATH: &str = "/v1/cv/text";

    // Fail fast 1 — the encoding: the content's bytes travel inside the
    // body, so a body that is not UTF-8 can never carry valid content
    // (422 of the catalogue, the contract's first gate).
    let body = match std::str::from_utf8(&body) {
        Ok(body) => body,
        Err(_) => {
            return Problem::new(ApiError::CvInvalidEncoding)
                .on(PATH)
                .into_response()
        }
    };

    // The JSON of the contract over the validated UTF-8 (the 400 of the
    // shape otherwise — never a text of serde).
    let request: SaveCvRequest = match serde_json::from_str(body) {
        Ok(request) => request,
        Err(_) => return bad_request("bad-request", PATH),
    };

    // Fail fast 2 — the size in BYTES (the cap of FR-015; the exact edge
    // of the limit is accepted, the data-model's backstop agrees).
    if request.content.len() > state.cv_max_bytes {
        return Problem::new(ApiError::CvTooLarge {
            max_bytes: state.cv_max_bytes,
        })
        .on(PATH)
        .into_response();
    }

    // The optimistic precondition (Q2): the header's `If-Unmodified-Since`
    // — an unreadable instant is the 400 of the shape; the FIRST write
    // (without a CV yet) requires no header.
    let expected = match headers
        .get("if-unmodified-since")
        .and_then(|value| value.to_str().ok())
    {
        None => None,
        Some(value) => match chrono::DateTime::parse_from_rfc3339(value) {
            Ok(instant) => Some(instant.with_timezone(&chrono::Utc)),
            Err(_) => {
                return Problem::new(ApiError::BadRequest {
                    detail: "The If-Unmodified-Since is not an ISO8601 instant.".to_string(),
                })
                .on(PATH)
                .into_response()
            }
        },
    };

    match cv_texts::save(
        &state.pool,
        user.user_id,
        &request.content,
        &request_id_of(&headers),
        expected,
        state.cv_versions_retention,
    )
    .await
    {
        Ok(outcome) => {
            // The D18 emission: `meta.bytes` + `previous_version_archived`
            // (the purge's accounting travels when the window closed — the
            // repository already emitted `cv.versions_purged` inside the
            // tx; the meta here records it against the save's event).
            let mut meta = Event::no_meta();
            meta.insert(
                "bytes".to_string(),
                serde_json::Value::from(request.content.len()),
            );
            meta.insert(
                "previous_version_archived".to_string(),
                serde_json::Value::from(outcome.previous_version_archived),
            );
            if outcome.purged_count > 0 {
                meta.insert(
                    "purged_count".to_string(),
                    serde_json::Value::from(outcome.purged_count),
                );
            }
            Event::new(
                Taxonomy::CvTextSaved,
                Outcome::Accepted,
                request_id_of(&headers),
                Some(user.user_id.to_string()),
                None,
                meta,
            )
            .emit();

            (
                StatusCode::CREATED,
                Json(SaveCvResponse {
                    cv: CvSaved {
                        id: outcome.id,
                        updated_at: iso_of(outcome.updated_at),
                    },
                    previous_version_archived: outcome.previous_version_archived,
                }),
            )
                .into_response()
        }
        Err(crate::db::RepoError::Stale(live)) => Problem::new(ApiError::StaleVersion {
            current_updated_at: iso_of(live),
        })
        .on(PATH)
        .into_response(),
        // The Sql of the tx answers the dependency's 503: NOTHING stays
        // half-way (the tx rolled back — the contract's promise).
        Err(_) => Problem::new(ApiError::DependencyUnavailable)
            .on(PATH)
            .into_response(),
    }
}

/// `GET /v1/cv/text` (FR-015): the live text of the credential's user —
/// the 404 is the only expected not-found (without a saved CV yet).
pub async fn get_text(
    State(state): State<AppState>,
    _headers: HeaderMap,
    user: crate::middleware::auth::AuthenticatedUser,
) -> Response {
    const PATH: &str = "/v1/cv/text";

    match cv_texts::get(&state.pool, user.user_id).await {
        Ok(Some(row)) => (
            StatusCode::OK,
            Json(CvResponse {
                cv: CvText {
                    id: row.id,
                    content: row.content,
                    created_at: iso_of(row.created_at),
                    updated_at: iso_of(row.updated_at),
                },
            }),
        )
            .into_response(),
        Ok(None) => Problem::new(ApiError::NotFound {
            detail: "The CV of the account does not exist yet.".to_string(),
        })
        .on(PATH)
        .into_response(),
        Err(_) => Problem::new(ApiError::DependencyUnavailable)
            .on(PATH)
            .into_response(),
    }
}

/// `GET /v1/cv/versions` (FR-018, SC-005): the history's METADATA — never
/// the contents — most recent first, within the retention window.
pub async fn list_versions(
    State(state): State<AppState>,
    _headers: HeaderMap,
    user: crate::middleware::auth::AuthenticatedUser,
) -> Response {
    const PATH: &str = "/v1/cv/versions";

    match cv_texts::list_versions(&state.pool, user.user_id, state.cv_versions_retention).await {
        Ok(rows) => (
            StatusCode::OK,
            Json(VersionsResponse {
                versions: rows
                    .into_iter()
                    .map(
                        |VersionMetaRow {
                             id,
                             origin,
                             archived_at,
                         }| VersionMeta {
                            id,
                            origin,
                            archived_at: iso_of(archived_at),
                        },
                    )
                    .collect(),
            }),
        )
            .into_response(),
        Err(_) => Problem::new(ApiError::DependencyUnavailable)
            .on(PATH)
            .into_response(),
    }
}

/// `GET /v1/cv/versions/{id}` (FR-018/019): one archived version with its
/// content — an id that does not exist OR belongs to another user answers
/// the SAME not-found (the anti-BOLA: never reveals the existence).
pub async fn get_version(
    State(state): State<AppState>,
    _headers: HeaderMap,
    user: crate::middleware::auth::AuthenticatedUser,
    version_id: axum::extract::Path<Uuid>,
) -> Response {
    const PATH: &str = "/v1/cv/versions/{id}";

    match cv_texts::version(&state.pool, user.user_id, version_id.0).await {
        Ok(Some(row)) => {
            let VersionRow {
                id,
                content,
                origin,
                archived_at,
            } = row;
            (
                StatusCode::OK,
                Json(VersionResponse {
                    version: Version {
                        id,
                        content,
                        origin,
                        archived_at: iso_of(archived_at),
                    },
                }),
            )
                .into_response()
        }
        Ok(None) => Problem::new(ApiError::NotFound {
            detail: "The version does not exist.".to_string(),
        })
        .on(PATH)
        .into_response(),
        Err(_) => Problem::new(ApiError::DependencyUnavailable)
            .on(PATH)
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support;
    use crate::middleware::auth::AuthenticatedUser;
    use axum::body::to_bytes;
    use sqlx::PgPool;

    /// The pool of the local DB of tests (None = the run skips — the same
    /// guard of every repository test; the CI always has it).
    async fn db() -> Option<PgPool> {
        test_support::pool().await
    }

    /// The state of the app over the pool of tests (the handlers only
    /// read the pool and the two config values of the CV).
    fn state_of(pool: &PgPool) -> AppState {
        let mut state = AppState::for_tests();
        state.pool = pool.clone();
        state
    }

    /// The identity of the test (the token opaque — the handlers only
    /// filter by the credential's user_id).
    fn user_of(id: Uuid) -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: id,
            token: "the-credential".to_string(),
        }
    }

    /// The POST of the CV: the raw body travels exactly as the router
    /// would hand it (the handler owns the JSON's decoding).
    async fn post(
        pool: &PgPool,
        user: Uuid,
        raw_body: Vec<u8>,
        if_unmodified_since: Option<String>,
    ) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(instant) = if_unmodified_since {
            headers.insert(
                "if-unmodified-since",
                instant.parse().expect("header value"),
            );
        }
        post_text(
            State(state_of(pool)),
            headers,
            user_of(user),
            axum::body::Bytes::from(raw_body),
        )
        .await
    }

    /// The JSON of a save request (the contract's body).
    fn body_of(content: &str) -> Vec<u8> {
        serde_json::to_vec(&SaveCvRequest {
            content: content.to_string(),
        })
        .expect("the DTO always serializes")
    }

    async fn json_of(response: Response) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("complete");
        serde_json::from_slice(&body).expect("the problem/contract answers carry JSON")
    }

    /// FR-015/016: the FIRST save answers the 201 of the contract — the
    /// row's identity, `previous_version_archived: false` — and the read
    /// agrees right after (the id stable, the created travels).
    #[tokio::test]
    async fn the_first_save_answers_201_unarchived() {
        let Some(pool) = db().await else {
            return;
        };
        let user = test_support::create_user(&pool).await;

        let response = post(
            &pool,
            user,
            body_of("Juan\nDesarrollador backend\nEXPERIENCIA..."),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED, "ADR 0007: Created");
        let body = json_of(response).await;
        assert_eq!(body["previous_version_archived"], false);
        assert!(
            body["cv"]["id"].as_str().is_some(),
            "the 201 carries the row's identity"
        );

        // The read agrees: the live text IS the saved one.
        let saved_id: Uuid = body["cv"]["id"].as_str().expect("id").parse().unwrap();
        let read = get_text(State(state_of(&pool)), HeaderMap::new(), user_of(user)).await;
        assert_eq!(read.status(), StatusCode::OK);
        let body = json_of(read).await;
        assert_eq!(body["cv"]["id"], saved_id.to_string());
        assert_eq!(
            body["cv"]["content"],
            "Juan\nDesarrollador backend\nEXPERIENCIA..."
        );
        chrono::DateTime::parse_from_rfc3339(body["cv"]["updated_at"].as_str().expect("instant"))
            .expect("the updated is ISO8601");
    }

    /// FR-016/018: the REPLACEMENT answers `previous_version_archived:
    /// true`, the history lists the archived metadata (most recent first,
    /// origin user, NO content) and the per-version read recovers the
    /// anterior content exactly.
    #[tokio::test]
    async fn the_replacement_archives_and_the_history_lists_and_reads() {
        let Some(pool) = db().await else {
            return;
        };
        let user = test_support::create_user(&pool).await;
        post(&pool, user, body_of("primera redacción"), None).await;
        let second = post(&pool, user, body_of("segunda redacción"), None).await;
        assert_eq!(second.status(), StatusCode::CREATED);
        let body = json_of(second).await;
        assert_eq!(
            body["previous_version_archived"], true,
            "the anterior content archived (FR-016)"
        );

        // The listing: metadata only, most recent first.
        let list = list_versions(State(state_of(&pool)), HeaderMap::new(), user_of(user)).await;
        assert_eq!(list.status(), StatusCode::OK);
        let listed = json_of(list).await;
        let versions = listed["versions"].as_array().expect("the listing");
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0]["origin"], "user", "the provenance of D15");
        assert!(
            versions[0].get("content").is_none(),
            "the listing NEVER carries the history's contents"
        );

        // The per-version read: the archived text travels.
        let version_id: Uuid = versions[0]["id"]
            .as_str()
            .expect("id")
            .parse()
            .expect("a UUID");
        let read = get_version(
            State(state_of(&pool)),
            HeaderMap::new(),
            user_of(user),
            axum::extract::Path(version_id),
        )
        .await;
        assert_eq!(read.status(), StatusCode::OK);
        let body = json_of(read).await;
        assert_eq!(body["version"]["content"], "primera redacción");
        assert_eq!(body["version"]["origin"], "user");
    }

    /// The fail fast of the contract (its order): a body not UTF-8 → 422
    /// `cv-invalid-encoding`; over the valid UTF-8, a body that is not
    /// the JSON of the contract → 400; and the BYTES' cap answers 413
    /// with its `max_bytes` (the exact edge 51200 IS accepted).
    #[tokio::test]
    async fn the_encoding_and_the_size_fail_fast_in_order() {
        let Some(pool) = db().await else {
            return;
        };
        let user = test_support::create_user(&pool).await;

        // The encoding: the 422 of the catalogue BEFORE anything else.
        let invalid = post(&pool, user, vec![0xC3, 0x28, 0xFF, 0xFE], None).await;
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = json_of(invalid).await;
        assert!(body["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("cv-invalid-encoding")));

        // The valid UTF-8 that is not the contract's JSON: the 400.
        let not_json = post(&pool, user, b"plain text, not json".to_vec(), None).await;
        assert_eq!(not_json.status(), StatusCode::BAD_REQUEST);

        // The exact edge: 51200 bytes accepted (the spec's edge).
        let edge = "a".repeat(51200);
        let ok_edge = post(&pool, user, body_of(&edge), None).await;
        assert_eq!(
            ok_edge.status(),
            StatusCode::CREATED,
            "51200 exact = accepted"
        );

        // One byte over: 413 with the catalogue's extension.
        let over = "a".repeat(51201);
        let too_large = post(&pool, user, body_of(&over), None).await;
        assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = json_of(too_large).await;
        assert!(body["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("cv-too-large")));
        assert_eq!(body["max_bytes"], 51200);
    }

    /// Q2: the stale rewrite answers the 412 with the catalogue's
    /// `current_updated_at` and NOTHING travelled — no archive, no
    /// overwrite (the same promise the repository tests prove).
    #[tokio::test]
    async fn the_stale_rewrite_answers_412_and_saves_nothing() {
        let Some(pool) = db().await else {
            return;
        };
        let user = test_support::create_user(&pool).await;

        post(&pool, user, body_of("CV vigente"), None).await;
        // A DIFFERENT second — the contract's comparison grain (the
        // profile's dialect: the responses quote seconds).
        let old = chrono::Utc::now() - chrono::Duration::seconds(2);
        let conflict = post(
            &pool,
            user,
            body_of("CV del conflicto"),
            Some(old.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)),
        )
        .await;
        assert_eq!(conflict.status(), StatusCode::PRECONDITION_FAILED);
        let body = json_of(conflict).await;
        assert!(body["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("stale-version")));
        assert!(
            body["current_updated_at"].as_str().is_some(),
            "the extension carries the current version"
        );

        // Nothing archived, the live text remains.
        let read =
            json_of(get_text(State(state_of(&pool)), HeaderMap::new(), user_of(user)).await).await;
        assert_eq!(read["cv"]["content"], "CV vigente");
        let archived: i64 =
            sqlx::query_scalar("select count(*) from cv_versions where user_id = $1")
                .bind(user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(archived, 0, "Q2: nothing archived in the conflict");
    }

    /// FR-019 (the anti-BOLA at the handlers): the queries NEVER cross
    /// users — the version of alice does not travel for bob (the 404 that
    /// never reveals), and bob's listing is empty.
    #[tokio::test]
    async fn the_cv_routes_never_cross_users() {
        let Some(pool) = db().await else {
            return;
        };
        let alice = test_support::create_user(&pool).await;
        let bob = test_support::create_user(&pool).await;

        post(&pool, alice, body_of("CV de alice"), None).await;
        post(&pool, alice, body_of("CV dos de alice"), None).await;

        let list = list_versions(State(state_of(&pool)), HeaderMap::new(), user_of(alice)).await;
        let listed = json_of(list).await;
        let id: Uuid = listed["versions"][0]["id"]
            .as_str()
            .expect("alice's history has its version")
            .parse()
            .unwrap();

        // Bob attacks alice's version: the SAME not-found.
        let stolen = get_version(
            State(state_of(&pool)),
            HeaderMap::new(),
            user_of(bob),
            axum::extract::Path(id),
        )
        .await;
        assert_eq!(stolen.status(), StatusCode::NOT_FOUND);

        // Bob's own history: empty — nothing of alice leaks in.
        let empty = list_versions(State(state_of(&pool)), HeaderMap::new(), user_of(bob)).await;
        let empty = json_of(empty).await;
        assert_eq!(empty["versions"].as_array().expect("list").len(), 0);
    }
}
