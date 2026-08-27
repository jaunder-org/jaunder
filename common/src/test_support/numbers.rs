// Test scaffolding that deliberately `expect()`s on a fixture parse, so the
// workspace's `expect_used = deny` lint is expected off for this module; `#[expect]`
// self-flags if the scaffolding ever stops using `expect`.
// lint-suppression:allow approved in #294; existing expectation documents intentional test-scaffolding or naming exception
#![expect(clippy::expect_used)]

use crate::backup::{DestinationPath, RetentionCount};
use crate::invite::InviteTtlHours;
use crate::pagination::{PageOffset, PageSize, RowLimit};

/// Parse `s` into a valid [`RetentionCount`] for tests — the single place a test
/// retention-count literal is parsed, so a malformed fixture (e.g. `"0"`) fails loudly
/// and the parse isn't re-spelled at every `BackupConfig` construction site.
///
/// # Panics
///
/// Panics if `s` is not a whole number of at least 1.
#[must_use]
pub fn parse_retention_count(s: &str) -> RetentionCount {
    s.parse().expect("valid test retention count")
}

/// Parse `s` into an [`InviteTtlHours`] for tests — the single place a test invite-TTL literal
/// is parsed, so a malformed fixture fails loudly.
///
/// # Panics
///
/// Panics if `s` is not an integer in `1..=336`.
#[must_use]
pub fn parse_invite_ttl_hours(s: &str) -> InviteTtlHours {
    s.parse().expect("valid test invite TTL")
}

/// Parse `s` into a valid [`DestinationPath`] for tests — the single place a test backup
/// destination literal is parsed, so a malformed fixture (empty/whitespace) fails loudly
/// and the parse isn't re-spelled at every `BackupConfig` construction site.
///
/// # Panics
///
/// Panics if `s` is empty or whitespace-only.
#[must_use]
pub fn parse_destination_path(s: &str) -> DestinationPath {
    s.parse().expect("valid test destination path")
}

/// Parse `s` into a [`PageSize`] for tests — the single place a test page-size literal is
/// parsed, so a malformed fixture (e.g. `"0"`/`"51"`) fails loudly and the parse isn't
/// re-spelled at every pagination call site.
///
/// # Panics
///
/// Panics if `s` is not an integer in `1..=50`.
#[must_use]
pub fn parse_page_size(s: &str) -> PageSize {
    s.parse().expect("valid test page size")
}

/// Parse `s` into a [`PageOffset`] for tests — the single place a test offset literal is
/// parsed, so a malformed fixture fails loudly and the parse isn't re-spelled at every
/// media-listing call site.
///
/// # Panics
///
/// Panics if `s` is not a `u32` (non-integer or negative).
#[must_use]
pub fn parse_page_offset(s: &str) -> PageOffset {
    s.parse().expect("valid test page offset")
}

/// Parse `s` into a [`RowLimit`] for tests — the single place a test row-limit literal is
/// parsed, so a malformed fixture (e.g. `"0"`) fails loudly and the parse isn't re-spelled
/// at every storage listing call site.
///
/// # Panics
///
/// Panics if `s` is not a whole number of at least 1.
#[must_use]
pub fn parse_row_limit(s: &str) -> RowLimit {
    s.parse().expect("valid test row limit")
}
