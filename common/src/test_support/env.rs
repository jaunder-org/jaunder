//! The workspace's single seam for mutating the process environment in tests.
//!
//! `std::env::set_var` / `remove_var` are `unsafe` as of edition 2024 (RFC 3543):
//! they are not thread-safe, and a mutation racing *any* concurrent read — including
//! reads inside libc, on a thread the test never spawned — is undefined behaviour.
//!
//! [`with_env`] is the only place in this workspace that calls either, so the whole
//! obligation is discharged in one auditable function. Everything else asks for a
//! critical section instead of taking one.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::sync::{Mutex, PoisonError};

/// Serializes every test that reads or writes the process environment.
///
/// One lock for the whole workspace, rather than the per-module `Mutex` this
/// replaced: two modules in one test binary held two *different* mutexes and so did
/// not serialize against each other at all.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Runs `f` with serialized, exclusive access to the process environment, restoring
/// every variable `f` touched to its prior value on the way out — including when `f`
/// panics.
///
/// Mutate through the [`Env`] handle; a test that only *reads* the environment still
/// wraps in `with_env(|_env| …)`, because the lock's job is to serialize readers
/// against writers, not merely writers against each other.
///
/// Illustration only — `resolve` and `Format` belong to `server`, which cannot be
/// named from `common`:
///
/// ```text
/// with_env(|env| {
///     env.set("JAUNDER_LOG_FORMAT", "json");
///     env.remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
///     assert_eq!(resolve().format, Format::Json);
/// });
/// ```
///
/// **Not reentrant** — nesting one `with_env` inside another deadlocks. Apply the
/// whole delta through the single handle instead; interleaving mutations and
/// assertions within one closure is supported and is the reason the handle exists.
///
/// Lock poisoning is deliberately ignored: the lock guards no invariant of its own,
/// so a test that panics while holding it must not cascade into every later test.
pub fn with_env<R>(f: impl FnOnce(&mut Env) -> R) -> R {
    let _guard = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);

    // `Env`'s `Drop` restores, so an unwind out of `f` is covered without
    // `catch_unwind` — and without swallowing the panic.
    let mut env = Env {
        prior: HashMap::new(),
    };
    f(&mut env)
}

/// The mutation handle [`with_env`] lends to its closure.
///
/// Borrowed from the closure and never returned by it, so a caller cannot hold env
/// access open past the critical section — which is exactly why `with_env` takes a
/// closure rather than returning a guard.
pub struct Env {
    /// The value each touched key had on *first* touch, so set-then-change-then-remove
    /// still restores the original rather than an intermediate.
    prior: HashMap<OsString, Option<OsString>>,
}

impl Env {
    /// Sets `key` to `value` for the rest of the critical section.
    pub fn set(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) {
        let key = key.as_ref();
        self.remember(key);
        // SAFETY: `with_env` holds `ENV_LOCK` for the whole closure, and this module
        // is the workspace's only caller of the env-mutating functions, so no other
        // test thread is reading or writing the environment concurrently.
        unsafe { std::env::set_var(key, value.as_ref()) };
    }

    /// Unsets `key` for the rest of the critical section.
    pub fn remove(&mut self, key: impl AsRef<OsStr>) {
        let key = key.as_ref();
        self.remember(key);
        // SAFETY: as in `set` — the `ENV_LOCK` held by `with_env` makes this the only
        // thread touching the environment.
        unsafe { std::env::remove_var(key) };
    }

    /// Records `key`'s pre-existing value the first time it is touched.
    fn remember(&mut self, key: &OsStr) {
        self.prior
            .entry(key.to_os_string())
            .or_insert_with(|| std::env::var_os(key));
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (key, prior) in self.prior.drain() {
            match prior {
                // SAFETY: still inside `with_env`'s closure scope, so `ENV_LOCK` is
                // still held — `Env` is dropped before the guard.
                Some(value) => unsafe { std::env::set_var(&key, value) },
                None => unsafe { std::env::remove_var(&key) },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test uses a key of its own: `with_env` serializes access, but two tests
    /// sharing a key would still be asserting on each other's fixture.
    const A: &str = "JAUNDER_TEST_WITH_ENV_A";
    const B: &str = "JAUNDER_TEST_WITH_ENV_B";
    const C: &str = "JAUNDER_TEST_WITH_ENV_C";
    const D: &str = "JAUNDER_TEST_WITH_ENV_D";
    const E: &str = "JAUNDER_TEST_WITH_ENV_E";
    const F: &str = "JAUNDER_TEST_WITH_ENV_F";

    /// Reads a variable outside any critical section — for asserting on what
    /// `with_env` left behind.
    fn get(key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    #[test]
    fn restores_prior_value_after_closure() {
        with_env(|env| env.set(A, "before"));
        with_env(|env| {
            env.set(A, "during");
            assert_eq!(get(A).as_deref(), Some("during"));
        });
        // The outer `with_env` restored `A` to unset, not to "before" — so assert the
        // inner one restored what it actually found.
        assert_eq!(get(A), None);
    }

    #[test]
    fn restores_prior_value_when_closure_panics() {
        with_env(|env| {
            env.set(B, "before");

            let unwound = std::panic::catch_unwind(|| {
                // A nested `with_env` would deadlock, so mutate through a fresh
                // `Env` bound to this scope — the same `Drop` path `with_env` uses.
                let mut inner = Env {
                    prior: HashMap::new(),
                };
                inner.set(B, "during");
                assert_eq!(get(B).as_deref(), Some("during"));
                panic!("boom");
            });

            assert!(unwound.is_err(), "the closure was supposed to panic");
            assert_eq!(
                get(B).as_deref(),
                Some("before"),
                "unwinding must restore the prior value"
            );
        });
    }

    #[test]
    fn removes_variable_that_was_previously_unset() {
        assert_eq!(get(C), None, "fixture key must start unset");
        with_env(|env| {
            env.set(C, "transient");
            assert_eq!(get(C).as_deref(), Some("transient"));
        });
        assert_eq!(get(C), None, "a previously-unset key must end unset");
    }

    #[test]
    fn supports_interleaved_states_in_one_acquisition() {
        with_env(|env| {
            env.set(E, "pre-existing");

            // One acquisition, three states, assertions between them — the shape
            // `host::capture`'s unset/blank test needs. Completing at all proves
            // the handle does not re-enter the lock.
            env.set(D, "first");
            assert_eq!(get(D).as_deref(), Some("first"));

            env.set(D, "second");
            assert_eq!(get(D).as_deref(), Some("second"));

            env.remove(E);
            assert_eq!(get(E), None);
        });

        assert_eq!(get(D), None, "D was unset before, so it ends unset");
        assert_eq!(get(E), None, "E's pre-existing value was itself transient");
    }

    #[test]
    fn restores_the_first_value_seen_not_an_intermediate() {
        with_env(|env| {
            env.set(F, "original");

            with_inner(|inner| {
                inner.set(F, "one");
                inner.set(F, "two");
                inner.remove(F);
            });

            assert_eq!(
                get(F).as_deref(),
                Some("original"),
                "restoration must unwind to the first value seen, not the last"
            );
        });
    }

    #[test]
    fn reader_only_section_leaves_the_environment_untouched() {
        with_env(|env| env.set(A, "reader-visible"));
        assert_eq!(get(A), None);

        // The reader-only shape: no mutation, but the lock is still taken.
        let seen = with_env(|_env| get(A));
        assert_eq!(seen, None);
    }

    #[test]
    fn poisoned_lock_does_not_break_later_calls() {
        let unwound = std::panic::catch_unwind(|| {
            with_env(|_env| panic!("poison the lock"));
        });
        assert!(unwound.is_err(), "the closure was supposed to panic");

        // Inherits the behaviour of the former `observability::lock_env_recovers_from_
        // poisoned_mutex`: a poisoned lock must not cascade.
        let ran = with_env(|_env| "still works");
        assert_eq!(ran, "still works");
    }

    /// Runs `f` against a scoped [`Env`] *without* re-acquiring the lock, for the
    /// tests that need a nested restore boundary inside an outer `with_env`.
    fn with_inner<R>(f: impl FnOnce(&mut Env) -> R) -> R {
        let mut env = Env {
            prior: HashMap::new(),
        };
        f(&mut env)
    }
}
