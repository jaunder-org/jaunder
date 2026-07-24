//! Tag autocomplete: the `/list_tags` endpoint + its `TagSummary` wire DTO, and
//! the `TagInput` tag-entry widget.
mod api;

// Pure, host-tested input logic (ADR-0070 §6): dedup, keyboard-nav, and typed-tag
// parsing extracted out of the wasm-only component so they stay host-compiled and
// coverage-measured.
mod input_logic;

#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{list_tags, ListTags, DEFAULT_TAG_LIMIT, MAX_TAG_LIMIT};

// Re-exported at the (public) `crate::tags::…` path so the pure `input_logic` fns
// are reachable exported items on the host build too — consumed only by the
// wasm-only `component`, an unexported fn would fail the host build as `dead_code`.
pub use input_logic::{next_suggestion, parse_committed_tag, prev_suggestion, push_unique};

#[cfg(target_arch = "wasm32")]
pub use component::TagInput;
