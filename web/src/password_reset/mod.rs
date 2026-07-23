//! Password-reset vertical — module wiring (ADR-0070). The API surface lives in
//! `api.rs`; the wasm-only UI in `component.rs`.

mod api;

pub use api::{
    confirm_password_reset, request_password_reset, ConfirmPasswordReset, RequestPasswordReset,
};
