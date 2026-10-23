//! The auth flow (US1/US2): the public routes of the credential lifecycle
//! (register/resend/session) with the D3 rule underneath — the user id never
//! arrives from the client; it arrives from the credential itself.

pub mod dto;
pub mod dto_session;
pub mod handlers;
