//! Server-side mail sending.
//!
//! The transport-neutral pieces — the [`MailSender`] trait, `EmailMessage`,
//! `MailError`, and the dependency-free `NoopMailSender`/`CapturingMailSender`
//! — live in [`common::mailer`]. `common` is compiled to WebAssembly (as part
//! of the `web` CSR client), so it must stay free of native-only crates like `lettre`;
//! keeping the trait and data types there lets the web layer name the mailer in
//! `#[server]` functions without pulling a SMTP stack into the wasm build.
//!
//! The concrete senders here depend on `lettre` (async SMTP) and filesystem
//! I/O, so they are server-only. Per [ADR-0016](../../../docs/adr/0016-dependency-injection-and-appstate.md)
//! they are constructed at the composition root and injected per-consumer
//! rather than bundled into shared state:
//!
//! - [`LettreMailSender`] — production SMTP transport (in [`smtp`]).
//! - [`FileMailSender`] — JSON-line capture for end-to-end tests (in [`file`]).
//! - [`build_mailer`] — selects one based on environment and stored config.

mod factory;
mod file;
mod smtp;

pub use factory::build_mailer;
pub use file::FileMailSender;
pub use smtp::{BuildMailerError, LettreMailSender};
