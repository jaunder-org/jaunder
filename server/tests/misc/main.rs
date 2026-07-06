// unwrap/expect are permitted in test code (CONTRIBUTING). clippy's
// allow-{unwrap,expect}-in-tests only exempts `#[test]`/`#[tokio::test]`
// bodies, not the crate's test-helper functions, so this crate-level `#![expect]`
// covers those helpers.
#![expect(clippy::unwrap_used, clippy::expect_used)]

#[path = "../helpers/mod.rs"]
mod helpers;

mod backup_fixture;
mod backup_interop;
mod cli_subprocess;
mod commands;
mod media_handlers;
mod pg_teardown;
mod static_assets;
