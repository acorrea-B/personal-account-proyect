//! The tower layer of the rate limiting: it classifies the request, gates
//! the keys of its policy and answers 429 with `Retry-After` (FR-021),
//! emitting `security.rate_limited` (D18) in every block.
//!
//! One shared store per function instance (in-memory, per cold start): the
//! beta does not require a distributed store.
//!
//! Keying per policy (R2/FR-005): the global limit keys by IP; the failed
//! logins count only the 401s per IP; the resends key by IP AND email; the
//! CV writes key by the identity of the credential.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tower::{Layer, Service};
use uuid::Uuid;

use crate::errors::{ApiError, Problem};
use crate::events::{Event, Outcome, Taxonomy};
use crate::middleware::auth::SignatureVerifier;
use crate::middleware::ratelimit::{Clock, Policy, RateLimiter, Rates, SystemClock};

/// The tower layer: the shared gate of the instance.
#[derive(Clone)]
pub struct RateLimitLayer {
    gate: Arc<Gate>,
}

struct Gate {
    limiter: Arc<Mutex<RateLimiter>>,
    clock: Arc<dyn Clock>,
    rates: Rates,
    verifier: Arc<dyn SignatureVerifier>,
    /// Cap of the read body to key the resends by email.
    resend_body_cap: usize,
}

impl RateLimitLayer {
    /// The layer of the instance: one shared limiter, the wall clock, the
    /// configured rates and the credentials verifier (for the user keys).
    pub fn new(rates: Rates, verifier: Arc<dyn SignatureVerifier>, resend_body_cap: usize) -> Self {
        Self {
            gate: Arc::new(Gate {
                limiter: Arc::new(Mutex::new(RateLimiter::new())),
                clock: Arc::new(SystemClock),
                rates,
                verifier,
                resend_body_cap,
            }),
        }
    }

    #[cfg(test)]
    pub fn for_tests(rates: Rates, clock: Arc<dyn Clock>) -> Self {
        Self {
            gate: Arc::new(Gate {
                limiter: Arc::new(Mutex::new(RateLimiter::new())),
                clock,
                rates,
                verifier: Arc::new(crate::middleware::hs256::Hs256Verifier::new(
                    "test-jwt-secret",
                    Some("supabase"),
                    "authenticated",
                )),
                resend_body_cap: usize::MAX,
            }),
        }
    }
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitMiddleware {
            inner,
            gate: self.gate.clone(),
        }
    }
}

/// The service of the rate limit (tower): classifies, gates and records.
#[derive(Clone)]
pub struct RateLimitMiddleware<S> {
    inner: S,
    gate: Arc<Gate>,
}

impl<S> Service<axum::extract::Request> for RateLimitMiddleware<S>
where
    S: Service<axum::extract::Request, Response = Response, Error = std::convert::Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: axum::extract::Request) -> Self::Future {
        let gate = self.gate.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let method = request.method().to_string();
            let path = request.uri().path().to_string();
            let request_id = request
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            // The global limit gates every request (per IP, R2) upfront.
            let ip = client_ip(request.headers());
            let ip_key = ip.clone().unwrap_or_else(|| "unknown".to_string());
            if let Some((_key, wait)) =
                gate.gate(Policy::GlobalRequest, std::slice::from_ref(&ip_key), true)
            {
                return Ok(gate.blocked(&request_id, ip.as_deref(), None, &path, "ip", wait));
            }

            // The sensitive flows of R2 besides the global one.
            let policy = Policy::of(&method, &path);
            match policy {
                Policy::CvWrite => {
                    // R2: the write keys by user (the credential); without
                    // a valid credential the gate does not count here — the
                    // auth rejects the request.
                    if let Some(user_id) = gate.identity(request.headers()) {
                        if let Some((_key, wait)) = gate.gate(policy, &[user_id.to_string()], true)
                        {
                            return Ok(gate.blocked(
                                &request_id,
                                ip.as_deref(),
                                Some(user_id),
                                &path,
                                "user",
                                wait,
                            ));
                        }
                    }
                }
                Policy::ResendVerification => {
                    // FR-005: per IP AND account: the body is buffered to
                    // key the email; an unreadable body keys only by IP. A
                    // blocked resend does not consume the cupo of the key.
                    let email = resend_email(&mut request, gate.resend_body_cap).await;
                    let email_key = email.clone().unwrap_or_else(|| ip_key.clone());
                    let label = if email.is_some() { "user" } else { "ip" };
                    if let Some((_key, wait)) =
                        gate.gate(policy, &[ip_key.clone(), email_key.clone()], false)
                    {
                        return Ok(gate.blocked(
                            &request_id,
                            ip.as_deref(),
                            None,
                            &path,
                            label,
                            wait,
                        ));
                    }
                    gate.record(policy, &ip_key);
                    gate.record(policy, &email_key);
                }
                // R2: the counter of the FAILED logins only feeds on the
                // 401s of the response (pre-gated below).
                Policy::LoginFailure => {
                    if let Some((_key, wait)) =
                        gate.gate(policy, std::slice::from_ref(&ip_key), false)
                    {
                        return Ok(gate.blocked(
                            &request_id,
                            ip.as_deref(),
                            None,
                            &path,
                            "ip",
                            wait,
                        ));
                    }
                }
                _ => {}
            }

            let response = tower::Service::call(&mut inner, request).await?;

            // R2: the counter of the FAILED logins only feeds on 401s.
            if policy == Policy::LoginFailure && response.status() == StatusCode::UNAUTHORIZED {
                gate.record(Policy::LoginFailure, &ip_key);
            }

            Ok(with_request_id(response, &request_id))
        })
    }
}

impl Gate {
    /// Gates the keys of a policy (a blocked request does not consume the
    /// cupo of the policy; the upfront flows record all their keys just
    /// before going forward). Resolves to the (key, waiting time) blocked.
    fn gate(
        &self,
        policy: Policy,
        keys: &[String],
        count_upfront: bool,
    ) -> Option<(String, Duration)> {
        let mut limiter = self.limiter.lock().expect("the limiter never poisons");
        let mut blocked = None;
        for key in keys {
            if let Some(wait) = limiter.allowed(policy, key, &self.rates, self.clock.as_ref()) {
                blocked = Some((key.clone(), wait));
            }
        }
        if blocked.is_none() && count_upfront {
            for key in keys {
                limiter.record(policy, key, self.clock.as_ref());
            }
        }
        blocked
    }

    /// Consumes one cupo (the counting side of the failed logins).
    fn record(&self, policy: Policy, key: &str) {
        let mut limiter = self.limiter.lock().expect("the limiter never poisons");
        limiter.record(policy, key, self.clock.as_ref());
    }

    /// Emits `security.rate_limited` (one event per significant outcome —
    /// D18) and builds the canonical 429 answer with `Retry-After`.
    fn blocked(
        &self,
        request_id: &str,
        ip: Option<&str>,
        user_id: Option<Uuid>,
        route: &str,
        key: &str,
        wait: Duration,
    ) -> Response {
        let mut meta = Event::no_meta();
        meta.insert(
            "route".to_string(),
            serde_json::Value::String(route.to_string()),
        );
        meta.insert(
            "key".to_string(),
            serde_json::Value::String(key.to_string()),
        );

        Event::new(
            Taxonomy::SecurityRateLimited,
            Outcome::Blocked,
            request_id,
            user_id.map(|id| id.to_string()),
            ip.map(|ip| ip.to_string()),
            meta,
        )
        .emit();

        Problem::new(ApiError::RateLimited {
            retry_after_seconds: wait.as_secs().max(1),
        })
        .on(route)
        .into_response()
    }

    /// The identity certified by the Authorization header (None without a
    /// valid credential — the gate does not count here: the auth rejects).
    fn identity(&self, headers: &axum::http::HeaderMap) -> Option<Uuid> {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim())?;
        let claims = self.verifier.verify(token).ok()?;
        claims.sub.parse::<Uuid>().ok()
    }
}

/// The first value of `x-forwarded-for` (behind the edge of the platform
/// the client IP arrives in this header).
fn client_ip(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(|first| first.trim().to_string())
}

/// The email of the body of a resend, buffered: the body must travel
/// complete towards the handler.
async fn resend_email(request: &mut axum::extract::Request, cap: usize) -> Option<String> {
    let bytes = to_bytes(std::mem::replace(request.body_mut(), Body::empty()), cap)
        .await
        .ok()?;
    let body: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let email = body
        .get("email")
        .and_then(|email| email.as_str())
        .map(|email| email.trim().to_lowercase())?;

    *request.body_mut() = Body::from(bytes);
    Some(email)
}

/// Exposes the request id generated here via the correlation header (the
/// access log reads it, D18 rule 3).
fn with_request_id(mut response: Response, request_id: &str) -> Response {
    if response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.is_empty())
        .unwrap_or(true)
    {
        if let Ok(value) = HeaderValue::from_str(request_id) {
            response.headers_mut().insert("x-request-id", value);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::ratelimit::FakeClock;
    use crate::state::AppState;
    use tower::ServiceExt;

    /// The layered service for the layer tests: the health route (the global
    /// limit) + a stub login that answers 401 to test the failed-login
    /// counting of R2.
    fn layered_router(state: AppState, layer: RateLimitLayer) -> RateLimitMiddleware<axum::Router> {
        let router = axum::Router::new()
            .route("/health", axum::routing::get(|| async { "ok" }))
            .route(
                "/v1/auth/login",
                axum::routing::post(|| async {
                    // A login that always fails, for the counter R2.
                    crate::errors::Problem::new(ApiError::InvalidCredentials)
                }),
            )
            .with_state(state);

        layer.layer(router)
    }

    async fn call<
        S: tower::Service<
                axum::extract::Request,
                Response = Response,
                Error = std::convert::Infallible,
            > + Clone
            + Send,
    >(
        service: &mut S,
        request: axum::extract::Request,
    ) -> Response {
        service
            .clone()
            .oneshot(request)
            .await
            .expect("the router never fails")
    }

    /// The global limit answers 429 with the canonical document and the
    /// `Retry-After`; FR-021 (estados 429 con tiempo de espera).
    #[tokio::test]
    async fn the_global_limit_answers_429_with_retry_after() {
        let clock = Arc::new(FakeClock::start());
        let layer = RateLimitLayer::for_tests(
            Rates {
                login_failed_per_min: 5,
                resend_per_10min: 3,
                cv_write_per_min: 10,
                request_per_min: 2,
            },
            clock.clone(),
        );
        let mut router = layered_router(AppState::for_tests(), layer);

        for _ in 0..2 {
            let response = call(
                &mut router,
                axum::http::Request::get("/health")
                    .body(Body::empty())
                    .expect("valid"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }

        let response = call(
            &mut router,
            axum::http::Request::get("/health")
                .body(Body::empty())
                .expect("valid"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
            Some("60")
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("complete");
        let document: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");
        assert_eq!(
            document["type"],
            "https://api.example.com/errors/rate-limited"
        );
        assert_eq!(document["retry_after_seconds"], 60);
        assert_eq!(document["instance"], "/health");
    }

    /// The failed logins: 5 per minute (R2) — the 6th 401 within the window
    /// answers 429, and the window expires with the clock advanced.
    #[tokio::test]
    async fn the_failed_logins_limit_at_five_per_minute() {
        let clock = Arc::new(FakeClock::start());
        let layer = RateLimitLayer::for_tests(
            Rates {
                login_failed_per_min: 5,
                resend_per_10min: 3,
                cv_write_per_min: 10,
                request_per_min: 120,
            },
            clock.clone(),
        );
        let mut router = layered_router(AppState::for_tests(), layer);

        for _ in 0..5 {
            let response = call(
                &mut router,
                axum::http::Request::post("/v1/auth/login")
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::empty())
                    .expect("valid"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        let response = call(
            &mut router,
            axum::http::Request::post("/v1/auth/login")
                .header("x-forwarded-for", "203.0.113.10")
                .body(Body::empty())
                .expect("valid"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        clock.advance(Duration::from_secs(61));
        let response = call(
            &mut router,
            axum::http::Request::post("/v1/auth/login")
                .header("x-forwarded-for", "203.0.113.10")
                .body(Body::empty())
                .expect("valid"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// The successful requests do not consume the cupo of the failed
    /// logins: only the 401s feed it (R2).
    #[tokio::test]
    async fn the_login_cupo_only_feeds_on_the_401s() {
        let clock = Arc::new(FakeClock::start());
        let layer = RateLimitLayer::for_tests(
            Rates {
                login_failed_per_min: 2,
                resend_per_10min: 3,
                cv_write_per_min: 10,
                request_per_min: 120,
            },
            clock.clone(),
        );
        let mut router = layered_router(AppState::for_tests(), layer);

        for _ in 0..3 {
            let response = call(
                &mut router,
                axum::http::Request::get("/health")
                    .body(Body::empty())
                    .expect("valid"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }

        for _ in 0..2 {
            let response = call(
                &mut router,
                axum::http::Request::post("/v1/auth/login")
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::empty())
                    .expect("valid"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let response = call(
            &mut router,
            axum::http::Request::post("/v1/auth/login")
                .header("x-forwarded-for", "203.0.113.10")
                .body(Body::empty())
                .expect("valid"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    /// The IP of the client behind the edge comes from `x-forwarded-for`
    /// (first value); the requests of two IPs do not share cupo.
    #[tokio::test]
    async fn the_forwarded_ips_do_not_share_cupo() {
        let clock = Arc::new(FakeClock::start());
        let layer = RateLimitLayer::for_tests(
            Rates {
                login_failed_per_min: 5,
                resend_per_10min: 3,
                cv_write_per_min: 10,
                request_per_min: 1,
            },
            clock.clone(),
        );
        let mut router = layered_router(AppState::for_tests(), layer);

        let request = axum::http::Request::get("/health")
            .header("x-forwarded-for", "203.0.113.10, 198.51.100.2")
            .body(Body::empty())
            .expect("valid");
        assert_eq!(call(&mut router, request).await.status(), StatusCode::OK);

        // The first IP exhausted its cupo; another IP still has one.
        let request = axum::http::Request::get("/health")
            .header("x-forwarded-for", "203.0.113.10, 198.51.100.2")
            .body(Body::empty())
            .expect("valid");
        assert_eq!(
            call(&mut router, request).await.status(),
            StatusCode::TOO_MANY_REQUESTS
        );

        let request = axum::http::Request::get("/health")
            .header("x-forwarded-for", "198.51.100.2")
            .body(Body::empty())
            .expect("valid");
        assert_eq!(call(&mut router, request).await.status(), StatusCode::OK);
    }

    /// The request id generated by the layer travels back in
    /// `x-request-id` (D18 rule 3: correlation with the access log).
    #[tokio::test]
    async fn the_generated_request_id_travels_back() {
        let clock = Arc::new(FakeClock::start());
        let layer = RateLimitLayer::for_tests(
            Rates {
                login_failed_per_min: 5,
                resend_per_10min: 3,
                cv_write_per_min: 10,
                request_per_min: 120,
            },
            clock.clone(),
        );
        let mut router = layered_router(AppState::for_tests(), layer);

        let request = axum::http::Request::get("/health")
            .header("x-forwarded-for", "203.0.113.10")
            .body(Body::empty())
            .expect("valid");
        let response = call(&mut router, request).await;

        assert!(response
            .headers()
            .get("x-request-id")
            .and_then(|value| Uuid::parse_str(value.to_str().expect("ascii")).ok())
            .is_some());
    }
}
