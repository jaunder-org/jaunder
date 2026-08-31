use host::feed::FeedPath;

/// Parses `s` into the canonical [`FeedPath`] identity key. The one shared
/// feed-path constructor for both the `storage` crate's tests and `server`'s
/// integration tests, so the `"…".parse().expect(…)` shape lives in one place.
///
/// # Panics
///
/// If `s` is not a valid canonical feed path.
#[must_use]
pub fn fp(s: &str) -> FeedPath {
    s.parse().expect("valid feed path")
}
