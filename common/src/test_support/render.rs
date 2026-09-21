use crate::render::{RenderedHtml, RenderedPostTitle};

/// Build exact rendered HTML for a test fixture without sanitizing or rewriting it.
///
/// Test data often needs persisted or wire-format bytes that intentionally differ from
/// sanitizer output. This helper is the only cross-crate fixture door for those bytes;
/// it is available only when `common` is built for tests or with `test-support`.
#[must_use]
pub fn rendered_html(html: &str) -> RenderedHtml {
    RenderedHtml(html.to_owned())
}

/// Build an exact trusted Rendered Title for a test fixture without sanitizing.
///
/// This fixture mirrors server-authored DTO reconstruction and is available only
/// to test builds; production title construction remains ammonia-owned.
#[must_use]
pub fn rendered_post_title(html: &str) -> RenderedPostTitle {
    RenderedPostTitle::fixture(html)
}
