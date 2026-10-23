//! The business domain (US1+): DTOs (the contract shapes with their
//! validation) and handlers (the flows against the clients/repositories).
//!
//! The modules grow with their tasks: `auth` (US1/US2), `profile` (US3) and
//! `cv` (US4).

pub mod auth;
