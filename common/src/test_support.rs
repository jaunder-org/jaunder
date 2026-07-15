//! Cross-crate test fixtures for `common`'s domain types, gated by the
//! `test-support` feature (mirroring `storage::test_support`, ADR-0033): `common`'s
//! own tests reach it under `cfg(test)`; `storage`, `server`, and `web` reach it via
//! the `test-support` feature. Kept out of shipped binaries.

// Test scaffolding that deliberately `expect()`s on a fixture parse, so the
// workspace's `expect_used = deny` lint is expected off for this module; `#[expect]`
// self-flags if the scaffolding ever stops using `expect`.
#![expect(clippy::expect_used)]

use crate::audience::AudienceName;
use crate::display_name::DisplayName;
use crate::email::Email;

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
