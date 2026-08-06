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
    // ORDER-SENSITIVE, and the `unsafe` in `Env::drop` depends on it: locals drop in
    // reverse declaration order, so `_guard` must be declared FIRST for `env` to be
    // dropped — restoring the environment — while the lock is still held. Swapping
    // these two lines would let the restore race another test.
    let _guard = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    // `Env`'s `Drop` restores, so an unwind out of `f` is covered without
    // `catch_unwind` — and without swallowing the panic.
    let mut env = Env::new();

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
    /// An empty handle, recording nothing yet.
    ///
    /// Private: constructing one outside [`with_env`] would produce a restore
    /// boundary that is *not* protected by `ENV_LOCK`, which is the whole
    /// invariant. The only exception is this module's own tests, which exercise
    /// [`Env::drop`] directly.
    fn new() -> Self {
        Self {
            prior: HashMap::new(),
        }
    }

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

    // A key per behaviour under test, named for it. `with_env` serializes access, but
    // two tests sharing a key would still assert on each other's fixture — so each
    // key below appears in exactly one test.
    const RESTORED: &str = "JAUNDER_TEST_WITH_ENV_RESTORED";
    const PANIC_UNWOUND: &str = "JAUNDER_TEST_WITH_ENV_PANIC_UNWOUND";
    const PANIC_PRIOR_VALUE: &str = "JAUNDER_TEST_WITH_ENV_PANIC_PRIOR_VALUE";
    const PREVIOUSLY_UNSET: &str = "JAUNDER_TEST_WITH_ENV_PREVIOUSLY_UNSET";
    const INTERLEAVED: &str = "JAUNDER_TEST_WITH_ENV_INTERLEAVED";
    const INTERLEAVED_REMOVED: &str = "JAUNDER_TEST_WITH_ENV_INTERLEAVED_REMOVED";
    const FIRST_SEEN: &str = "JAUNDER_TEST_WITH_ENV_FIRST_SEEN";
    const READER_ONLY: &str = "JAUNDER_TEST_WITH_ENV_READER_ONLY";

    /// Reads a variable outside any critical section — for asserting on what
    /// `with_env` left behind.
    fn get(key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    #[test]
    fn restores_prior_value_after_closure() {
        with_env(|env| {
            env.set(RESTORED, "during");
            assert_eq!(get(RESTORED).as_deref(), Some("during"));
        });
        assert_eq!(get(RESTORED), None, "must restore the prior (unset) state");
    }

    /// Restore-on-unwind through the **public** API. The prior state here is
    /// "unset", because `with_env` restores everything it touches — so a durable
    /// pre-existing value cannot be established through the public surface alone.
    /// [`env_drop_restores_a_prior_value_under_unwind`] covers that half.
    #[test]
    fn restores_prior_state_when_closure_panics() {
        assert_eq!(get(PANIC_UNWOUND), None, "fixture key must start unset");

        let unwound = std::panic::catch_unwind(|| {
            with_env(|env| {
                env.set(PANIC_UNWOUND, "during");
                assert_eq!(get(PANIC_UNWOUND).as_deref(), Some("during"));
                panic!("boom");
            });
        });

        assert!(unwound.is_err(), "the closure was supposed to panic");
        assert_eq!(
            get(PANIC_UNWOUND),
            None,
            "unwinding out of with_env must still restore"
        );
    }

    /// The other half of restore-on-unwind: a **non-empty** prior value.
    ///
    /// White-box by necessity — it drives [`Env::drop`] directly, because the only
    /// way to establish a durable prior value is to be inside a `with_env` already,
    /// and nesting deadlocks. Held together with the public test above.
    #[test]
    fn env_drop_restores_a_prior_value_under_unwind() {
        with_env(|env| {
            env.set(PANIC_PRIOR_VALUE, "before");

            let unwound = std::panic::catch_unwind(|| {
                let mut inner = Env::new();
                inner.set(PANIC_PRIOR_VALUE, "during");
                assert_eq!(get(PANIC_PRIOR_VALUE).as_deref(), Some("during"));
                panic!("boom");
            });

            assert!(unwound.is_err(), "the closure was supposed to panic");
            assert_eq!(
                get(PANIC_PRIOR_VALUE).as_deref(),
                Some("before"),
                "unwinding must restore the prior value, not clear it"
            );
        });
    }

    #[test]
    fn removes_variable_that_was_previously_unset() {
        assert_eq!(get(PREVIOUSLY_UNSET), None, "fixture key must start unset");
        with_env(|env| {
            env.set(PREVIOUSLY_UNSET, "transient");
            assert_eq!(get(PREVIOUSLY_UNSET).as_deref(), Some("transient"));
        });
        assert_eq!(
            get(PREVIOUSLY_UNSET),
            None,
            "a previously-unset key must end unset"
        );
    }

    #[test]
    fn supports_interleaved_states_in_one_acquisition() {
        with_env(|env| {
            env.set(INTERLEAVED_REMOVED, "pre-existing");

            // One acquisition, three states, assertions between them — the shape
            // `host::capture`'s unset/blank test needs. Completing at all proves
            // the handle does not re-enter the lock.
            env.set(INTERLEAVED, "first");
            assert_eq!(get(INTERLEAVED).as_deref(), Some("first"));

            env.set(INTERLEAVED, "second");
            assert_eq!(get(INTERLEAVED).as_deref(), Some("second"));

            env.remove(INTERLEAVED_REMOVED);
            assert_eq!(get(INTERLEAVED_REMOVED), None);
        });

        assert_eq!(get(INTERLEAVED), None, "unset before, so unset after");
        assert_eq!(
            get(INTERLEAVED_REMOVED),
            None,
            "its pre-existing value was itself transient"
        );
    }

    #[test]
    fn restores_the_first_value_seen_not_an_intermediate() {
        with_env(|env| {
            env.set(FIRST_SEEN, "original");

            with_inner(|inner| {
                inner.set(FIRST_SEEN, "one");
                inner.set(FIRST_SEEN, "two");
                inner.remove(FIRST_SEEN);
            });

            assert_eq!(
                get(FIRST_SEEN).as_deref(),
                Some("original"),
                "restoration must unwind to the first value seen, not the last"
            );
        });
    }

    #[test]
    fn reader_only_section_leaves_the_environment_untouched() {
        assert_eq!(get(READER_ONLY), None, "fixture key must start unset");

        // The reader-only shape — no mutation, but the lock is still taken. This is
        // what the ~26 converted reader tests in `server` look like.
        let seen = with_env(|_env| get(READER_ONLY));

        assert_eq!(seen, None);
        assert_eq!(get(READER_ONLY), None);
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
    ///
    /// Sound only because every caller is already inside a `with_env`, so the lock
    /// is held for the whole of `f`. Not a pattern for production test code — the
    /// public API is `with_env`.
    fn with_inner<R>(f: impl FnOnce(&mut Env) -> R) -> R {
        let mut env = Env::new();
        f(&mut env)
    }
}
