//! The **tags** vertical's wasm-only UI (ADR-0070): the `TagInput` tag-entry
//! widget — a chip list plus a debounced autocomplete field backed by the
//! [`list_tags`](super::list_tags) endpoint. Declared
//! `#[cfg(target_arch = "wasm32")] mod component;` in `tags/mod.rs`, so this file
//! is wasm-only by its `mod` declaration and carries no cfg gates of its own.

use leptos::prelude::*;

use common::seed::TagSummary;
use common::tag::TagLabel;

/// Chip-based tag input with debounced autocomplete.
///
/// Renders each tag in `tags` as a removable chip and emits one
/// `<input type="hidden" name=name value=display>` per chip so an enclosing
/// form receives a `Vec<String>`.
///
/// Key bindings: `Enter`/`Tab` commit a chip from the text field; `Backspace`
/// on an empty field removes the last chip; `ArrowUp`/`ArrowDown` navigate
/// the autocomplete dropdown; `Escape` closes it.
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
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

    let on_input = move |ev: leptos::ev::Event| {
        use leptos::task::spawn_local;
        use leptos_dom::helpers::set_timeout;
        use std::time::Duration;

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
    };

    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        let key = ev.key();
        match key.as_str() {
            "Enter" | "Tab" => {
                // If a suggestion is keyboard-selected, commit it.
                if let Some(i) = selected_idx.get() {
                    if let Some(tag) = suggestions.get().get(i).cloned() {
                        ev.prevent_default();
                        tags.update(|t| {
                            if !t.iter().any(|x| x.slug == tag.slug) {
                                t.push(tag.clone());
                            }
                        });
                        input_text.set(String::new());
                        error.set(None);
                        suggestions.set(Vec::new());
                        suggestions_open.set(false);
                        selected_idx.set(None);
                        return;
                    }
                }
                // Commit the typed text; Tab passes through if the field is empty.
                if input_text.get().trim().is_empty() {
                    return;
                }
                ev.prevent_default();
                // Validate the raw input via `TagLabel::from_str` (the single
                // validity source, shared with the server) — trims and validates
                // without lowercasing, so the author's casing is preserved
                // (Decision 4). Dedup is on the canonical slug.
                match input_text.get().parse::<TagLabel>() {
                    Ok(label) => {
                        let slug = label.slug();
                        tags.update(|t| {
                            if !t.iter().any(|x| x.slug == slug) {
                                t.push(TagSummary {
                                    slug,
                                    display: label,
                                });
                            }
                        });
                        input_text.set(String::new());
                        error.set(None);
                        suggestions.set(Vec::new());
                        suggestions_open.set(false);
                        selected_idx.set(None);
                    }
                    Err(e) => error.set(Some(e.to_string())),
                }
            }
            "Backspace" if input_text.get().is_empty() => {
                tags.update(|t| {
                    t.pop();
                });
            }
            "ArrowDown" => {
                ev.prevent_default();
                let len = suggestions.get().len();
                if len > 0 {
                    selected_idx.update(|i| {
                        *i = Some(i.map_or(0, |n| (n + 1).min(len - 1)));
                    });
                }
            }
            "ArrowUp" => {
                ev.prevent_default();
                selected_idx.update(|i| {
                    *i = i.and_then(|n| n.checked_sub(1));
                });
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
            {move || {
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
            }}
            <input
                type="text"
                class="j-tag-text"
                placeholder="Add tag\u{2026}"
                prop:value=input_text
                on:input=on_input
                on:keydown=on_keydown
                autocomplete="off"
            />
            {move || {
                if !suggestions_open.get() {
                    return ().into_any();
                }
                let items = suggestions
                    .get()
                    .into_iter()
                    .enumerate()
                    .map(|(idx, tag)| {
                        let is_active = selected_idx.get() == Some(idx);
                        let slug = tag.slug.clone();
                        let display = tag.display.clone();
                        view! {
                            <li
                                class=if is_active {
                                    "j-tag-suggest-item is-active"
                                } else {
                                    "j-tag-suggest-item"
                                }
                                on:click=move |_| {
                                    let slug = slug.clone();
                                    let display = display.clone();
                                    tags.update(|t| {
                                        if !t.iter().any(|x| x.slug == slug) {
                                            t.push(TagSummary {
                                                slug: slug.clone(),
                                                display: display.clone(),
                                            });
                                        }
                                    });
                                    input_text.set(String::new());
                                    error.set(None);
                                    suggestions.set(Vec::new());
                                    suggestions_open.set(false);
                                    selected_idx.set(None);
                                }
                            >
                                "#"
                                {tag.display.to_string()}
                            </li>
                        }
                    })
                    .collect::<Vec<_>>();
                view! { <ul class="j-tag-suggest">{items}</ul> }.into_any()
            }}
        </div>
        {move || error.get().map(|e| view! { <p class="j-tag-error">{e}</p> })}
    }
}
