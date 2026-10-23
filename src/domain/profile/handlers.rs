//! The handlers of the profile (T018, US3): the read of the own mirror
//! (FR-012/013) and its full-put with optimistic precondition (Q2) — the
//! D3 underneath: the user_id only comes from the credential.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use validator::Validate;

use crate::db::profiles::{self, ProfileRow};
use crate::domain::profile::dto::{links_policy_of, ProfileLink, UpdateProfileRequest};
use crate::errors::{ApiError, Problem};
use crate::events::{Event, Outcome, Taxonomy};
use crate::state::AppState;

/// The profile's public face (the contract): FR-013 — nothing of the
/// internals travels (`processing_consent` reserved, D15; the email of the
/// mirror = the columns of the contract's shape only).
#[derive(Debug, Serialize)]
pub struct ProfileResponse {
    pub id: uuid::Uuid,
    pub email: String,
    pub full_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<ProfileLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    pub created_at: String,
    pub updated_at: String,
}

fn response_of(row: &ProfileRow) -> ProfileResponse {
    ProfileResponse {
        id: row.id,
        email: row.email.clone(),
        full_name: row.full_name.clone(),
        headline: row.headline.clone(),
        bio: row.bio.clone(),
        location: row.location.clone(),
        links: row
            .links
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        skills: row
            .skills
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok()),
        created_at: row
            .created_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        updated_at: row
            .updated_at
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

/// The request id the access log will correlate (D18) — the same
/// resolution the layer does.
fn request_id_of(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// The body rejected by the extractor: the canonical 400.
fn bad_request(rejection: JsonRejection, path: &str) -> Response {
    tracing::warn!(reason = "bad-request", path = %path, %rejection);
    Problem::new(ApiError::BadRequest {
        detail: "The body is not the JSON of the contract.".to_string(),
    })
    .on(path)
    .into_response()
}

/// The validation of the request → the per-field extensions of the
/// catalogue (422 `profile-invalid-field` with `fields` = the reasons).
fn fields_error_of(
    errors: &validator::ValidationErrors,
    request: &UpdateProfileRequest,
) -> ApiError {
    let mut fields = std::collections::BTreeMap::new();
    for field in errors.field_errors().keys() {
        fields.insert(
            field.to_string(),
            "the rule of the contract does not hold".to_string(),
        );
    }
    // The links' scheme (http/https) — the validator's url accepts the
    // others: the tightening of the contract has its own check.
    if !links_policy_of(&request.links) {
        fields.insert("url".to_string(), "the url must be http(s)".to_string());
    }
    ApiError::ProfileInvalidField { fields }
}

/// `GET /v1/profile` (FR-012/013, US3): the profile of the credential's
/// identity — a mirrorless verified account (the trigger's edge) is the
/// `not-found` of the catalogue; the contract does not expect it otherwise.
pub async fn get_profile(
    State(state): State<AppState>,
    _headers: HeaderMap,
    user: crate::middleware::auth::AuthenticatedUser,
) -> Response {
    const PATH: &str = "/v1/profile";

    match profiles::get(&state.pool, user.user_id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(response_of(&row))).into_response(),
        Ok(None) => Problem::new(ApiError::NotFound {
            detail: "The profile of the account does not exist yet.".to_string(),
        })
        .on(PATH)
        .into_response(),
        // The read's only real failure = the repository's answer: the
        // dependency 503 (the Missing/Stale are impossible in a read;
        // the catalogue's not-found already resolved the None).
        Err(_) => Problem::new(ApiError::DependencyUnavailable)
            .on(PATH)
            .into_response(),
    }
}

/// `PUT /v1/profile` (FR-012/014, US3): full put (the absent = emptied),
/// the optimistic precondition `If-Unmodified-Since` (Q2) inside the
/// repository's transaction; the event carries the fields the request
/// gave a value (meta.fields, D18).
pub async fn put_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    user: crate::middleware::auth::AuthenticatedUser,
    body: Result<Json<UpdateProfileRequest>, JsonRejection>,
) -> Response {
    const PATH: &str = "/v1/profile";

    let mut request = match body {
        Ok(Json(request)) => request,
        Err(rejection) => return bad_request(rejection, PATH),
    };

    // full_name: 1..200 AFTER the trim (the rule verbatim of data-model).
    request.full_name = request.full_name.trim().to_string();
    if let Err(errors) = request.validate() {
        let error = fields_error_of(&errors, &request);
        return Problem::new(error).on(PATH).into_response();
    }
    // The links' http(s) scheme (the validator's url accepts the others —
    // the contract's tightening has its own check).
    if !links_policy_of(&request.links) {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("url".to_string(), "the url must be http(s)".to_string());
        return Problem::new(ApiError::ProfileInvalidField { fields })
            .on(PATH)
            .into_response();
    }

    // The optimistic precondition (Q2): the header's instant of
    // `If-Unmodified-Since` — an unreadable one is the 400 of the shape.
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

    let patch = profiles::ProfileUpdate {
        full_name: request.full_name.clone(),
        headline: request.headline.clone(),
        bio: request.bio.clone(),
        location: request.location.clone(),
        links: request
            .links
            .clone()
            .map(|links| serde_json::to_value(&links).expect("the links always serialize")),
        skills: request
            .skills
            .clone()
            .map(|skills| serde_json::to_value(&skills).expect("the skills always serialize")),
        // Reserved of the D15 (post-beta): the API never sets it.
        processing_consent: None,
    };

    match profiles::update(&state.pool, user.user_id, expected, patch).await {
        Ok(row) => {
            let fields = provided_field_names(&request);
            let mut meta = Event::no_meta();
            if let Ok(fields_json) = serde_json::to_value(&fields) {
                meta.insert("fields".to_string(), fields_json);
            }
            Event::new(
                Taxonomy::ProfileUpdated,
                Outcome::Accepted,
                request_id_of(&headers),
                Some(user.user_id.to_string()),
                None,
                meta,
            )
            .emit();
            (StatusCode::OK, Json(response_of(&row))).into_response()
        }
        Err(crate::db::RepoError::Stale(live)) => Problem::new(ApiError::StaleVersion {
            current_updated_at: live.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        })
        .on(PATH)
        .into_response(),
        Err(crate::db::RepoError::Missing) => Problem::new(ApiError::NotFound {
            detail: "The profile of the account does not exist yet.".to_string(),
        })
        .on(PATH)
        .into_response(),
        Err(crate::db::RepoError::Sql(_)) => Problem::new(ApiError::DependencyUnavailable)
            .on(PATH)
            .into_response(),
    }
}

/// The D18's meta.fields: the names of the fields the request GAVE VALUES
/// (full_name + the optionals' presence — the full-put's empties do not
/// count as modifications by value).
fn provided_field_names(request: &UpdateProfileRequest) -> Vec<&'static str> {
    let mut fields = vec!["full_name"];
    if request.headline.is_some() {
        fields.push("headline");
    }
    if request.bio.is_some() {
        fields.push("bio");
    }
    if request.location.is_some() {
        fields.push("location");
    }
    if request.links.is_some() {
        fields.push("links");
    }
    if request.skills.is_some() {
        fields.push("skills");
    }
    fields
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support;
    use crate::middleware::auth::AuthenticatedUser;
    use axum::body::to_bytes;
    use axum::http::header::IF_UNMODIFIED_SINCE;
    use axum::http::Request;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    /// The pool of the local DB of tests (None = the run skips: the same
    /// guard of every repository test — the CI always has it).
    async fn db() -> Option<PgPool> {
        test_support::pool().await
    }

    /// The state of the app over the pool of tests (the pool of for_tests
    /// points elsewhere: the field swaps in — the handlers only read it).
    fn state_of(pool: &PgPool) -> AppState {
        let mut state = AppState::for_tests();
        state.pool = pool.clone();
        state
    }

    /// The mirrored user (the trigger's effect replicated — the handlers
    /// do not depend on how the mirror was created).
    async fn mirror_user(pool: &PgPool) -> Uuid {
        let user = test_support::create_user(pool).await;
        let email = format!("mirror-{user}@test.local");
        sqlx::query("insert into profiles (id, email, full_name) values ($1, $2, 'User')")
            .bind(user)
            .bind(&email)
            .execute(pool)
            .await
            .expect("the mirror inserts");
        user
    }

    /// The identity of the test (the token opaque — the flows of these
    /// tests do not verify credentials, only mirror rows).
    fn user_of(id: Uuid) -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: id,
            token: "the-credential".to_string(),
        }
    }

    async fn read(pool: &PgPool, user: Uuid) -> Response {
        get_profile(State(state_of(pool)), HeaderMap::new(), user_of(user)).await
    }

    async fn write(
        pool: &PgPool,
        user: Uuid,
        body: serde_json::Value,
        if_unmodified_since: Option<&str>,
    ) -> Response {
        let request: UpdateProfileRequest = serde_json::from_value(body).expect("the fixture");
        let mut headers = HeaderMap::new();
        if let Some(instant) = if_unmodified_since {
            headers.insert(
                IF_UNMODIFIED_SINCE,
                instant.parse().expect("the fixture instant"),
            );
        }
        put_profile(
            State(state_of(pool)),
            headers,
            user_of(user),
            Ok(Json(request)),
        )
        .await
    }

    async fn body_of(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("reads");
        serde_json::from_slice(&bytes).expect("the answers are JSON")
    }

    /// The read of the own mirror echoes the contract shape (FR-012/013):
    /// the placeholder of Q1, the email of the mirror + the ISO instants —
    /// and NOTHING internal (FR-013: `processing_consent` never travels).
    #[tokio::test]
    async fn the_read_answers_the_public_shape_without_internals() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;
        let response = read(&pool, user).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_of(response).await;
        assert_eq!(body["full_name"], "User", "the placeholder of Q1");
        assert_eq!(
            body["email"],
            format!("mirror-{user}@test.local"),
            "the mirror's email travels verbatim"
        );
        chrono::DateTime::parse_from_rfc3339(body["created_at"].as_str().expect("instant"))
            .expect("the created is ISO8601");
        chrono::DateTime::parse_from_rfc3339(body["updated_at"].as_str().expect("instant"))
            .expect("the updated is ISO8601");
        assert!(
            body.get("processing_consent").is_none(),
            "FR-013: the reserved field never travels"
        );
    }

    /// A verified account without a mirror row (the trigger's edge): the
    /// not-found of the catalogue — the read-only flow never invents.
    #[tokio::test]
    async fn the_read_of_a_mirrorless_account_answers_not_found() {
        let Some(pool) = db().await else { return };
        let user = test_support::create_user(&pool).await;

        let response = read(&pool, user).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(body_of(response).await["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("not-found")));
    }

    /// The FULL PUT (FR-014): the fields omitted arrive as None and the
    /// update EMPTIES them — the contract's full-put, not a patch.
    #[tokio::test]
    async fn the_full_put_empties_the_fields_it_omits() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;

        let full = serde_json::json!({
            "full_name": "Juan Perez",
            "headline": "Backend",
            "bio": "diez años",
            "location": "Buenos Aires, AR",
            "skills": ["rust"]
        });
        let response = write(&pool, user, full.clone(), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_of(response).await["full_name"], "Juan Perez");

        // The put of ONLY the name: the rest empties (the full-put).
        let response = write(
            &pool,
            user,
            serde_json::json!({ "full_name": "Juan" }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_of(response).await;
        assert_eq!(body["full_name"], "Juan");
        assert!(body.get("headline").is_none(), "the omitted = emptied");
        assert!(body.get("bio").is_none(), "the omitted = emptied");
        assert!(body.get("skills").is_none(), "the omitted = emptied");
    }

    /// The values that the request carries reflect with their shape
    /// (the links' type of the contract and the skills' list).
    #[tokio::test]
    async fn the_put_reflects_the_values_of_the_request() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;
        let response = write(
            &pool,
            user,
            serde_json::json!({
                "full_name": "Juan Perez",
                "headline": "Desarrollador backend",
                "links": [{ "type": "github", "url": "https://github.com/juan" }],
                "skills": ["rust", "postgres"]
            }),
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_of(response).await;
        assert_eq!(body["links"][0]["type"], "github");
        assert_eq!(body["links"][0]["url"], "https://github.com/juan");
        assert_eq!(body["skills"][1], "postgres");
    }

    /// `profile-invalid-field` (422) with its `fields` the extension: the
    /// empty name (after the trim, FR-014) and the lengthly limits.
    #[tokio::test]
    async fn the_invalid_fields_answer_422_with_their_names() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;

        let (status, body) = {
            let response =
                write(&pool, user, serde_json::json!({ "full_name": "   " }), None).await;
            (response.status(), body_of(response).await)
        };
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("profile-invalid-field")));
        assert!(
            body["fields"].get("full_name").is_some(),
            "the name: 1..200 after the trim"
        );

        let (status, body) = {
            let response = write(
                &pool,
                user,
                serde_json::json!({ "full_name": "x".repeat(201) }),
                None,
            )
            .await;
            (response.status(), body_of(response).await)
        };
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["fields"].get("full_name").is_some(), "≤ 200 chars");

        let (status, body) = {
            let response = write(
                &pool,
                user,
                serde_json::json!({
                    "full_name": "Juan",
                    "headline": "x".repeat(121)
                }),
                None,
            )
            .await;
            (response.status(), body_of(response).await)
        };
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body["fields"].get("headline").is_some(), "≤ 120 chars");
    }

    /// The data-model's rules of the lists: ≥ the limit, the duplicates
    /// and the url's scheme — everything in the `fields` extension.
    #[tokio::test]
    async fn the_lists_limits_and_duplicates_answer_422() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;

        let eleven_links: Vec<_> = (0..11)
            .map(|n| serde_json::json!({ "type": "site", "url": format!("https://n{n}.example") }))
            .collect();
        let response = write(
            &pool,
            user,
            serde_json::json!({ "full_name": "Juan", "links": eleven_links }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            body_of(response).await["fields"].get("links").is_some(),
            "≤ 10"
        );

        let response = write(
            &pool,
            user,
            serde_json::json!({
                "full_name": "Juan",
                "skills": ["rust", "Rust"] // the duplicates (insensitive)
            }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            body_of(response).await["fields"].get("skills").is_some(),
            "no duplicates"
        );

        let response = write(
            &pool,
            user,
            serde_json::json!({
                "full_name": "Juan",
                "links": [{ "type": "site", "url": "ftp://no-es-web" }]
            }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            body_of(response).await["fields"].get("url").is_some(),
            "the http(s) only"
        );
    }

    /// The optimistic precondition (Q2): the write with a stale version
    /// answers 412 with `current_updated_at` and NOTHING was saved.
    #[tokio::test]
    async fn the_stale_write_answers_412_and_saves_nothing() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;
        let _ = read(&pool, user).await; // (the row's state material.)

        let response = write(
            &pool,
            user,
            serde_json::json!({ "full_name": "El Robado" }),
            Some("2020-01-01T00:00:00Z"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::PRECONDITION_FAILED);
        let body = body_of(response).await;
        assert!(body["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("stale-version")));
        assert!(
            body["current_updated_at"].as_str().is_some(),
            "the live version names itself"
        );
        assert!(
            !body["type"].as_str().unwrap_or_default().is_empty(),
            "the document carries its type"
        );

        // Nothing written: the mirror keeps its placeholder.
        let after = read(&pool, user).await;
        assert_eq!(body_of(after).await["full_name"], "User", "nothing saved");
    }

    /// The precondition's instant of the LIVE row: the write passes and
    /// the version rotates (the next write's precondition takes the NEW).
    #[tokio::test]
    async fn the_correct_precondition_writes_and_rotates_the_version() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;
        // The contract's precision = seconds: the mirror's INSERT and the
        // first write share the second otherwise (the tests are fast) —
        // the clock leaves that second BEFORE the read, so the write
        // REALLY rotates the version.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let read_body = body_of(read(&pool, user).await).await;
        let live_version = read_body["updated_at"].as_str().expect("the live instant");

        let response = write(
            &pool,
            user,
            serde_json::json!({ "full_name": "Juan Perez" }),
            Some(live_version),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "the optimistic pass");

        // The version of the answer = the NEW instant (the precondition of
        // next write): a write with the OLD instant answers 412.
        let new_version = body_of(response).await["updated_at"]
            .as_str()
            .expect("the new instant")
            .to_string();
        let stale = write(
            &pool,
            user,
            serde_json::json!({ "full_name": "Sin la nueva versión" }),
            Some(live_version),
        )
        .await;
        assert_eq!(stale.status(), StatusCode::PRECONDITION_FAILED);
        let body_of_stale: serde_json::Value = body_of(stale).await;
        assert_eq!(body_of_stale["current_updated_at"], new_version);
    }

    /// A precondition of another shape: the 400 of the catalogue.
    #[tokio::test]
    async fn a_malformed_precondition_answers_400() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;

        let response = write(
            &pool,
            user,
            serde_json::json!({ "full_name": "Juan" }),
            Some("no-es-una-instante"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(body_of(response).await["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("bad-request")));
    }
    /// The body that is not the JSON of the contract: the 400 canonical
    /// (at the router level: the rejection of the extractor is of the
    /// wiring — the same proof of the session flows).
    #[tokio::test]
    async fn the_put_of_a_body_that_is_not_json_answers_the_400() {
        // The route is protected: the gate of the credential runs FIRST
        // (the test path's token passes it; the body's rejection = next).
        let state = AppState::for_tests();
        let token = crate::middleware::hs256::Hs256Verifier::test_token(
            &state,
            "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5",
            false,
        );
        let router = crate::router::build_router(state);
        let response = router
            .oneshot(
                Request::put("/v1/profile")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("no-es-json"))
                    .expect("the request is valid"),
            )
            .await
            .expect("the router never fails");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// The mirrorless verified account (the trigger's edge): the PUT
    /// answers the not-found of the catalogue (the optimistic's arbitration
    /// needs the row to exist).
    #[tokio::test]
    async fn the_put_of_a_mirrorless_account_answers_not_found() {
        let Some(pool) = db().await else { return };
        let user = test_support::create_user(&pool).await;
        let response = write(
            &pool,
            user,
            serde_json::json!({ "full_name": "Sin Espejo" }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(body_of(response).await["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("not-found")));
    }

    /// The strings of the skills: the 41st char = 422 with its field
    /// (≤ 40, no duplicates — the rule verbatim).
    #[tokio::test]
    async fn the_41_char_skill_answers_422_with_its_field() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;
        let response = write(
            &pool,
            user,
            serde_json::json!({
                "full_name": "Juan",
                "skills": ["x".repeat(41)]
            }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(body_of(response).await["fields"].get("skills").is_some());
    }

    /// The combinator (the fields_error_of's own path): MULTIPLE rules
    /// broken at once — the length and the scheme travel in the SAME
    /// document with each one's name.
    #[tokio::test]
    async fn the_combined_violations_carry_every_field() {
        let Some(pool) = db().await else { return };
        let user = mirror_user(&pool).await;
        let response = write(
            &pool,
            user,
            serde_json::json!({
                "full_name": "x".repeat(201),
                "headline": "y".repeat(121),
                "links": [{ "type": "site", "url": "ftp://no-es-web" }]
            }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_of(response).await;
        for field in ["full_name", "headline", "url"] {
            assert!(
                body["fields"].get(field).is_some(),
                "the field {field} named"
            );
        }
    }

    /// The DB's fall: the pool of a dead connection (the lazy pool
    /// resolves its error at the first use) — the PUT answers the
    /// dependency 503 (the edge of the spec: nothing of the write runs).
    #[tokio::test]
    async fn the_db_failure_of_the_put_answers_the_dependency_503() {
        let dead_pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(2))
            .connect_lazy("postgres://postgres:x@127.0.0.1:1/none")
            .expect("the connection string is valid");
        let state = {
            let mut state = AppState::for_tests();
            state.pool = dead_pool;
            state
        };
        let response = put_profile(
            State(state.clone()),
            HeaderMap::new(),
            user_of(Uuid::new_v4()),
            Ok(Json(UpdateProfileRequest {
                full_name: "Con la DB muerta".to_string(),
                headline: None,
                bio: None,
                location: None,
                links: None,
                skills: None,
            })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(body_of(response).await["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("dependency-unavailable")));
    }

    /// The DB's fall of the read: the same dependency 503 (the mirror's
    /// query never answers from a dead pool).
    #[tokio::test]
    async fn the_db_failure_of_the_read_answers_the_dependency_503() {
        let dead_pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(2))
            .connect_lazy("postgres://postgres:x@127.0.0.1:1/none")
            .expect("the connection string is valid");
        let state = {
            let mut state = AppState::for_tests();
            state.pool = dead_pool;
            state
        };
        let response = get_profile(
            State(state.clone()),
            HeaderMap::new(),
            user_of(Uuid::new_v4()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(body_of(response).await["type"]
            .as_str()
            .is_some_and(|t| t.ends_with("dependency-unavailable")));
    }
}
