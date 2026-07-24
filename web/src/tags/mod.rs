//! Tag autocomplete: the `/list_tags` endpoint + its `TagSummary` wire DTO.
mod api;

pub use api::{list_tags, ListTags, DEFAULT_TAG_LIMIT, MAX_TAG_LIMIT};
