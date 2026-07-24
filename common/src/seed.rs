//! The projector↔client seed contract (#610, ADR-0041): `PageSeed` — the initial data
//! a public page renders from — and the public-surface wire DTOs it embeds. The server
//! projector serializes `PageSeed` into the `#jaunder-seed` DOM blob; the `csr` client
//! deserializes it on boot for a byte-identical first paint. These are also the return
//! types of the media/post/tag `#[server]` fns. Pure `Serialize`/`Deserialize` data —
//! every field is a `common` type, so this module has no `leptos`/`web_sys`/`storage` coupling.

use serde::{Deserialize, Serialize};

use crate::ids::PostId;
use crate::post_body::PostBody;
use crate::post_summary::PostSummary;
use crate::post_title::PostTitle;
use crate::render::{deserialize_rendered_html, PostFormat, RenderedHtml};
use crate::root_relative_url::RootRelativeUrl;
use crate::slug::Slug;
use crate::tag::{Tag, TagLabel};
use crate::time::UtcInstant;
use crate::username::Username;

/// A tag row returned by [`list_tags`].
///
/// `slug` is the canonical lowercase form used in URLs (`/tags/:slug`).
/// `display` is the case-preserving form the author most recently used; the
/// autocomplete dropdown should render this to the user. When a tag has been
/// applied with multiple casings across posts, `display` reflects whichever
/// row the underlying `SELECT` returned first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagSummary {
    pub slug: Tag,
    pub display: TagLabel,
}

/// A published post row returned by timeline listing endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelinePostSummary {
    pub post_id: PostId,
    pub username: Username,
    pub title: Option<PostTitle>,
    pub summary: Option<PostSummary>,
    pub slug: Slug,
    #[serde(deserialize_with = "deserialize_rendered_html")]
    pub rendered_html: RenderedHtml,
    pub created_at: UtcInstant,
    pub published_at: UtcInstant,
    /// Root-relative permalink of a published post; `None` when the summary is
    /// rebuilt from a draft `PostResponse` (no public permalink), so the title
    /// renders without a link — coinciding with the projector's draft paint.
    pub permalink: Option<RootRelativeUrl>,
    /// True when the viewing user is the post author.
    pub is_author: bool,
    /// True when this post is the author's own unpublished draft. Timeline
    /// listings only ever carry published rows, so this is `false` there; it is
    /// `true` only when `PostPage` renders a draft at its permalink.
    pub is_draft: bool,
    /// Tags applied to this post, ordered by canonical slug.
    pub tags: Vec<TagSummary>,
}

/// A cursor-paginated page of timeline posts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelinePage {
    pub posts: Vec<TimelinePostSummary>,
    pub next_cursor_created_at: Option<UtcInstant>,
    pub next_cursor_post_id: Option<PostId>,
    pub has_more: bool,
}

/// Details of a post returned by [`get_post`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PostResponse {
    pub post_id: PostId,
    pub username: Username,
    pub title: Option<PostTitle>,
    pub slug: Slug,
    pub body: PostBody,
    pub format: PostFormat,
    #[serde(deserialize_with = "deserialize_rendered_html")]
    pub rendered_html: RenderedHtml,
    pub created_at: UtcInstant,
    pub published_at: Option<UtcInstant>,
    pub is_draft: bool,
    pub is_author: bool,
    /// Permalink URL for published posts; `None` for drafts.
    pub permalink: Option<RootRelativeUrl>,
    /// Tags applied to this post, ordered by canonical slug.
    pub tags: Vec<TagSummary>,
    /// Optional summary/excerpt of the post.
    pub summary: Option<PostSummary>,
}

/// The initial data a public page is rendered from — serialized into the
/// projector's `#jaunder-seed` blob and adopted by the CSR client on boot.
///
/// Variants carry the route context (`username` / `tag`) the bare
/// [`TimelinePage`] lacks but the heading, title, and permalinks need — the
/// reactive components get it from the route params today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageSeed {
    SiteTimeline(TimelinePage),
    Profile {
        username: Username,
        page: TimelinePage,
    },
    SiteTag {
        tag: Tag,
        page: TimelinePage,
    },
    UserTag {
        username: Username,
        tag: Tag,
        page: TimelinePage,
    },
    Permalink(PostResponse),
}
