//! Current rights data projected with Posts for public web presentation.

use std::ops::Deref;

use common::content_license::ContentLicense;

use crate::posts::models::PostRecord;

/// A public-presentation Post projection with the author's current Content License.
///
/// This keeps current publication rights out of [`PostRecord`], preserving the
/// source-oriented `AtomPub` Collection and Member representations.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct PublicPresentationPostRecord {
    #[sqlx(flatten)]
    pub post: PostRecord,
    pub content_license: ContentLicense,
}

impl Deref for PublicPresentationPostRecord {
    type Target = PostRecord;

    fn deref(&self) -> &Self::Target {
        &self.post
    }
}

/// Shared columns for public Post projections.
///
/// The correlated lookup preserves each Post's author association and defaults
/// only an absent configuration row. An explicit invalid stored token reaches
/// [`ContentLicense`]'s `SQLx` decoder and fails the read.
pub(crate) const CONTENT_LICENSE_COLUMN: &str = "COALESCE((SELECT value FROM user_config WHERE user_id = p.user_id AND key = 'content.license'), 'all-rights-reserved') AS content_license";
