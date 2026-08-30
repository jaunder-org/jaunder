//! Server-authored revision-history wire values.
//!
//! The rendered HTML field reconstructs inside `common`, so application crates
//! cannot turn arbitrary raw wire strings into [`crate::render::RenderedHtml`].

use serde::{Deserialize, Serialize};

use crate::ids::{AudienceId, PostId, RevisionId};
use crate::post_body::PostBody;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;
use crate::render::{PostFormat, RenderedHtml, deserialize_rendered_html};
use crate::slug::Slug;
use crate::tag::{Tag, TagLabel};
use crate::time::UtcInstant;

/// One normalized tag captured with a revision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionHistoryTag {
    pub tag: Tag,
    pub display: TagLabel,
}

/// One captured audience target; `audience_id` is present only for `named`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionHistoryAudience {
    pub kind: String,
    pub audience_id: Option<AudienceId>,
}

/// The complete immutable scalar revision snapshot.
///
/// Child collections stay explicit so a client cannot mistake current
/// tag/audience/media state for historical state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RevisionHistoryDetail {
    pub revision_id: RevisionId,
    pub post_id: PostId,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub body: PostBody,
    pub format: PostFormat,
    #[serde(deserialize_with = "deserialize_rendered_html")]
    // rendered-html-from-trusted:allow revision DTO rebuilds HTML serialized by Jaunder's own server (#1147)
    pub rendered_html: RenderedHtml,
    pub summary: Option<PostSummary>,
    pub created_at: UtcInstant,
    pub updated_at: UtcInstant,
    pub published_at: Option<UtcInstant>,
    pub deleted_at: Option<UtcInstant>,
    pub captured_at: UtcInstant,
    pub tags: Vec<RevisionHistoryTag>,
    pub audiences: Vec<RevisionHistoryAudience>,
    pub media: Vec<String>,
}
