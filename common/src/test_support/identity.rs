// Test scaffolding that deliberately `expect()`s on a fixture parse, so the
// workspace's `expect_used = deny` lint is expected off for this module; `#[expect]`
// self-flags if the scaffolding ever stops using `expect`.
#![expect(clippy::expect_used)]

use crate::audience::AudienceName;
use crate::bio::Bio;
use crate::display_name::DisplayName;
use crate::email::Email;
use crate::password::Password;
use crate::session_label::SessionLabel;
use crate::smtp_password::SmtpPassword;
use crate::smtp_username::SmtpUsername;
use crate::token::{RawToken, TokenHash};
use crate::username::Username;

/// Parse `addr` into a valid [`Email`] for tests — the single place a test email
/// literal is parsed, so a malformed fixture fails loudly and the parse isn't
/// re-spelled at every call site across the workspace.
///
/// # Panics
///
/// Panics if `addr` is not a valid email address.
#[must_use]
pub fn parse_email(addr: &str) -> Email {
    addr.parse().expect("valid test email address")
}

/// Parse `name` into a valid [`AudienceName`] for tests — the single place a test
/// audience-name literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every store-seeding call site across the workspace.
///
/// # Panics
///
/// Panics if `name` is empty or whitespace-only.
#[must_use]
pub fn parse_audience_name(name: &str) -> AudienceName {
    name.parse().expect("valid test audience name")
}

/// Parse `name` into a valid [`DisplayName`] for tests — the single place a test
/// display-name literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every store-seeding call site across the workspace.
///
/// # Panics
///
/// Panics if `name` is empty, whitespace-only, or longer than the length bound.
#[must_use]
pub fn parse_display_name(name: &str) -> DisplayName {
    name.parse().expect("valid test display name")
}

/// Parse `s` into a valid [`Bio`] for tests — the single place a test bio literal is
/// parsed, so a malformed fixture (empty or over the length bound) fails loudly and the
/// validating `FromStr` isn't re-spelled at every profile fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is empty/whitespace-only or longer than the length bound.
#[must_use]
pub fn parse_bio(s: &str) -> Bio {
    s.parse().expect("valid test bio")
}

/// Parse `s` into a valid [`SessionLabel`] for tests — the single place a test
/// session/app-password label literal is parsed, so a malformed fixture (empty or
/// over the length bound) fails loudly and the validating `FromStr` isn't
/// re-spelled at every session/create-session fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is empty/whitespace-only or longer than the length bound.
#[must_use]
pub fn parse_session_label(s: &str) -> SessionLabel {
    s.parse().expect("valid test session label")
}

/// Parse `s` into a [`RawToken`] for tests — the single place a test token literal is
/// constructed, so `RawToken::try_from("…".to_string()).unwrap()` isn't re-spelled at
/// every call site. Takes `&str` (no `.to_string()`), routing through `RawToken`'s
/// validating `FromStr`.
///
/// # Panics
///
/// Panics if `s` is empty or not base64url.
#[must_use]
pub fn parse_raw_token(s: &str) -> RawToken {
    s.parse().expect("valid test raw token")
}

/// Parse `name` into a valid [`Username`] for tests — the single place a test
/// username literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every call site across the workspace.
///
/// # Panics
///
/// Panics if `name` is not a valid username (`[a-z0-9_-]+`).
#[must_use]
pub fn parse_username(name: &str) -> Username {
    name.parse().expect("valid test username")
}

/// Parse `s` into a valid [`TokenHash`] for tests — the single place a test
/// token-hash literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every session-row fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is not a valid token hash.
#[must_use]
pub fn parse_token_hash(s: &str) -> TokenHash {
    s.parse().expect("valid test token hash")
}

/// Parse `s` into a valid [`Password`] for tests — the single place a test password
/// literal is parsed, so a too-short fixture fails loudly and the validating `FromStr`
/// isn't re-spelled at every `create_user`/`verify_password` call site.
///
/// # Panics
///
/// Panics if `s` does not meet the minimum-length requirement.
#[must_use]
pub fn parse_password(s: &str) -> Password {
    s.parse().expect("valid test password")
}

/// Parse `s` into a valid [`SmtpPassword`] for tests — the single place a test
/// SMTP-password literal is parsed, so an empty fixture fails loudly and the
/// validating `FromStr` isn't re-spelled at every `SmtpConfig` fixture.
///
/// # Panics
///
/// Panics if `s` is empty.
#[must_use]
pub fn parse_smtp_password(s: &str) -> SmtpPassword {
    s.parse().expect("valid test SMTP password")
}

/// Parse `s` into a valid [`SmtpUsername`] for tests — the single place a test
/// SMTP-username literal is parsed, so an empty fixture fails loudly.
///
/// # Panics
///
/// Panics if `s` is empty.
#[must_use]
pub fn parse_smtp_username(s: &str) -> SmtpUsername {
    s.parse().expect("valid test SMTP username")
}
