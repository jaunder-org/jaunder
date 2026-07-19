//! Client-side domain-value form validation (#414): validate a field by parsing input
//! into a domain newtype — the same `FromStr` the typed `#[server]`-arg `Deserialize`
//! routes through — and surface the newtype's own message inline. See ADR (draft):
//! `docs/adr/0065-client-side-domain-validation.md`.
//!
//! Client-side form primitives: `Field<T>` state, its validator, and the
//! `ValidatedInput` widget.

#[cfg(target_arch = "wasm32")]
mod component;
mod field;

#[cfg(target_arch = "wasm32")]
pub use component::ValidatedInput;
pub use field::{field_error, Field};
