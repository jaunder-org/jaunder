//! Email vertical — module wiring (ADR-0070). The API surface lives in `api.rs`;
//! the wasm-only UI in `component.rs`; pure host-tested helpers in `status.rs`.

mod api;

pub use api::{request_email_verification, verify_email, RequestEmailVerification, VerifyEmail};
