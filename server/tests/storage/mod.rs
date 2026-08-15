use chrono::Utc;
use common::test_support::{parse_bio, parse_email, parse_raw_token};

use storage::{
    ConfirmPasswordResetError, ProfileUpdate, UseEmailVerificationError, UsePasswordResetError,
    UserAuthError,
};

use rstest::*;
// `#[template]`/`#[apply]` come from the `rstest_reuse` companion crate; the
// glob alone is not enough
// (docs/adr/0124-rstest-reuse-cross-module-templates.md).
use rstest_reuse::*;

use crate::helpers::create_session_for;
use storage::test_support::{Backend, SeedUser, backends};

mod audiences;
mod database;
mod email_verification;
mod feed_events;
mod fixtures;
mod fk_constraints;
mod invites;
mod listing;
mod lookups;
mod media;
mod password_reset;
mod posts;
mod resolution;
mod sessions;
mod site_config;
mod subscriptions;
mod tags;
mod user_config;
mod users_auth;

use fixtures::{password, raw_exec};
