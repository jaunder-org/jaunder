//! Tag autocomplete: the `/list_tags` endpoint + its `TagSummary` wire DTO, and
//! the `TagInput` tag-entry widget.
mod api;

// Pure, host-tested input logic (ADR-0070 §6): dedup, keyboard-nav, and typed-tag
// parsing extracted out of the wasm-only component so they stay host-compiled and
// coverage-measured.
mod input_logic;

// The widget's reactive state + dispatch, host-tested under an `Owner` (ADR-0070 §6):
// the keyboard/input logic lives here, covered, rather than exempt in the component.
mod input_state;

#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{list, List, DEFAULT_TAG_LIMIT, MAX_TAG_LIMIT};

// Re-exported at the (public) `crate::tags::…` path so the host-lib items consumed
// only by the wasm-only `component` (which never host-compiles) don't look like
// `dead_code` on the host build.
pub use input_logic::{next_suggestion, parse_committed_tag, prev_suggestion, push_unique};
pub use input_state::TagInputState;

#[cfg(target_arch = "wasm32")]
pub use component::TagInput;
