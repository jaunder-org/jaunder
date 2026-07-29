//! The **tags** vertical's wasm-only UI (ADR-0070): the `TagInput` tag-entry
//! widget — a chip list plus a debounced autocomplete field backed by the
//! [`list`](super::list) endpoint. Declared
//! `#[cfg(target_arch = "wasm32")] mod component;` in `tags/mod.rs`, so this file
//! is wasm-only by its `mod` declaration and carries no cfg gates of its own. Its
//! state and dispatch logic live in the host-tested [`super::input_state`]; only the
//! irreducible event wiring stays here, inline in the component, via leptos's own
//! helpers — `web` names no `web_sys` type and carries no `web-sys` dependency (#520).

use leptos::prelude::*;

use common::seed::TagSummary;

use super::input_state::TagInputState;

/// Chip-based tag input with debounced autocomplete.
///
/// Renders each tag in `tags` as a removable chip and emits one
/// `<input type="hidden" name=name value=display>` per chip so an enclosing
/// form receives a `Vec<String>`. Its behavior lives on [`TagInputState`].
#[component]
pub fn TagInput(
    tags: RwSignal<Vec<TagSummary>>,
    #[prop(default = "tags")] name: &'static str,
) -> impl IntoView {
    let state = TagInputState::new(tags);

    view! {
        <div class="j-tag-input">
            <TagChips tags name />
            <input
                type="text"
                class="j-tag-text"
                placeholder="Add tag\u{2026}"
                prop:value=state.input_text
                on:input=move |ev| {
                    if let Some((prefix, tick)) = state.begin_input(&event_target_value(&ev)) {
                        schedule_suggestion_fetch(
                            prefix,
                            tick,
                            state.debounce_tick,
                            state.suggestions,
                            state.suggestions_open,
                        );
                    }
                }
                on:keydown=move |ev| {
                    if state.handle_key(ev.key().as_str()) {
                        ev.prevent_default();
                    }
                }
                autocomplete="off"
            />
            <TagSuggestions
                suggestions=state.suggestions
                suggestions_open=state.suggestions_open
                selected_idx=state.selected_idx
                on_commit=Callback::new(move |tag| state.commit(tag))
            />
        </div>
        {move || state.error.get().map(|e| view! { <p class="j-tag-error">{e}</p> })}
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
/// superseded `tick`, query `list(prefix)` and publish the results (opening
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
                if let Ok(results) = crate::tags::list(Some(prefix), Some(10)).await {
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
