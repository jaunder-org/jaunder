// The single server integration-test binary. The six subsystems below were once
// six separate test crates that each path-cloned a shared `helpers` module into
// themselves; folding them into one crate lets `helpers` compile once and collapses
// six crate-level `#![expect]`s into this one (#298).
//
// unwrap/expect are permitted in test code (CONTRIBUTING); clippy's
// allow-{unwrap,expect}-in-tests only exempts #[test]/#[tokio::test] bodies, not the
// shared/local test-helper fns, so this single crate-level expect covers them.
#![expect(clippy::unwrap_used, clippy::expect_used)]

mod helpers;

mod atompub;
mod feed;
mod misc;
mod projector;
mod storage;
mod web;
