// unwrap/expect are permitted in test code (CONTRIBUTING). clippy's
// allow-{unwrap,expect}-in-tests only exempts `#[test]`/`#[tokio::test]`
// bodies, not the crate's test-helper functions, so this crate-level allow
// covers those helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../helpers/mod.rs"]
mod helpers;

mod projector;
