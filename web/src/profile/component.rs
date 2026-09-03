use crate::auth;
use crate::error::WebError;
use crate::forms::{Field, ValidatedInput, ValidatedTextarea};
use crate::topbar::Topbar;
use common::{
    MutationOutcome, bio::Bio, display_name::DisplayName, render::PostFormat, theme::Theme,
};
use leptos::prelude::*;

use super::api::{
    self, ResetYourPagesTheme, SetDefaultPostFormat, SetSiteTheme, SetYourPagesTheme,
};
use super::{DefaultPostFormatState, ThemeControlState, ThemeMutationDecision, ThemeSelection};

/// Profile page — shows username, display name, bio; allows updating.
#[component]
pub fn ProfilePage() -> impl IntoView {
    let update_action = ServerAction::<api::Update>::new();
    let profile = Resource::new(move || update_action.version().get(), |_| api::get());
    // Client-validated display name and bio (both optional: empty clears them),
    // owned by the component so the bespoke form can `.dispatch` the typed
    // `Update` args — the ADR-0065 direct-bind pattern (mirrors the post
    // compose/edit forms); an `<ActionForm>`'s string fields cannot carry
    // validated `Option<DisplayName>`/`Option<Bio>`.
    let dn_field = Field::<DisplayName>::optional();
    let bio_field = Field::<Bio>::optional();

    view! {
        <Topbar title="Profile" sub="Your details" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match profile.await {
                            Ok(data) => {
                                dn_field
                                    .value
                                    .set(
                                        data.display_name.as_deref().unwrap_or_default().to_string(),
                                    );
                                bio_field
                                    .value
                                    .set(data.bio.as_deref().unwrap_or_default().to_string());
                                let submit = move |_| {
                                    update_action
                                        .dispatch(api::Update {
                                            display_name: dn_field.parsed(),
                                            bio: bio_field.parsed(),
                                        });
                                };
                                // Seed the form from the persisted profile. This re-runs
                                // (re-seeding) whenever a successful update bumps the
                                // resource; a stored display name is always valid, so the
                                // optional field stays valid.
                                view! {
                                    <div class="j-card">
                                        <div class="j-card-head">
                                            <div>
                                                <h2>"Profile"</h2>
                                                <div class="j-sub">"Your display name and bio."</div>
                                            </div>
                                        </div>
                                        <div class="j-form-body">
                                            <p>"Username: " {data.username.to_string()}</p>
                                            <ValidatedInput<DisplayName>
                                                label="Display Name"
                                                name="display_name"
                                                field=dn_field
                                            />
                                            <ValidatedTextarea<Bio>
                                                label="Bio"
                                                name="bio"
                                                field=bio_field
                                            />
                                        </div>
                                        <div class="j-form-actions">
                                            <button
                                                type="button"
                                                class="j-btn is-primary"
                                                prop:disabled=move || {
                                                    !dn_field.is_valid() || !bio_field.is_valid()
                                                }
                                                on:click=submit
                                            >
                                                "Update Profile"
                                            </button>
                                        </div>
                                    </div>
                                    <DefaultPostFormatControl />
                                }
                                    .into_any()
                            }
                            Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        }
                    })}
                </Suspense>
                <ThemeControl />
                {move || {
                    update_action
                        .value()
                        .get()
                        .and_then(|result: Result<MutationOutcome<()>, WebError>| {
                            match crate::mutation_feedback::classify(
                                result,
                                "Your profile may have been updated, but its status could not be confirmed. Refresh to check.",
                            ) {
                                crate::mutation_feedback::MutationFeedback::Confirmed(()) => None,
                                crate::mutation_feedback::MutationFeedback::Error(message) => {
                                    Some(view! { <p class="error">{message}</p> }.into_any())
                                }
                            }
                        })
                }}
            </div>
        </div>
    }
}

/// Persisted public-theme controls. Every authenticated author receives an
/// author-scoped control; operators additionally receive the site control.
#[component]
fn ThemeControl() -> impl IntoView {
    let session = auth::use_session();
    view! {
        {move || match session.current.get() {
            Some(user) => {
                view! {
                    <AuthorThemeControl />
                    {user.is_operator.then(|| view! { <SiteThemeControl /> })}
                }
                    .into_any()
            }
            None => ().into_any(),
        }}
    }
}

struct AuthorThemeRuntime {
    action: ServerAction<SetYourPagesTheme>,
    reset: ServerAction<ResetYourPagesTheme>,
    state: RwSignal<ThemeControlState>,
    error: RwSignal<Option<String>>,
}

fn author_theme_runtime() -> AuthorThemeRuntime {
    let action = ServerAction::<SetYourPagesTheme>::new();
    let reset = ServerAction::<ResetYourPagesTheme>::new();
    let reload = RwSignal::new(0);
    let persisted = Resource::new(move || reload.get(), |_| api::get_your_pages_theme());
    let state = RwSignal::new(ThemeControlState::Loading);
    let error = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        if let Some(result) = persisted.get() {
            let last = state.get_untracked().selection();
            if let Err(problem) = &result {
                error.set(Some(problem.to_string()));
            }
            state.set(ThemeControlState::resolve(last, Some(&result)));
        }
    });
    Effect::new(move |_| {
        if let Some(result) = action.value().get() {
            match ThemeControlState::mutation_decision(&result) {
                ThemeMutationDecision::Error => {
                    if let Err(problem) = result {
                        error.set(Some(problem.to_string()));
                    }
                }
                ThemeMutationDecision::RevalidateConfirmed => {
                    error.set(None);
                    reload.update(|version| *version += 1);
                }
                ThemeMutationDecision::RevalidateIndeterminate => {
                    error.set(Some(
                        "The theme change may have committed; its status could not be confirmed."
                            .into(),
                    ));
                    reload.update(|version| *version += 1);
                }
            }
        }
    });
    Effect::new(move |_| {
        if let Some(result) = reset.value().get() {
            match ThemeControlState::mutation_decision(&result) {
                ThemeMutationDecision::Error => {
                    if let Err(problem) = result {
                        error.set(Some(problem.to_string()));
                    }
                }
                ThemeMutationDecision::RevalidateConfirmed => {
                    error.set(None);
                    reload.update(|version| *version += 1);
                }
                ThemeMutationDecision::RevalidateIndeterminate => {
                    error.set(Some(
                        "The reset may have committed; its status could not be confirmed.".into(),
                    ));
                    reload.update(|version| *version += 1);
                }
            }
        }
    });

    AuthorThemeRuntime {
        action,
        reset,
        state,
        error,
    }
}

#[component]
fn AuthorThemeControl() -> impl IntoView {
    let AuthorThemeRuntime {
        action,
        reset,
        state,
        error,
    } = author_theme_runtime();

    view! {
        <div class="j-card">
            <div class="j-card-head">
                <div>
                    <h2>"Your pages theme"</h2>
                    <div class="j-sub">"Choose how your public pages look."</div>
                </div>
            </div>
            <div class="j-form-body">
                <div class="j-seg" role="group" aria-label="Your pages theme">
                    {[
                        (ThemeSelection::SiteDefault, "Site default"),
                        (ThemeSelection::Theme(Theme::Terminal), "Terminal"),
                        (ThemeSelection::Theme(Theme::Studio), "Studio"),
                        (ThemeSelection::Theme(Theme::Reader), "Reader"),
                    ]
                        .into_iter()
                        .map(move |(selection, label)| {
                            view! {
                                <button
                                    type="button"
                                    class=move || selection.button_class(state.get().selection())
                                    aria-pressed=move || {
                                        selection.aria_pressed(state.get().selection())
                                    }
                                    prop:disabled=move || state.get().is_loading()
                                    on:click=move |_| {
                                        match selection {
                                            ThemeSelection::SiteDefault => {
                                                reset.dispatch(ResetYourPagesTheme {});
                                            }
                                            ThemeSelection::Theme(theme) => {
                                                action.dispatch(SetYourPagesTheme { theme });
                                            }
                                        }
                                    }
                                >
                                    {label}
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
                {move || error.get().map(|message| view! { <p class="error">{message}</p> })}
            </div>
        </div>
    }
}

struct SiteThemeRuntime {
    action: ServerAction<SetSiteTheme>,
    state: RwSignal<ThemeControlState>,
    error: RwSignal<Option<String>>,
}

fn site_theme_runtime() -> SiteThemeRuntime {
    let action = ServerAction::<SetSiteTheme>::new();
    let reload = RwSignal::new(0);
    let persisted = Resource::new(move || reload.get(), |_| api::get_site_theme());
    let state = RwSignal::new(ThemeControlState::Loading);
    let error = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        if let Some(result) = persisted.get() {
            let last = state.get_untracked().selection();
            state.set(match result {
                Ok(theme) => ThemeControlState::Ready(ThemeSelection::Theme(theme)),
                Err(problem) => {
                    error.set(Some(problem.to_string()));
                    ThemeControlState::Failed(last)
                }
            });
        }
    });
    Effect::new(move |_| {
        if let Some(result) = action.value().get() {
            match ThemeControlState::mutation_decision(&result) {
                ThemeMutationDecision::Error => {
                    if let Err(problem) = result {
                        error.set(Some(problem.to_string()));
                    }
                }
                ThemeMutationDecision::RevalidateConfirmed => {
                    error.set(None);
                    reload.update(|version| *version += 1);
                }
                ThemeMutationDecision::RevalidateIndeterminate => {
                    error.set(Some(
                        "The site theme change may have committed; its status could not be confirmed."
                            .into(),
                    ));
                    reload.update(|version| *version += 1);
                }
            }
        }
    });

    SiteThemeRuntime {
        action,
        state,
        error,
    }
}

#[component]
fn SiteThemeControl() -> impl IntoView {
    let SiteThemeRuntime {
        action,
        state,
        error,
    } = site_theme_runtime();

    view! {
        <div class="j-card">
            <div class="j-card-head">
                <div>
                    <h2>"Site theme"</h2>
                    <div class="j-sub">"Choose the default theme for public pages."</div>
                </div>
            </div>
            <div class="j-form-body">
                <div class="j-seg" role="group" aria-label="Site theme">
                    {[
                        (Theme::Terminal, "Terminal"),
                        (Theme::Studio, "Studio"),
                        (Theme::Reader, "Reader"),
                    ]
                        .into_iter()
                        .map(move |(theme, label)| {
                            view! {
                                <button
                                    type="button"
                                    class=move || {
                                        ThemeSelection::Theme(theme)
                                            .button_class(state.get().selection())
                                    }
                                    aria-pressed=move || {
                                        ThemeSelection::Theme(theme)
                                            .aria_pressed(state.get().selection())
                                    }
                                    prop:disabled=move || state.get().is_loading()
                                    on:click=move |_| {
                                        action.dispatch(SetSiteTheme { theme });
                                    }
                                >
                                    {label}
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
                {move || error.get().map(|message| view! { <p class="error">{message}</p> })}
            </div>
        </div>
    }
}

/// Control for setting the user's default post format preference.
///
/// ADR-0065 direct-bind: a [`DefaultPostFormatState`] signal owned by the
/// component is seeded only by the persisted preference, bound to a `<select>`
/// whose `on:change` parses the token, and read by a plain `type="button"` "Save"
/// that `.dispatch`es the typed `SetDefaultPostFormat` action — no
/// `<ActionForm>` / string-submit path.
#[component]
fn DefaultPostFormatControl() -> impl IntoView {
    let action = ServerAction::<SetDefaultPostFormat>::new();
    let initial = Resource::new(|| (), |()| api::get_default_post_format());
    // The state belongs to the component rather than the transient Suspend
    // scope. Loading and Failed deliberately carry no format, so neither can
    // dispatch a fabricated Markdown preference.
    let state = RwSignal::new(DefaultPostFormatState::Loading);
    let save = move |_| {
        if let Some(format) = state.get().format_to_save() {
            action.dispatch(SetDefaultPostFormat { format });
        }
    };

    view! {
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                let resolved = initial.await;
                state.set(DefaultPostFormatState::resolve(Some(&resolved)));
                view! {
                    <div class="j-card">
                        <div class="j-card-head">
                            <div>
                                <h2>"Default Post Format"</h2>
                                <div class="j-sub">"The editor format new posts start in."</div>
                            </div>
                        </div>
                        <div class="j-form-body">
                            <DefaultPostFormatBody state=state />
                        </div>
                        {move || {
                            action
                                .value()
                                .get()
                                .and_then(|result: Result<MutationOutcome<()>, WebError>| {
                                    match crate::mutation_feedback::classify(
                                        result,
                                        "Save acknowledgement was lost; reload to verify the default post format.",
                                    ) {
                                        crate::mutation_feedback::MutationFeedback::Confirmed(
                                            (),
                                        ) => None,
                                        crate::mutation_feedback::MutationFeedback::Error(
                                            message,
                                        ) => Some(message),
                                    }
                                })
                                .map(|error| view! { <p class="error">{error}</p> })
                        }}
                        <div class="j-form-actions">
                            <button
                                type="button"
                                class="j-btn"
                                prop:disabled=move || state.get().format_to_save().is_none()
                                on:click=save
                            >
                                "Save"
                            </button>
                        </div>
                    </div>
                }
            })}
        </Suspense>
    }
}

/// The resolved preference body: loading, explicit failure, or the real select.
#[component]
fn DefaultPostFormatBody(state: RwSignal<DefaultPostFormatState>) -> impl IntoView {
    view! {
        <Show
            when=move || matches!(state.get(), DefaultPostFormatState::Ready(_))
            fallback=move || {
                view! {
                    <Show
                        when=move || matches!(state.get(), DefaultPostFormatState::Loading)
                        fallback=|| {
                            view! { <p class="error">"Could not load the default post format."</p> }
                        }
                    >
                        <p class="j-loading">"Loading\u{2026}"</p>
                    </Show>
                }
            }
        >
            <DefaultPostFormatSelect state=state />
        </Show>
    }
}

/// The ready preference select, derived from user-selectable [`PostFormat`] variants.
#[component]
fn DefaultPostFormatSelect(state: RwSignal<DefaultPostFormatState>) -> impl IntoView {
    use strum::{EnumMessage, VariantArray};

    let change = move |ev| {
        if let Ok(format) = event_target_value(&ev).parse::<PostFormat>() {
            state.set(DefaultPostFormatState::Ready(format));
        }
    };

    view! {
        <label class="j-form-field">
            <span class="j-form-label">"Default post format"</span>
            <select id="default-post-format" class="j-form-input" on:change=change>
                <For
                    each=move || {
                        PostFormat::VARIANTS
                            .iter()
                            .copied()
                            .filter_map(|format| {
                                format.get_message().map(|label| (format, label))
                            })
                    }
                    key=|(format, _)| *format
                    children=move |(format, label)| {
                        view! {
                            <option
                                value=format.to_string()
                                selected=move || state.get().format_to_save() == Some(format)
                            >
                                {label}
                            </option>
                        }
                    }
                />
            </select>
        </label>
    }
}
