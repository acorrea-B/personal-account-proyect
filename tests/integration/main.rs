//! Root of the integration test binary (T010+): one process, one harness —
//! the modules are the user story suites (one per task).

mod auth_ratelimit;
mod common;
mod cv_flow;
mod cv_versions_retention;
mod profile_flow;
mod profile_mirror;
mod register;
mod session_flow;
mod wiring;
