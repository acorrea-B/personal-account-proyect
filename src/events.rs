//! Emission of the business events (D18 — plan §13.5).
//!
//! Closed taxonomy and common envelope, log-only in beta: every event
//! travels through `tracing` as structured fields (never free text) and
//! correlates with the access log through `request_id`. The `audit_events`
//! table is post-beta and additive: it will write the same envelope/taxonomy
//! (§13.5 rule 5).
//!
//! Rules of the standard: one event per significant outcome (not per
//! layer); the `reason` of any meta uses catalogue slugs always (R9);
//! `ip` only in security/rate-limit events; no tokens, no full PII (§13.4).

use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value};

use crate::errors::ApiError;

/// Outcome of the event (envelope of §13.5). It drives the tracing level:
/// `Accepted` logs at info, the rest at warn (§13.4: warn for 4xx).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Accepted,
    Rejected,
    Blocked,
}

impl Outcome {
    /// The `outcome` value of the envelope (closed vocabulary).
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Accepted => "accepted",
            Outcome::Rejected => "rejected",
            Outcome::Blocked => "blocked",
        }
    }

    /// `info` for accepted, `warn` for rejected/blocked.
    fn level(self) -> Level {
        if self == Outcome::Accepted {
            Level::Info
        } else {
            Level::Warn
        }
    }
}

/// Level of the envelope.
#[derive(Debug, Clone, Copy)]
enum Level {
    Info,
    Warn,
}

impl Level {
    // Only in tests for now: the envelope becomes material with
    // audit_events (post-beta, §13.5 rule 5).
    #[cfg(test)]
    fn as_str(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
        }
    }
}

/// Closed taxonomy (plan §13.5): amplifying by PR means one pass that adds
/// the table entry, the variant and the test, like the error catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taxonomy {
    /// handler register.
    AuthRegistered,
    /// resend (after the generic decision — FR-005/006).
    AuthVerificationSent,
    /// flow of confirmation (profile created — FR-004).
    AuthEmailVerified,
    /// handler login.
    AuthLoginSucceeded,
    /// handler login, `meta.reason` = catalogue slug (R9).
    AuthLoginFailed,
    /// handler refresh.
    AuthSessionRefreshed,
    /// middleware/handler refresh, `meta.reason` = `invalid-refresh-token` |
    /// `refresh-token-reused`.
    AuthRefreshRejected,
    /// handler revoke (FR-009).
    AuthSessionRevoked,
    /// handler profile, `meta.fields` = modified fields.
    ProfileUpdated,
    /// handler cv, `meta.bytes` + `previous_version_archived`.
    CvTextSaved,
    /// cv repository (tx), `meta.purged_count` + `kept = 10`.
    CvVersionsPurged,
    /// middleware ratelimit, `meta.route` + `key = ip|user`.
    SecurityRateLimited,
    /// middleware auth, `meta.route` + `meta.reason` = canonical slug.
    SecurityAuthBlocked,
    /// (reserved: emitted only on a real forced cross-access test or case).
    SecurityBolaBlocked,
}

impl Taxonomy {
    /// Value of the `event` field (the closed table of §13.5).
    fn as_str(self) -> &'static str {
        match self {
            Taxonomy::AuthRegistered => "auth.registered",
            Taxonomy::AuthVerificationSent => "auth.verification_sent",
            Taxonomy::AuthEmailVerified => "auth.email_verified",
            Taxonomy::AuthLoginSucceeded => "auth.login_succeeded",
            Taxonomy::AuthLoginFailed => "auth.login_failed",
            Taxonomy::AuthSessionRefreshed => "auth.session_refreshed",
            Taxonomy::AuthRefreshRejected => "auth.refresh_rejected",
            Taxonomy::AuthSessionRevoked => "auth.session_revoked",
            Taxonomy::ProfileUpdated => "profile.updated",
            Taxonomy::CvTextSaved => "cv.text_saved",
            Taxonomy::CvVersionsPurged => "cv.versions_purged",
            Taxonomy::SecurityRateLimited => "security.rate_limited",
            Taxonomy::SecurityAuthBlocked => "security.auth_blocked",
            Taxonomy::SecurityBolaBlocked => "security.bola_blocked",
        }
    }
}

/// One business event of the envelope D18.
pub struct Event {
    taxonomy: Taxonomy,
    outcome: Outcome,
    request_id: String,
    user_id: Option<String>,
    ip: Option<String>,
    meta: Map<String, Value>,
}

impl Event {
    pub fn new(
        taxonomy: Taxonomy,
        outcome: Outcome,
        request_id: impl Into<String>,
        user_id: Option<String>,
        ip: Option<String>,
        meta: Map<String, Value>,
    ) -> Self {
        Self {
            taxonomy,
            outcome,
            request_id: request_id.into(),
            user_id,
            ip,
            meta,
        }
    }

    /// Empty meta for the events without extensions.
    pub fn no_meta() -> Map<String, Value> {
        Map::new()
    }

    /// Convenience of R9: the `reason` of any meta is the slug of a
    /// catalogue error — the type makes another vocabulary impossible.
    pub fn reason_of(slug: ApiError) -> &'static str {
        slug.slug()
    }

    /// The envelope of the contract: `ts`, `event`, `request_id` and
    /// `outcome` obligatory; `user_id`/`ip` per event; `meta` minimal.
    // Only in tests for now: with `audit_events` (post-beta, §13.5 rule 5)
    // this envelope becomes material and this method gains a writer.
    #[cfg(test)]
    fn envelope(&self) -> Map<String, Value> {
        let mut envelope = Map::new();
        envelope.insert(
            "ts".to_string(),
            Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
        );
        envelope.insert(
            "level".to_string(),
            Value::String(self.outcome.level().as_str().to_string()),
        );
        envelope.insert(
            "event".to_string(),
            Value::String(self.taxonomy.as_str().to_string()),
        );
        envelope.insert(
            "request_id".to_string(),
            Value::String(self.request_id.clone()),
        );
        envelope.insert("user_id".to_string(), self.user_id.clone().into());
        envelope.insert("ip".to_string(), self.ip.clone().into());
        envelope.insert(
            "outcome".to_string(),
            Value::String(self.outcome.as_str().to_string()),
        );
        envelope.insert("meta".to_string(), Value::Object(self.meta.clone()));
        envelope
    }

    /// Emits the event through `tracing` as structured fields; the JSON
    /// subscriber of production renders the envelope (§13.4).
    pub fn emit(self) {
        let meta_json = serde_json::to_string(&self.meta).expect("the meta always serializes");
        let user_id = self.user_id.as_deref();
        let ip = self.ip.as_deref();
        let event = self.taxonomy.as_str();
        let outcome = self.outcome.as_str();
        let request_id = &self.request_id;
        let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

        // One record per level: the fields stay first-class for the log
        // queries, with the envelope fields verbatim.
        match self.outcome.level() {
            Level::Info => tracing::info!(target: "event",
                request_id = %request_id, user_id = ?user_id, ip = ?ip,
                ts = %ts, meta = %meta_json,
                event = %event, outcome = %outcome),
            Level::Warn => tracing::warn!(target: "event",
                request_id = %request_id, user_id = ?user_id, ip = ?ip,
                ts = %ts, meta = %meta_json,
                event = %event, outcome = %outcome),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ApiError;

    /// A request id of the format uuid (the envelope carries it verbatim).
    const REQUEST_ID: &str = "3f2d1a9c-8b7e-4f21-9c65-a1b2c3d4e5f6";

    /// The envelope of an accepted event carries the mandatory fields, the
    /// level info and an empty meta.
    #[test]
    fn the_envelope_of_an_accepted_event_is_the_contract_one() {
        let event = Event::new(
            Taxonomy::AuthRegistered,
            Outcome::Accepted,
            REQUEST_ID,
            Some("c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5".to_string()),
            None,
            Event::no_meta(),
        );
        let envelope = event.envelope();

        // ts: ISO8601 readable by a RFC3339 parser.
        chrono::DateTime::parse_from_rfc3339(envelope["ts"].as_str().expect("ts present"))
            .expect("the ts is ISO8601");

        assert_eq!(envelope["level"], "info");
        assert_eq!(envelope["event"], "auth.registered");
        assert_eq!(envelope["request_id"], REQUEST_ID);
        assert_eq!(envelope["user_id"], "c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5");
        assert_eq!(envelope["ip"], serde_json::Value::Null);
        assert_eq!(envelope["outcome"], "accepted");
        assert_eq!(envelope["meta"], serde_json::json!({}));
    }

    /// Rejected and blocked events log at warn (§13.4).
    #[test]
    fn the_rejected_and_blocked_outcomes_log_at_warn() {
        for (taxonomy, outcome) in [
            (Taxonomy::AuthLoginFailed, Outcome::Rejected),
            (Taxonomy::SecurityRateLimited, Outcome::Blocked),
            (Taxonomy::SecurityAuthBlocked, Outcome::Blocked),
        ] {
            let event = Event::new(
                taxonomy,
                outcome,
                REQUEST_ID,
                None,
                Some("203.0.113.10".to_string()),
                Event::no_meta(),
            );
            assert_eq!(event.envelope()["level"], "warn");
            assert_eq!(event.envelope()["outcome"], outcome.as_str());
        }
    }

    /// The closed taxonomy: every variant prints the exact table value.
    #[test]
    fn the_taxonomy_strings_are_the_closed_table() {
        for (taxonomy, name) in [
            (Taxonomy::AuthRegistered, "auth.registered"),
            (Taxonomy::AuthVerificationSent, "auth.verification_sent"),
            (Taxonomy::AuthEmailVerified, "auth.email_verified"),
            (Taxonomy::AuthLoginSucceeded, "auth.login_succeeded"),
            (Taxonomy::AuthLoginFailed, "auth.login_failed"),
            (Taxonomy::AuthSessionRefreshed, "auth.session_refreshed"),
            (Taxonomy::AuthRefreshRejected, "auth.refresh_rejected"),
            (Taxonomy::AuthSessionRevoked, "auth.session_revoked"),
            (Taxonomy::ProfileUpdated, "profile.updated"),
            (Taxonomy::CvTextSaved, "cv.text_saved"),
            (Taxonomy::CvVersionsPurged, "cv.versions_purged"),
            (Taxonomy::SecurityRateLimited, "security.rate_limited"),
            (Taxonomy::SecurityAuthBlocked, "security.auth_blocked"),
            (Taxonomy::SecurityBolaBlocked, "security.bola_blocked"),
        ] {
            assert_eq!(taxonomy.as_str(), name);
        }
    }

    /// R9: every `reason` of the meta is a slug of the error catalogue —
    /// the vocabulary is one, never free text.
    #[test]
    fn the_reasons_are_always_catalogue_slugs() {
        assert_eq!(
            Event::reason_of(ApiError::InvalidCredentials),
            "invalid-credentials"
        );
        // The two reasons of `auth.refresh_rejected` (§13.5).
        assert_eq!(
            Event::reason_of(ApiError::InvalidRefreshToken { detail: "".into() }),
            "invalid-refresh-token"
        );
        assert_eq!(
            Event::reason_of(ApiError::RefreshTokenReused),
            "refresh-token-reused"
        );
        // The reason of `security.auth_blocked` is a canonical error.
        assert_eq!(
            Event::reason_of(ApiError::Unauthorized { detail: "".into() }),
            "unauthorized"
        );
    }

    /// The meta travels in the envelope: the extensions of
    /// `profile.updated` and of `cv.text_saved` with their own names.
    #[test]
    fn the_meta_travels_in_the_envelope() {
        let mut meta = Event::no_meta();
        meta.insert("bytes".to_string(), Value::from(1024));
        meta.insert("previous_version_archived".to_string(), Value::Bool(true));

        let event = Event::new(
            Taxonomy::CvTextSaved,
            Outcome::Accepted,
            REQUEST_ID,
            Some("c0a8e1f2-33b4-4c5d-8e9f-a0b1c2d3e4f5".to_string()),
            None,
            meta,
        );
        let envelope = event.envelope();
        assert_eq!(envelope["meta"]["bytes"], 1024);
        assert_eq!(envelope["meta"]["previous_version_archived"], true);
    }

    /// `ip` only in the security events (rule 4): the envelope carries it
    /// verbatim when the event declares it.
    #[test]
    fn the_security_events_carry_the_ip() {
        let event = Event::new(
            Taxonomy::SecurityRateLimited,
            Outcome::Blocked,
            REQUEST_ID,
            None,
            Some("203.0.113.10".to_string()),
            Event::no_meta(),
        );
        assert_eq!(event.envelope()["ip"], "203.0.113.10");
    }
}
