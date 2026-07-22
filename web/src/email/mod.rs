//! Email vertical — module wiring (ADR-0070). The API surface lives in `api.rs`;
//! the wasm-only UI in `component.rs`; pure host-tested helpers in `status.rs`.

mod api;
#[cfg(target_arch = "wasm32")]
mod component;
mod status;

pub use api::{request_email_verification, verify_email, RequestEmailVerification, VerifyEmail};
#[cfg(target_arch = "wasm32")]
pub use component::{EmailPage, VerifyEmailPage};
pub use status::{email_status_line, parse_verification_token};
