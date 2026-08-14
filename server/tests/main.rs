// The single server integration-test binary: one crate, so `helpers` compiles
// once and one crate-level `#![expect]` covers every subsystem (#298).
//
// unwrap/expect are permitted in test code (CONTRIBUTING); clippy's
// allow-{unwrap,expect}-in-tests only exempts #[test]/#[tokio::test] bodies, not the
// shared/local test-helper fns, so this single crate-level expect covers them.
#![expect(clippy::unwrap_used, clippy::expect_used)]

mod helpers;

mod build_script;

mod atompub;
mod feed;
mod misc;
mod projector;
mod storage;
mod web;
