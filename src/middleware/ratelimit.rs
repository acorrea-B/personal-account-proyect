//! In-memory rate limiting of the sensitive flows (FR-021, limits R2).
//!
//! Sliding-window counters with expiry: every key (policy + ip/user/email)
//! keeps the timestamps of the attempts inside its window; the pruning
//! happens during the counting. The clock is injectable — the simulated
//! clock of the tests keeps them deterministic.
//!
//! Counting policy per route (plan §3, R2):
//! - `POST /v1/auth/login` — FAILED logins only (R2, FR-006): the attempt is
//!   recorded when the answer is a 401.
//! - `POST /v1/auth/resend-verification` — per IP AND email (FR-005), 3
//!   every 10 minutes: the body is read (buffered) to key the account.
//! - `POST /v1/cv/text` — per user (R2), 10/min.
//! - everything else — global requests per IP, 120/min (R2).

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Clock injected for the expiry of the windows (the tests inject a
/// simulated one).
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// The wall clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Simulated clock of the tests: advanced by hand, no real waiting.
#[cfg(test)]
pub struct FakeClock {
    now: std::sync::Mutex<Instant>,
}

#[cfg(test)]
impl FakeClock {
    pub fn start() -> Self {
        Self {
            now: std::sync::Mutex::new(Instant::now()),
        }
    }

    pub fn advance(&self, delta: Duration) {
        *self.now.lock().expect("the fake clock never poisons") += delta;
    }
}

#[cfg(test)]
impl Clock for FakeClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("the fake clock never poisons")
    }
}

/// The rate limits configured of R2, resolved once per cold start.
#[derive(Debug, Clone, Copy)]
pub struct Rates {
    /// Failed logins per IP, per minute.
    pub login_failed_per_min: u32,
    /// Verification resends (FR-005), per 10 minutes.
    pub resend_per_10min: u32,
    /// CV writes per user, per minute.
    pub cv_write_per_min: u32,
    /// Global requests per IP, per minute.
    pub request_per_min: u32,
}

/// The policy class of a request: the limits of R2 per sensitive flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Policy {
    /// Any request (global, per IP).
    GlobalRequest,
    /// `POST /v1/auth/login` (the counter only feeds on the 401s).
    LoginFailure,
    /// `POST /v1/auth/resend-verification` (per IP and email).
    ResendVerification,
    /// `POST /v1/cv/text` (per user).
    CvWrite,
}

impl Policy {
    /// The policy of a request by its method and path. The insensitive
    /// routes still fall under the global limit.
    pub fn of(method: &str, path: &str) -> Self {
        match (method, path) {
            ("POST", "/v1/auth/login") => Policy::LoginFailure,
            ("POST", "/v1/auth/resend-verification") => Policy::ResendVerification,
            ("POST", "/v1/cv/text") => Policy::CvWrite,
            _ => Policy::GlobalRequest,
        }
    }

    /// The limit of the policy from the configured rates (R2).
    pub fn limit(self, rates: &Rates) -> u32 {
        match self {
            Policy::GlobalRequest => rates.request_per_min,
            Policy::LoginFailure => rates.login_failed_per_min,
            Policy::ResendVerification => rates.resend_per_10min,
            Policy::CvWrite => rates.cv_write_per_min,
        }
    }

    /// The window of the policy (constant; only it fixes it).
    pub fn window(self) -> Duration {
        match self {
            Policy::GlobalRequest | Policy::LoginFailure | Policy::CvWrite => {
                Duration::from_secs(60)
            }
            Policy::ResendVerification => Duration::from_secs(10 * 60),
        }
    }
}

/// The counters of the policies: keyed by (policy, ip|user|email).
pub struct RateLimiter {
    events: HashMap<(Policy, String), VecDeque<Instant>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            events: HashMap::new(),
        }
    }

    /// Whether the key has cupo left in the window: `None` allow (without
    /// consuming it); `Some(wait)` when the window is full — `wait` is the
    /// time towards the expiry of the oldest attempt (the exact value of
    /// the `Retry-After`).
    pub fn allowed(
        &mut self,
        policy: Policy,
        key: &str,
        rates: &Rates,
        clock: &dyn Clock,
    ) -> Option<Duration> {
        let limit = policy.limit(rates);
        let window = policy.window();
        let now = clock.now();
        self.prune(policy, key, window, now);
        let count = self
            .events
            .get(&(policy, key.to_string()))
            .map_or(0, VecDeque::len);

        if count >= limit as usize {
            // The oldest inside the window is the first to expire.
            let oldest = self
                .events
                .get(&(policy, key.to_string()))
                .and_then(|queue| queue.front())
                .copied()
                .expect("a full window has at least one attempt");
            return Some(oldest + window - now);
        }

        None
    }

    /// Consumes one cupo of the key (the request goes forward). Called
    /// before a non-sensitive dispatch or after the 401 of a login.
    pub fn record(&mut self, policy: Policy, key: &str, clock: &dyn Clock) {
        let window = policy.window();
        let now = clock.now();
        self.events
            .entry((policy, key.to_string()))
            .or_default()
            .push_back(now);
        self.prune(policy, key, window, now);
    }

    /// Drops the attempts older than the window; the window duration is
    /// needed by the callers (they resolve it from the policy).
    fn prune(&mut self, policy: Policy, key: &str, window: Duration, now: Instant) -> Duration {
        let queue = self.events.entry((policy, key.to_string())).or_default();
        while queue
            .front()
            .is_some_and(|attempt| now.duration_since(*attempt) > window)
        {
            queue.pop_front();
        }
        window
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATES: Rates = Rates {
        login_failed_per_min: 5,
        resend_per_10min: 3,
        cv_write_per_min: 10,
        request_per_min: 120,
    };

    /// The window expires: after 6 blocked minutes of failed logins, the
    /// cupo returns (the attempts older than the window are pruned).
    #[test]
    fn the_window_expires_and_the_attempts_prune() {
        let clock = FakeClock::start();
        let mut limiter = RateLimiter::new();

        for _ in 0..5 {
            limiter.record(Policy::LoginFailure, "ip-1", &clock);
        }
        assert!(limiter
            .allowed(Policy::LoginFailure, "ip-1", &RATES, &clock)
            .is_some());

        clock.advance(Duration::from_secs(61));
        assert!(limiter
            .allowed(Policy::LoginFailure, "ip-1", &RATES, &clock)
            .is_none());
    }

    /// The `Retry-After` counts towards the expiry of the OLDEST attempt:
    /// at t=20s with a 60s window, 40s remain.
    #[test]
    fn the_retry_after_points_to_the_expiry_of_the_oldest() {
        let clock = FakeClock::start();
        let mut limiter = RateLimiter::new();

        for _ in 0..5 {
            limiter.record(Policy::LoginFailure, "ip-1", &clock);
        }
        clock.advance(Duration::from_secs(20));

        let wait = limiter
            .allowed(Policy::LoginFailure, "ip-1", &RATES, &clock)
            .expect("the window is full");
        assert_eq!(wait.as_secs(), 40);
    }

    /// Other keys are independent: the block of one IP does not touch the
    /// cupo of another (nor the one of another policy — the isolation of
    /// the flows).
    #[test]
    fn the_keys_are_independent_between_themselves() {
        let clock = FakeClock::start();
        let mut limiter = RateLimiter::new();

        for _ in 0..5 {
            limiter.record(Policy::LoginFailure, "ip-1", &clock);
        }
        assert!(limiter
            .allowed(Policy::LoginFailure, "ip-2", &RATES, &clock)
            .is_none());

        let resend = limiter
            .allowed(Policy::ResendVerification, "ip-1", &RATES, &clock)
            .is_none();
        assert!(resend, "the resend policy does not inherit the login cupo");
    }

    /// The resend window is 10 minutes (FR-005): at 5 minutes it is still
    /// blocked, after 10 it returns.
    #[test]
    fn the_resend_window_is_ten_minutes() {
        let clock = FakeClock::start();
        let mut limiter = RateLimiter::new();

        for _ in 0..3 {
            limiter.record(Policy::ResendVerification, "user@mail.com", &clock);
        }
        assert!(limiter
            .allowed(Policy::ResendVerification, "user@mail.com", &RATES, &clock)
            .is_some());

        clock.advance(Duration::from_secs(5 * 60));
        assert!(limiter
            .allowed(Policy::ResendVerification, "user@mail.com", &RATES, &clock)
            .is_some());

        clock.advance(Duration::from_secs(5 * 60 + 1));
        assert!(limiter
            .allowed(Policy::ResendVerification, "user@mail.com", &RATES, &clock)
            .is_none());
    }

    /// The limit of CV writes per user comes from the configuration (R2).
    #[test]
    fn the_cv_limit_comes_from_the_configuration() {
        let clock = FakeClock::start();
        let mut limiter = RateLimiter::new();
        let rates = Rates {
            cv_write_per_min: 1,
            ..RATES
        };

        limiter.record(Policy::CvWrite, "user", &clock);
        assert!(limiter
            .allowed(Policy::CvWrite, "user", &rates, &clock)
            .is_some());
    }

    /// The sensitive routes classify their policy; the rest is global.
    #[test]
    fn the_policies_classify_by_route() {
        assert_eq!(Policy::of("POST", "/v1/auth/login"), Policy::LoginFailure);
        assert_eq!(
            Policy::of("POST", "/v1/auth/resend-verification"),
            Policy::ResendVerification
        );
        assert_eq!(Policy::of("POST", "/v1/cv/text"), Policy::CvWrite);
        assert_eq!(Policy::of("GET", "/v1/profile"), Policy::GlobalRequest);
        assert_eq!(Policy::of("PUT", "/v1/profile"), Policy::GlobalRequest);
        assert_eq!(Policy::of("GET", "/health"), Policy::GlobalRequest);
    }
}
