//! Wasm-only Post UI assembled by routed surface.
//!
//! Pure state, parsing, and rendering stay in the host-compiled sibling leaves
//! owned by the `posts` vertical. These private leaves contain only browser UI.

mod audience;
mod composers;
mod display;
mod drafts;
mod listings;
mod permalink_editor;
mod support;

pub use audience::AudiencePicker;
pub use composers::{ComposerFields, CreatePostPage, InlineComposer, PostCreateForm};
pub use display::{PostCard, PostDisplay};
pub use drafts::{DraftsPage, ScheduledPage};
pub use listings::{SiteTagPage, UserTagPage, UserTimelinePage};
pub use permalink_editor::{EditPostPage, PostPage};
