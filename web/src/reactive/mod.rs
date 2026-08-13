//! Reactive revalidation core shared across `web` verticals.
//!
//! [`Invalidator`] is the canonical revalidation idiom (design record:
//! `docs/adr/0060-web-invalidator-revalidation-idiom.md`): a committed mutation
//! `notify()`s an invalidator, and every resource that `track()`s it refetches. This module
//! owns the host-tested core — `new` / `notify` / `track`; the `invalidator_scope!`
//! context-scope newtype lives in the [`scope`] leaf. The browser-bound helpers built on it
//! (`resource` / `action` / `patched` / `sticky`, the latter two driving ADR-0061's keyed
//! list and its sticky peer) live in `client::reactive` (#515) — wasm-only and e2e-exercised.

mod invalidator;

pub use invalidator::Invalidator;

// The macro's consumers are wasm-only `component.rs` files; the `test` arm keeps the
// generated newtype host-tested. Gating the leaf here (not inside it) is ADR-0070's
// file-level split — `scope.rs` carries no cfg of its own.
#[cfg(any(target_arch = "wasm32", test))]
mod scope;
// Same gate as the `mod` above, deliberately: `invalidator.rs`'s `tests` module consumes
// this re-export on a host test build, and it in turn is what consumes `scope.rs`'s own
// `pub(crate) use`. Gating either one wasm-only would leave the other unconsumed on
// host and trip `unused_imports` (denied).
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) use scope::invalidator_scope;
