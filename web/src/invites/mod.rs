//! Invites vertical — module wiring (ADR-0070). The API surface lives in
//! `api.rs`; the wasm-only UI in `component.rs`.

mod api;

pub use api::{create_invite, list_invites, CreateInvite, InviteInfo, ListInvites};
