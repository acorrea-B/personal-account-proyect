//! The DTOs of the CV flow (T021, US4): the contract shapes of
//! `contracts/http-cv.md` — the save (201 of ADR 0007), the live read, the
//! history's metadata and the per-version read. The validation travels in
//! the HANDLER (fail fast in order, the contract's): UTF-8 → bytes.

use serde::{Deserialize, Serialize};

/// `POST /v1/cv/text` (FR-015): the plaintext CV, exact as received — the
/// JSON guarantees UTF-8; the byte cap and the encoding fail fast live in
/// the handler (the contract's order).
#[derive(Debug, Serialize, Deserialize)]
pub struct SaveCvRequest {
    pub content: String,
}

/// The saved CV (the contract's 201 — ADR 0007: Created, the resource
/// exists immediately; the id stays the SAME over the replacements).
#[derive(Debug, Serialize)]
pub struct CvSaved {
    pub id: uuid::Uuid,
    /// The ISO8601 instant the NEXT request's `If-Unmodified-Since` cites.
    pub updated_at: String,
}

/// 201 of the save: the row's identity + whether the anterior content went
/// to the history (FR-016).
#[derive(Debug, Serialize)]
pub struct SaveCvResponse {
    pub cv: CvSaved,
    pub previous_version_archived: bool,
}

/// The live CV (the contract's GET): the full text + the instants of its
/// life.
#[derive(Debug, Serialize)]
pub struct CvText {
    pub id: uuid::Uuid,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct CvResponse {
    pub cv: CvText,
}

/// The metadata of an archived version (FR-018): NO content — the listing
/// never carries the texts of the history.
#[derive(Debug, Serialize)]
pub struct VersionMeta {
    pub id: uuid::Uuid,
    /// The provenance (D15): 'user' in the beta ('ai' reserves post-beta).
    pub origin: String,
    pub archived_at: String,
}

#[derive(Debug, Serialize)]
pub struct VersionsResponse {
    pub versions: Vec<VersionMeta>,
}

/// One archived version with its content (the contract's inner body of
/// GET /v1/cv/versions/{id}).
#[derive(Debug, Serialize)]
pub struct Version {
    pub id: uuid::Uuid,
    pub content: String,
    pub origin: String,
    pub archived_at: String,
}

#[derive(Debug, Serialize)]
pub struct VersionResponse {
    pub version: Version,
}
