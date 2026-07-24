//! Tag autocomplete: the `/list_tags` endpoint + its `TagSummary` wire DTO, and
//! the `TagInput` tag-entry widget.
mod api;

#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{list_tags, ListTags, DEFAULT_TAG_LIMIT, MAX_TAG_LIMIT};

#[cfg(target_arch = "wasm32")]
pub use component::TagInput;
