//! Tag autocomplete: the `/list_tags` endpoint + its `TagSummary` wire DTO.
mod api;

pub use api::{list_tags, ListTags, DEFAULT_TAG_LIMIT, MAX_TAG_LIMIT};

// Transitional re-export: `TagSummary` moved to `common::seed` (#610); consumers
// still reach it at `crate::tags::TagSummary` until Task 2 repoints them.
pub use common::seed::TagSummary;
