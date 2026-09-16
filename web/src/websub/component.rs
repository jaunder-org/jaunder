//! Wasm-only operator `WebSub` settings and recovery UI.

use crate::error::WebError;
use crate::forms::{self, Field, ValidatedBareInput};
use crate::mutation_feedback::{self, MutationFeedback};
use crate::topbar::Topbar;
use crate::websub::{
    self, DeadLetterCursor, DeadLetterPage, DeadLetterRow, RedriveDeadLetters, UpdateWebsubHub,
    WebsubPhase,
};
use common::MutationOutcome;
use common::ids::FeedEventId;
use common::pagination::PageSize;
use common::tagged_url::HubUrl;
use leptos::prelude::*;

#[component]
pub fn WebsubPage() -> impl IntoView {
    let update = ServerAction::<UpdateWebsubHub>::new();
    let settings = Resource::new(
        move || update.version().get(),
        |_| websub::get_websub_settings(),
    );
    view! {
        <Topbar title="WebSub" sub="Operations" />
        <div class="j-scroll">
            <div class="j-settings">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match settings.await {
                            Ok(settings) => {
                                hub_form(settings.hub_url.as_deref().unwrap_or_default(), update)
                                    .into_any()
                            }
                            Err(error) => {
                                view! { <p class="error">{error.to_string()}</p> }.into_any()
                            }
                        }
                    })}
                </Suspense>
                {move || mutation_flash(update.value().get(), "WebSub hub saved.")}
                <div class="j-card j-websub-recovery-guidance" data-test="websub-recovery-guidance">
                    <p>
                        "Dead-lettered work is Syndication Feed work whose automatic processing stopped after retries were exhausted or publication encountered a terminal, non-retryable failure."
                    </p>
                    <p>"Dead-letter rows are retained for seven days."</p>
                </div>
                <DeadLetterList phase=WebsubPhase::Regeneration />
                <DeadLetterList phase=WebsubPhase::Publication />
            </div>
        </div>
    }
}

fn hub_form(initial: &str, action: ServerAction<UpdateWebsubHub>) -> impl IntoView {
    let hub = Field::<HubUrl>::optional_prefilled(initial);
    let submit = move |_| {
        action.dispatch(UpdateWebsubHub {
            hub_url: hub.parsed(),
        });
    };
    view! {
        <div class="j-card">
            <div class="j-card-head">
                <div>
                    <h2>"WebSub hub"</h2>
                    <div class="j-sub">"Configure the hub used to announce regenerated feeds."</div>
                </div>
            </div>
            <div class="j-form-body">
                <label class="j-form-field">
                    <span class="j-form-label">"Hub URL"</span>
                    <ValidatedBareInput<HubUrl>
                        name="hub_url"
                        field=hub
                        placeholder=Some("https://hub.example/")
                        class=Some("j-form-input")
                    />
                </label>
                {forms::validated_error(
                    hub.error(),
                    Signal::derive(move || hub.is_touched()),
                    |message| view! { <p class="error">{message}</p> }.into_any(),
                )}
            </div>
            <div class="j-form-actions">
                <button
                    type="button"
                    class="j-btn is-primary"
                    prop:disabled=move || !hub.is_valid()
                    on:click=submit
                >
                    "Save WebSub Hub"
                </button>
            </div>
        </div>
    }
}

#[component]
fn DeadLetterList(phase: WebsubPhase) -> impl IntoView {
    let (phase_key, heading, subtitle, empty_message) = match phase {
        WebsubPhase::Regeneration => (
            "regeneration",
            "Regeneration dead letters",
            "Feed changes that could not rebuild their cached public Syndication Feed representation. Review the diagnostic and correct storage, site identity, or configuration problems before redriving.",
            "No regeneration work is currently dead-lettered.",
        ),
        WebsubPhase::Publication => (
            "publication",
            "Publication dead letters",
            "Feed changes that could not send a WebSub Publish Ping after regeneration. Review the diagnostic and correct the WebSub Hub, network, HTTP, or redirect problem before redriving.",
            "No publication work is currently dead-lettered.",
        ),
    };
    let cursor = RwSignal::new(None::<DeadLetterCursor>);
    let selection = RwSignal::new(Vec::new());
    let redrive = ServerAction::<RedriveDeadLetters>::new();
    Effect::new(move |_| {
        if let Some(Ok(MutationOutcome::Confirmed(()))) = redrive.value().get() {
            selection.set(Vec::new());
            cursor.set(None);
        }
    });
    let page = Resource::new(
        move || (cursor.get(), redrive.version().get()),
        move |(cursor, _)| websub::list_dead_letters(phase, cursor, PageSize::default()),
    );
    let submit = move |_| {
        redrive.dispatch(RedriveDeadLetters {
            ids: selection.get(),
        });
    };
    view! {
        <div class="j-card" data-phase=phase_key>
            <div class="j-card-head">
                <div>
                    <h2>{heading}</h2>
                    <div class="j-sub">{subtitle}</div>
                </div>
            </div>
            <Suspense fallback=|| {
                view! { <p class="j-loading">"Loading\u{2026}"</p> }
            }>
                {move || Suspend::new(async move {
                    match page.await {
                        Ok(page) => {
                            dead_letter_table(page, selection, cursor, empty_message).into_any()
                        }
                        Err(error) => view! { <p class="error">{error.to_string()}</p> }.into_any(),
                    }
                })}
            </Suspense>
            <p class="j-websub-redrive-guidance">
                "Address the reported cause, then select rows to redrive. If any selected row is stale or no longer dead-lettered, none are redriven."
            </p>
            <div class="j-form-actions">
                <button
                    type="button"
                    class="j-btn is-primary"
                    data-test=format!("websub-redrive-{phase_key}")
                    prop:disabled=move || selection.get().is_empty()
                    on:click=submit
                >
                    "Redrive selected"
                </button>
            </div>
            {move || mutation_flash(
                redrive.value().get(),
                "Selected dead-letter events queued for redrive.",
            )}
        </div>
    }
}

fn dead_letter_table(
    page: DeadLetterPage,
    selection: RwSignal<Vec<FeedEventId>>,
    cursor: RwSignal<Option<DeadLetterCursor>>,
    empty_message: &'static str,
) -> impl IntoView {
    if page.events.is_empty() {
        return view! { <p class="j-websub-empty">{empty_message}</p> }.into_any();
    }
    let next = page.next_cursor;
    view! {
        <table class="j-table">
            <thead>
                <tr>
                    <th>"Select"</th>
                    <th>"Event"</th>
                    <th>"Feed"</th>
                    <th>"Phase"</th>
                    <th>"Attempts"</th>
                    <th>"Terminal time"</th>
                    <th>"Diagnostic"</th>
                </tr>
            </thead>
            <tbody>
                {page
                    .events
                    .into_iter()
                    .map(move |row| dead_letter_row(row, selection))
                    .collect_view()}
            </tbody>
        </table>
        {next
            .map(|next| {
                view! {
                    <button
                        type="button"
                        class="j-btn"
                        data-test="websub-next-page"
                        on:click=move |_| cursor.set(Some(next))
                    >
                        "Next page"
                    </button>
                }
            })}
    }
    .into_any()
}

fn dead_letter_row(row: DeadLetterRow, selection: RwSignal<Vec<FeedEventId>>) -> impl IntoView {
    let id = row.id;
    view! {
        <tr>
            <td>
                <input
                    type="checkbox"
                    aria-label=format!("Select event {id}")
                    on:change=move |event| {
                        selection
                            .update(|selected| {
                                if event_target_checked(&event) {
                                    selected.push(id);
                                } else {
                                    selected.retain(|selected_id| *selected_id != id);
                                }
                            });
                    }
                />
            </td>
            <td>{id.to_string()}</td>
            <td>{row.feed_path}</td>
            <td>{format!("{:?}", row.phase)}</td>
            <td>{row.attempts}</td>
            <td>{row.terminal_at.to_string()}</td>
            <td>{row.diagnostic.unwrap_or_default()}</td>
        </tr>
    }
}

fn mutation_flash(
    result: Option<Result<MutationOutcome<()>, WebError>>,
    success: &'static str,
) -> Option<AnyView> {
    result.map(|result| {
        match mutation_feedback::classify(result, "The save outcome is unknown; reload to verify.")
        {
            MutationFeedback::Confirmed(()) => {
                view! { <p class="success">{success}</p> }.into_any()
            }
            MutationFeedback::Error(error) => view! { <p class="error">{error}</p> }.into_any(),
        }
    })
}
