//! Password-reset vertical — module wiring (ADR-0070). The API surface lives in
//! `api.rs`; the wasm-only UI in `component.rs`.

mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{Confirm, Request, confirm, request};
#[cfg(target_arch = "wasm32")]
pub use component::{ForgotPasswordPage, ResetPasswordPage};
