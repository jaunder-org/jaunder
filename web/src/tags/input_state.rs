//! The reactive state of the `TagInput` widget, extracted from the wasm-only
//! `component` so its dispatch logic is **host-tested under an `Owner`** rather
//! than left to e2e alone (ADR-0070 §6; the same convention
//! `web::reactive::Invalidator` and `forms::Field` follow). Extraction is now the
//! only way this logic gets covered: since #520 there is no `#[component]`
//! coverage exemption, because a wasm-only component never host-compiles at all.
//! Only the irreducible event wiring stays inline in the component, via leptos's
//! own helpers.

use leptos::prelude::*;

use common::seed::TagSummary;

use super::input_logic::{next_suggestion, parse_committed_tag, prev_suggestion, push_unique};

/// The committed `tags` plus the transient text-field / autocomplete signals.
///
/// Every field is an `RwSignal` (a `Copy` handle into the reactive runtime), so the
/// whole struct is `Copy` and can be handed to each event closure and child callback
/// without per-signal capture. `pub` (and re-exported from `tags`) only so the
/// wasm-only `component` — which never host-compiles — doesn't leave these host-lib
/// items looking like dead code.
#[derive(Clone, Copy)]
pub struct TagInputState {
    pub tags: RwSignal<Vec<TagSummary>>,
    pub input_text: RwSignal<String>,
    pub error: RwSignal<Option<String>>,
    pub suggestions: RwSignal<Vec<TagSummary>>,
    pub suggestions_open: RwSignal<bool>,
    pub selected_idx: RwSignal<Option<usize>>,
    pub debounce_tick: RwSignal<u64>,
}

impl TagInputState {
    /// Build the transient signals around the caller-owned `tags` signal.
    #[must_use]
    pub fn new(tags: RwSignal<Vec<TagSummary>>) -> Self {
        Self {
            tags,
            input_text: RwSignal::new(String::new()),
            error: RwSignal::new(None),
            suggestions: RwSignal::new(Vec::new()),
            suggestions_open: RwSignal::new(false),
            selected_idx: RwSignal::new(None),
            debounce_tick: RwSignal::new(0),
        }
    }

    /// Dismiss the autocomplete dropdown and clear its selection.
    pub fn close_suggestions(self) {
        self.suggestions.set(Vec::new());
        self.suggestions_open.set(false);
        self.selected_idx.set(None);
    }

    /// The one commit path — shared by the keyboard handler and the dropdown click:
    /// dedup-append `tag`, then clear the field and close the suggestions.
    pub fn commit(self, tag: TagSummary) {
        self.tags.update(|t| push_unique(t, tag));
        self.input_text.set(String::new());
        self.error.set(None);
        self.close_suggestions();
    }

    /// Apply a text-field change: mirror the value and reset the error/selection.
    /// Returns `Some((prefix, tick))` when a debounced suggestion fetch should be
    /// scheduled for a non-empty prefix, or `None` when the field is now empty.
    #[must_use]
    pub fn begin_input(self, value: &str) -> Option<(String, u64)> {
        self.input_text.set(value.to_owned());
        self.error.set(None);
        self.selected_idx.set(None);

        let prefix = value.trim().to_lowercase();
        if prefix.is_empty() {
            self.suggestions.set(Vec::new());
            self.suggestions_open.set(false);
            return None;
        }

        let tick = self.debounce_tick.get_untracked() + 1;
        self.debounce_tick.set(tick);
        Some((prefix, tick))
    }

    /// Apply a keydown by its `key` string, returning whether the caller should
    /// `prevent_default` the browser event.
    ///
    /// `Enter`/`Tab` commit a keyboard-selected suggestion, else the typed text
    /// (Tab passes through — no `prevent_default` — when the field is empty);
    /// `Backspace` on an empty field removes the last chip; `ArrowUp`/`ArrowDown`
    /// move the selection; `Escape` closes the dropdown.
    #[must_use]
    pub fn handle_key(self, key: &str) -> bool {
        match key {
            "Enter" | "Tab" => {
                // A keyboard-selected suggestion commits directly.
                if let Some(i) = self.selected_idx.get()
                    && let Some(tag) = self.suggestions.get().get(i).cloned()
                {
                    self.commit(tag);
                    return true;
                }
                // Otherwise commit the typed text; Tab passes through if empty.
                if self.input_text.get().trim().is_empty() {
                    return false;
                }
                match parse_committed_tag(&self.input_text.get()) {
                    Ok(tag) => self.commit(tag),
                    Err(e) => self.error.set(Some(e)),
                }
                true
            }
            "Backspace" if self.input_text.get().is_empty() => {
                self.tags.update(|t| {
                    t.pop();
                });
                false
            }
            "ArrowDown" => {
                self.selected_idx
                    .update(|i| *i = next_suggestion(*i, self.suggestions.get().len()));
                true
            }
            "ArrowUp" => {
                self.selected_idx.update(|i| *i = prev_suggestion(*i));
                true
            }
            "Escape" => {
                self.close_suggestions();
                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TagInputState;
    use common::seed::TagSummary;
    use leptos::prelude::*;

    fn summary(display: &str) -> TagSummary {
        let label: common::tag::TagLabel = display.parse().unwrap();
        TagSummary {
            slug: label.slug(),
            display: label,
        }
    }

    /// Run `body` under a fresh reactive `Owner` (the `web::reactive`/`forms::Field`
    /// convention), so `RwSignal`s work host-side without a browser.
    fn with_owner(body: impl FnOnce()) {
        let owner = Owner::new();
        owner.set();
        body();
        drop(owner);
    }

    #[test]
    fn arrow_keys_move_and_clamp_the_selection() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));
            state
                .suggestions
                .set(vec![summary("rust"), summary("leptos")]);

            // Every arrow keydown prevents default, including at both clamp
            // boundaries — asserted at each step rather than discarded, so the
            // return value is pinned wherever it is produced.
            assert!(state.handle_key("ArrowDown"), "ArrowDown prevents default");
            assert_eq!(state.selected_idx.get(), Some(0));
            assert!(state.handle_key("ArrowDown"), "ArrowDown prevents default");
            assert_eq!(state.selected_idx.get(), Some(1));
            assert!(
                state.handle_key("ArrowDown"),
                "a clamped ArrowDown still prevents default"
            );
            assert_eq!(state.selected_idx.get(), Some(1), "clamps at the last row");
            assert!(state.handle_key("ArrowUp"), "ArrowUp prevents default");
            assert_eq!(state.selected_idx.get(), Some(0));
            assert!(
                state.handle_key("ArrowUp"),
                "ArrowUp past the first row still prevents default"
            );
            assert_eq!(state.selected_idx.get(), None, "clears past the first row");
        });
    }

    #[test]
    fn enter_commits_the_selected_suggestion_and_clears_the_field() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));
            state.suggestions.set(vec![summary("rust")]);
            state.selected_idx.set(Some(0));
            state.input_text.set("ru".to_owned());

            assert!(state.handle_key("Enter"));
            assert_eq!(state.tags.get(), vec![summary("rust")]);
            assert_eq!(state.input_text.get(), "");
            assert!(!state.suggestions_open.get());
        });
    }

    #[test]
    fn enter_commits_typed_text_preserving_casing() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));
            state.input_text.set("Leptos".to_owned());

            assert!(state.handle_key("Enter"));
            assert_eq!(state.tags.get(), vec![summary("Leptos")]);
        });
    }

    #[test]
    fn enter_falls_through_to_typed_text_when_the_selection_is_stale() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));
            // The selection index points past the (empty) suggestion list, so the
            // keyboard-commit path falls through to committing the typed text.
            state.selected_idx.set(Some(0));
            state.input_text.set("Rust".to_owned());

            assert!(state.handle_key("Enter"));
            assert_eq!(state.tags.get(), vec![summary("Rust")]);
        });
    }

    #[test]
    fn enter_on_empty_field_passes_through_without_committing() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));
            assert!(!state.handle_key("Tab"), "Tab passes through when empty");
            assert!(state.tags.get().is_empty());
        });
    }

    #[test]
    fn enter_on_invalid_text_sets_an_error_and_keeps_the_tags() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));
            state.input_text.set("bad tag".to_owned());

            assert!(state.handle_key("Enter"), "still prevents default");
            assert!(state.error.get().is_some());
            assert!(state.tags.get().is_empty());
        });
    }

    #[test]
    fn backspace_on_empty_field_removes_the_last_chip() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(vec![summary("a"), summary("b")]));
            assert!(!state.handle_key("Backspace"));
            assert_eq!(state.tags.get(), vec![summary("a")]);
        });
    }

    #[test]
    fn backspace_with_text_present_is_ignored() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(vec![summary("a")]));
            state.input_text.set("x".to_owned());
            assert!(!state.handle_key("Backspace"));
            assert_eq!(state.tags.get(), vec![summary("a")], "chip kept");
        });
    }

    #[test]
    fn escape_closes_the_dropdown() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));
            state.suggestions.set(vec![summary("rust")]);
            state.suggestions_open.set(true);
            state.selected_idx.set(Some(0));

            assert!(!state.handle_key("Escape"));
            assert!(!state.suggestions_open.get());
            assert_eq!(state.selected_idx.get(), None);
        });
    }

    #[test]
    fn unhandled_key_is_a_no_op() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));
            state.input_text.set("x".to_owned());
            assert!(!state.handle_key("z"));
            assert_eq!(state.input_text.get(), "x", "untouched");
        });
    }

    #[test]
    fn begin_input_schedules_for_a_prefix_and_bumps_the_tick() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));

            let first = state.begin_input("Ru");
            assert_eq!(
                first,
                Some(("ru".to_owned(), 1)),
                "lowercased prefix, tick 1"
            );
            assert_eq!(state.input_text.get(), "Ru", "mirrors the raw value");

            let second = state.begin_input("Rus");
            assert_eq!(second, Some(("rus".to_owned(), 2)), "tick increments");
        });
    }

    #[test]
    fn begin_input_on_empty_clears_suggestions_and_schedules_nothing() {
        with_owner(|| {
            let state = TagInputState::new(RwSignal::new(Vec::new()));
            state.suggestions.set(vec![summary("rust")]);
            state.suggestions_open.set(true);

            assert_eq!(state.begin_input("   "), None);
            assert!(!state.suggestions_open.get());
            assert!(state.suggestions.get().is_empty());
        });
    }
}
