// unwrap/expect are permitted in test code (CONTRIBUTING). clippy's
// allow-{unwrap,expect}-in-tests only exempts `#[test]`/`#[tokio::test]`
// bodies, not the crate's test-helper functions, so this crate-level allow
// covers those helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../helpers/mod.rs"]
mod helpers;

mod web_account;
mod web_audiences;
mod web_auth;
mod web_backup;
mod web_email;
mod web_media;
mod web_password_reset;
mod web_posts;
mod web_sessions;
mod web_site;
mod web_subscriptions;
mod web_tags;
