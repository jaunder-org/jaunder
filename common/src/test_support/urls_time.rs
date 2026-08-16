// Test scaffolding that deliberately `expect()`s on a fixture parse, so the
// workspace's `expect_used = deny` lint is expected off for this module; `#[expect]`
// self-flags if the scaffolding ever stops using `expect`.
// lint-suppression:allow approved in #294; existing expectation documents intentional test-scaffolding or naming exception
#![expect(clippy::expect_used)]

use crate::etag::ETag;
use crate::root_relative_url::RootRelativeUrl;
use crate::tagged_url::{TaggedUrl, UrlRole};
use crate::time::{PermalinkDate, UtcInstant};

/// Parse `s` into a valid [`TaggedUrl`] under the ascribed role `T` for tests — the
/// single place a test absolute-URL literal is parsed, so a malformed fixture fails
/// loudly and the parse isn't re-spelled at every call site across the workspace.
///
/// Unlike [`compose`](crate::tagged_url::compose), whose output role is free and must
/// therefore be stated, `parse_url`'s role is pinned by the position it is parsed *into*
/// — the struct field or parameter type — so a fixture cannot stand in for the wrong
/// role even when the call site is bare. Where no such position exists, ascribe or
/// turbofish (#875).
///
/// # Panics
///
/// Panics if `s` is not a valid absolute `http(s)` URL.
#[must_use]
pub fn parse_url<T: UrlRole>(s: &str) -> TaggedUrl<T> {
    s.parse().expect("valid test absolute URL")
}

/// Parse `s` into a valid [`RootRelativeUrl`] for tests — the single place a test
/// root-relative-URL literal is parsed, so a malformed fixture fails loudly and the
/// parse isn't re-spelled at every post-DTO fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is not a valid root-relative (`/…`, host-less) URL.
#[must_use]
pub fn parse_root_relative_url(s: &str) -> RootRelativeUrl {
    s.parse().expect("valid test root-relative URL")
}

/// Parse `s` into a valid [`ETag`] for tests — the single place a test `ETag` literal is
/// parsed, so a malformed fixture fails loudly and the quoted form isn't re-spelled at
/// every feed-cache fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is not a valid double-quoted strong `ETag`.
#[must_use]
pub fn parse_etag(s: &str) -> ETag {
    s.parse().expect("valid test ETag")
}

/// Parse `s` into a valid [`UtcInstant`] for tests — the single place a test
/// instant literal is parsed, so a malformed fixture fails loudly and the parse
/// isn't re-spelled at every timeline/post fixture across the workspace.
///
/// # Panics
///
/// Panics if `s` is not a valid RFC3339 instant.
#[must_use]
pub fn parse_utc_instant(s: &str) -> UtcInstant {
    s.parse().expect("valid test UTC instant")
}

/// Build a valid [`PermalinkDate`] for tests from a `(year, month, day)` triple — the
/// single place a test permalink-date literal is assembled, so an impossible fixture
/// fails loudly and the `from_ymd(...).unwrap()` isn't re-spelled at every call site.
///
/// # Panics
///
/// Panics if the triple is not a real calendar date.
#[must_use]
pub fn permalink_date(year: i32, month: u32, day: u32) -> PermalinkDate {
    PermalinkDate::from_ymd(year, month, day).expect("valid test permalink date")
}
