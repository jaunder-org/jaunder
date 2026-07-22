//! Invites vertical — module wiring (ADR-0070). The API surface lives in
//! `api.rs`; the wasm-only UI in `component.rs`.

mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{create_invite, list_invites, CreateInvite, InviteInfo, ListInvites};
#[cfg(target_arch = "wasm32")]
pub use component::InvitesPage;
