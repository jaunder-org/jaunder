//! The **posts** vertical (ADR-0070, amended #530).
//!
//! This module is **wiring only**: module declarations and re-exports, no items
//! of its own. The single-post lifecycle `#[server]` endpoints and wire types
//! live in [`api`]; host-only marshalling for the `#[server]` bodies lives in
//! the `server` leaf. The cursor-paginated listing surface is its own vertical
//! (`crate::timeline`). Re-exports keep the stable `crate::posts::…` paths
//! external call sites and the server-fn registrar depend on.

mod api;

#[cfg(feature = "server")]
mod server;

// The `#[component]` UI and browser-bound code — wasm-only by its `mod`
// declaration (ADR-0070), so the file carries no cfg gates of its own.
#[cfg(target_arch = "wasm32")]
mod component;

// The pure post-render twins (host-compiled leaf, ADR-0070): plain-string HTML
// builders shared by the projector (`crate::app::render`) and the reactive
// `PostDisplay`, reachable crate-wide as `crate::posts::render::…`.
pub(crate) mod render;

// Pure, host-tested parsing/formatting logic (ADR-0070 §6, ADR-0055): the
// permalink route-param decoder and the draft-row display computation, extracted
// out of the wasm-only components so they stay host-compiled and coverage-measured.
mod parse;

// The listing pages' and the editor's host-tested decision folds (#306, ADR-0083):
// seed adoption, the route-param guards, the editor's publish redirect / post-id
// short-circuit, and `PostCard`'s parent-callback plumbing, extracted out of the
// wasm-only components so the branches are assertable and coverage-measured.
mod page_state;

// The new-post composer's shared signal bundle and its dispatch payload, extracted
// so the payload is host-tested and so each composer shape can be its own
// `#[component]` over one prop rather than seven (#301, ADR-0070 §6).
mod compose_state;
mod edit_state;

// Re-exported at the (public) `crate::posts::…` path so the pure `parse` fns are
// reachable exported items on the host build too — consumed only by the wasm-only
// `component`, an unexported `parse` fn would fail the host build as `dead_code`.
pub use parse::{DraftRowDisplay, PermalinkRoute, draft_row_display, parse_permalink_route};

// Same reason as `parse` above: `page_state`'s only caller is the wasm-only
// `component`, so without these the host build sees every one of them as `dead_code`.
pub use page_state::{
    ListingRoute, NamedAudienceState, notify, notify_with_fallback, publish_redirect, seeded_page,
    tag_query, user_query, user_tag_query, with_post_id,
};

// Same reason again: the composer and editor state seams are consumed by the
// wasm-only component.
pub use compose_state::{ComposeState, PublicationIntent, publication_from_local, submit_gate};
pub use edit_state::{
    EditPublicationState, InvalidSchedule, LoadedPublication, ScheduledEditState, edit_submit_gate,
    loaded_publication,
};

// The API surface — re-exported so external call sites and the server-fn
// registrar keep the stable `crate::posts::…` paths despite living in `api.rs`.
pub use api::{
    Create, Delete, EditPostPreview, Get, GetAudienceSelection, GetDefaultAudienceSelection,
    GetPreview, ListDrafts, ListScheduled, PostInputs, Publish, SavedPost, Unpublish,
    UnpublishedPost, Update, create, delete, get, get_audience_selection,
    get_default_audience_selection, get_preview, list_drafts, list_scheduled, publish, unpublish,
    update,
};

// Re-exported for the `server` crate's public projector, which maps the fetched
// record the same way this vertical does (one projection, no drift), and for
// `crate::timeline`, whose listing queries project their rows through the same
// summary builder. `authored_post` is a wire-type builder that stays in `web`;
// the projector imports the effectful `fetch_post_record` straight from `storage`.
#[cfg(feature = "server")]
pub use server::{authored_post, rendered_post};

// The wasm-only reactive UI (ADR-0070): the post widgets and the routed page
// components (#323). Re-exported so the `pages/` router keeps
// the stable `crate::posts::…` paths; the private helpers (`marker_matches`,
// `audience_checkbox`, `permalink_first_paint`, the `render_draft_row` builder) and the
// private `DraftList` subcomponent stay unexported.
#[cfg(target_arch = "wasm32")]
pub use component::{
    AudiencePicker, ComposerFields, CreatePostPage, DraftsPage, EditPostPage, InlineComposer,
    PostCard, PostCreateForm, PostDisplay, PostPage, ScheduledPage, SiteTagPage, UserTagPage,
    UserTimelinePage,
};
