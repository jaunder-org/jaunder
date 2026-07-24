//! The **tags** vertical's wasm-only UI (ADR-0070): the `TagInput` tag-entry
//! widget — a chip list plus a debounced autocomplete field backed by the
//! [`list_tags`](super::list_tags) endpoint. Declared
//! `#[cfg(target_arch = "wasm32")] mod component;` in `tags/mod.rs`, so this file
//! is wasm-only by its `mod` declaration and carries no cfg gates of its own. The
//! pure state logic lives in the host-tested [`super::input_logic`].

use leptos::prelude::*;

use common::seed::TagSummary;

use super::input_logic::{next_suggestion, parse_committed_tag, prev_suggestion, push_unique};

/// Chip-based tag input with debounced autocomplete.
///
/// Renders each tag in `tags` as a removable chip and emits one
/// `<input type="hidden" name=name value=display>` per chip so an enclosing
/// form receives a `Vec<String>`.
///
/// Key bindings: `Enter`/`Tab` commit a chip from the text field; `Backspace`
/// on an empty field removes the last chip; `ArrowUp`/`ArrowDown` navigate
/// the autocomplete dropdown; `Escape` closes it.
#[component]
pub fn TagInput(
    tags: RwSignal<Vec<TagSummary>>,
    #[prop(default = "tags")] name: &'static str,
) -> impl IntoView {
    let input_text = RwSignal::new(String::new());
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let suggestions: RwSignal<Vec<TagSummary>> = RwSignal::new(Vec::new());
    let suggestions_open = RwSignal::new(false);
    let selected_idx: RwSignal<Option<usize>> = RwSignal::new(None);
    // Tick counter for debounce: increment on each keystroke; the timeout
    // callback only fires if the tick hasn't changed.
    let debounce_tick = RwSignal::new(0u64);

    // The one commit path — used by the keyboard handler and the dropdown click:
    // dedup-append the tag, then clear the field and close the suggestions.
    let commit = Callback::new(move |tag: TagSummary| {
        tags.update(|t| push_unique(t, tag));
        input_text.set(String::new());
        error.set(None);
        suggestions.set(Vec::new());
        suggestions_open.set(false);
        selected_idx.set(None);
    });

    let on_input = move |ev: leptos::ev::Event| {
        let val = event_target_value(&ev);
        input_text.set(val.clone());
        error.set(None);
        selected_idx.set(None);

        let prefix = val.trim().to_lowercase();
        if prefix.is_empty() {
            suggestions.set(Vec::new());
            suggestions_open.set(false);
            return;
        }

        let tick = debounce_tick.get_untracked() + 1;
        debounce_tick.set(tick);
        schedule_suggestion_fetch(prefix, tick, debounce_tick, suggestions, suggestions_open);
    };

    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        match ev.key().as_str() {
            "Enter" | "Tab" => {
                // A keyboard-selected suggestion commits directly.
                if let Some(i) = selected_idx.get() {
                    if let Some(tag) = suggestions.get().get(i).cloned() {
                        ev.prevent_default();
                        commit.run(tag);
                        return;
                    }
                }
                // Otherwise commit the typed text; Tab passes through if empty.
                if input_text.get().trim().is_empty() {
                    return;
                }
                ev.prevent_default();
                match parse_committed_tag(&input_text.get()) {
                    Ok(tag) => commit.run(tag),
                    Err(e) => error.set(Some(e)),
                }
            }
            "Backspace" if input_text.get().is_empty() => {
                tags.update(|t| {
                    t.pop();
                });
            }
            "ArrowDown" => {
                ev.prevent_default();
                selected_idx.update(|i| *i = next_suggestion(*i, suggestions.get().len()));
            }
            "ArrowUp" => {
                ev.prevent_default();
                selected_idx.update(|i| *i = prev_suggestion(*i));
            }
            "Escape" => {
                suggestions.set(Vec::new());
                suggestions_open.set(false);
                selected_idx.set(None);
            }
            _ => {}
        }
    };

    view! {
        <div class="j-tag-input">
            <TagChips tags name />
            <input
                type="text"
                class="j-tag-text"
                placeholder="Add tag\u{2026}"
                prop:value=input_text
                on:input=on_input
                on:keydown=on_keydown
                autocomplete="off"
            />
            <TagSuggestions suggestions suggestions_open selected_idx on_commit=commit />
        </div>
        {move || error.get().map(|e| view! { <p class="j-tag-error">{e}</p> })}
    }
}

/// The committed-tag chips: one removable `#tag` chip per entry, each emitting a
/// hidden `<input name=… value=display>` so an enclosing form receives the tags.
#[component]
fn TagChips(tags: RwSignal<Vec<TagSummary>>, name: &'static str) -> impl IntoView {
    move || {
        tags.get()
            .into_iter()
            .map(|tag| {
                let slug = tag.slug.clone();
                let display = tag.display.to_string();
                view! {
                    <span class="j-tag-chip">
                        <input type="hidden" name=name value=display.clone() />
                        <span class="j-tag-chip-label">"#" {display}</span>
                        <button
                            type="button"
                            class="j-tag-chip-remove"
                            aria-label="Remove tag"
                            on:click=move |_| {
                                tags.update(|t| t.retain(|x| x.slug != slug));
                            }
                        >
                            "\u{00d7}"
                        </button>
                    </span>
                }
            })
            .collect::<Vec<_>>()
    }
}

/// The autocomplete dropdown: shown while open, one clickable `#tag` row per
/// suggestion with the keyboard-selected row highlighted. Clicking a row commits
/// it through `on_commit` (the same path as the keyboard handler).
#[component]
fn TagSuggestions(
    suggestions: RwSignal<Vec<TagSummary>>,
    suggestions_open: RwSignal<bool>,
    selected_idx: RwSignal<Option<usize>>,
    on_commit: Callback<TagSummary>,
) -> impl IntoView {
    move || {
        if !suggestions_open.get() {
            return ().into_any();
        }
        let items = suggestions
            .get()
            .into_iter()
            .enumerate()
            .map(|(idx, tag)| {
                let is_active = selected_idx.get() == Some(idx);
                let committed = tag.clone();
                view! {
                    <li
                        class=if is_active {
                            "j-tag-suggest-item is-active"
                        } else {
                            "j-tag-suggest-item"
                        }
                        on:click=move |_| on_commit.run(committed.clone())
                    >
                        "#"
                        {tag.display.to_string()}
                    </li>
                }
            })
            .collect::<Vec<_>>();
        view! { <ul class="j-tag-suggest">{items}</ul> }.into_any()
    }
}

/// Debounced autocomplete fetch: after 150 ms, if no later keystroke has
/// superseded `tick`, query `list_tags(prefix)` and publish the results (opening
/// the dropdown when non-empty). Later keystrokes bump `debounce_tick`, so a stale
/// timer both skips its fetch and discards a fetch that returns late.
fn schedule_suggestion_fetch(
    prefix: String,
    tick: u64,
    debounce_tick: RwSignal<u64>,
    suggestions: RwSignal<Vec<TagSummary>>,
    suggestions_open: RwSignal<bool>,
) {
    use leptos::task::spawn_local;
    use leptos_dom::helpers::set_timeout;
    use std::time::Duration;

    set_timeout(
        move || {
            if debounce_tick.get_untracked() != tick {
                return;
            }
            spawn_local(async move {
                if let Ok(results) = crate::tags::list_tags(Some(prefix), Some(10)).await {
                    if debounce_tick.get_untracked() == tick {
                        let open = !results.is_empty();
                        suggestions.set(results);
                        suggestions_open.set(open);
                    }
                }
            });
        },
        Duration::from_millis(150),
    );
}
