//! The profile domain (US3): the mirror's public read and its full-put
//! update (FR-012..014), always filtered by the `user_id` of the
//! credential (D3) — the email of the mirror never changes here.

pub mod dto;
pub mod handlers;
