#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::unused_async
)]
#![allow(dead_code)]
#![allow(unused_macros)]

// This crate only *defines* the templates, so it needs just the `template`
// attribute. `#[export]` is consumed by `#[template]` (no import needed), and the
// `rstest`/`case` attributes the expansion emits are resolved at the *apply* site
// in consumer crates, not here.
use rstest_reuse::template;

/// The storage backend a test runs against. Backend-parametrized tests take a
/// `#[case] backend: Backend` and call [`Backend::setup`].
#[derive(Copy, Clone)]
pub enum Backend {
    Sqlite,
    Postgres,
}

// `#[export]` adds `#[macro_export]` to the generated template macro so it is
// reachable at this crate's root and `#[apply]`-able from *other* crates
// (`storage` tests, `server` tests). Without it the macro is `pub(crate)` and a
// cross-crate `use db_test_harness::backends` fails with "private macro".
#[template]
#[export]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
pub fn sqlite_only(#[case] backend: Backend) {}

#[template]
#[export]
#[rstest]
#[case::postgres(Backend::Postgres)]
pub fn postgres_only(#[case] backend: Backend) {}

#[template]
#[export]
#[rstest]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
pub fn backends(#[case] backend: Backend) {}
