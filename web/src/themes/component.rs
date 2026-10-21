//! Wasm-only private Studio journey for Theme Package management.

use std::borrow::Cow;

use common::{
    MutationOutcome,
    ids::ThemeId,
    pagination::PageSize,
    theme::{PublicThemeSelection, Theme, ThemeImageRole},
};
use leptos::prelude::*;
use leptos::task;
use strum::VariantArray;

use crate::{
    auth,
    error::WebError,
    icon::{Icon, Icons},
    media::Item as MediaItem,
    reactive::Invalidator,
    topbar::Topbar,
};

use super::editor_inputs::{
    draft_package_editor_text, editor_catalog_name, image_patch_token, kept_image_label,
    normal_media_options, require_editor_lifetime, staged_media_token, tab_destination,
};
use super::page_state::{
    self, EditorState, Feedback, MutationStage, Revalidation, ScopeAvailability, ThemeChoice,
    ThemePageState,
};
use super::presentation_state::{HeaderMediaDraft, MEDIA_PAGE_SIZE, media_page_offset};
use super::{
    CatalogEntry, OwnershipScope, ThemeBindingInput, ThemeEditorPatch, ThemeMediaInput,
    ThemePoolInput, ThemePresentation, api,
};

/// The authenticated Studio route. It deliberately only renders server-provided preview
/// bytes inside a sandboxed iframe; no draft CSS is ever inserted into Studio's document.
#[component]
pub fn ThemesPage() -> impl IntoView {
    let session = auth::use_session().current;
    let page = ThemePageState::default();
    let scope = page.scope;
    let (catalog, selection) = owner_resources(scope, page.refresh);

    view! {
        {move || match session.get() {
            None => {
                view! {
                    <Topbar title="Themes" />
                    <p class="error" role="alert">
                        "Sign in to manage themes."
                    </p>
                }
                    .into_any()
            }
            Some(user) => {
                view! {
                    <For
                        each=move || vec![scope.get()]
                        key=|scope| *scope == OwnershipScope::Site
                        children=move |_| {
                            view! {
                                <ThemeWorkspace
                                    page=page
                                    catalog=catalog
                                    selection=selection
                                    availability=ScopeAvailability::from_session(
                                        true,
                                        user.is_operator,
                                    )
                                />
                            }
                        }
                    />
                }
                    .into_any()
            }
        }}
    }
}
type ThemeCatalogResource = Resource<Result<Vec<CatalogEntry>, WebError>>;

/// One workspace addresses two independent settings; owner changes remount local inputs.
#[component]
fn ThemeWorkspace(
    page: ThemePageState,
    catalog: ThemeCatalogResource,
    selection: ThemeSelectionResource,
    availability: ScopeAvailability,
) -> impl IntoView {
    view! {
        <Topbar
            title="Themes"
            sub=move || match page.scope.get() {
                OwnershipScope::Author => {
                    "Choose how visitors see your public pages. This does not change your private workspace."
                }
                OwnershipScope::Site => {
                    "Choose the default look for public site pages and user pages using the site theme."
                }
            }
        />
        <ThemeViewBody page=page catalog=catalog selection=selection availability=availability />
    }
}

#[component]
fn ThemeViewBody(
    page: ThemePageState,
    catalog: ThemeCatalogResource,
    selection: ThemeSelectionResource,
    availability: ScopeAvailability,
) -> impl IntoView {
    view! {
        <div class="j-scroll">
            <div class="j-page j-themes-page" data-theme-management="studio">
                <fieldset
                    class="j-theme-workflow"
                    disabled=move || page.pending.get()
                    style="border:0;padding:0;margin:0;min-width:0"
                >
                    <ThemeWorkflow
                        page=page
                        catalog=catalog
                        selection=selection
                        availability=availability
                    />
                </fieldset>
            </div>
        </div>
    }
}

#[component]
fn ThemeWorkflow(
    availability: ScopeAvailability,
    page: ThemePageState,
    catalog: ThemeCatalogResource,
    selection: ThemeSelectionResource,
) -> impl IntoView {
    let scope = page.scope;
    let package_file = NodeRef::<leptos::html::Input>::new();
    view! {
        <ThemeViewNavigation availability=availability page=page />
        <div
            class="j-theme-workspace-panel"
            id="theme-workspace-panel"
            role=move || availability.permits(OwnershipScope::Site).then_some("tabpanel")
            aria-labelledby=move || {
                availability.permits(OwnershipScope::Site).then_some(theme_tab_id(scope.get()))
            }
            tabindex="0"
        >
            <Show when=move || page.status.get().is_some()>
                <div class="j-theme-feedback">
                    <ThemeFeedback status=page.status feedback=page.feedback />
                </div>
            </Show>
            <section class="j-theme-available" aria-labelledby="theme-add-heading">
                <div class="j-theme-library-head">
                    <h2 id="theme-add-heading">"Available themes"</h2>
                    <div class="j-theme-library-actions">
                        <button
                            type="button"
                            class="j-btn"
                            data-theme-create
                            on:click=move |_| page.select_choice(ThemeChoice::New)
                        >
                            "Create theme"
                        </button>
                        {zip_import_button(package_file, page)}
                    </div>
                </div>
                <ThemeCurrent catalog=catalog selection=selection scope=scope />
                <ul class="j-theme-grid" aria-label="Available themes">
                    <ThemeReadyCards page=page selection=selection />
                    {catalog_content(catalog, page, selection)}
                </ul>
            </section>
            <section
                id="theme-editor"
                class="j-theme-editor-section"
                aria-labelledby="theme-editor-heading"
            >
                <h2 id="theme-editor-heading">{move || editor_heading(page, catalog)}</h2>
                <Show when=move || matches!(page.selected.get(), Some(ThemeChoice::Bundled(_)))>
                    <SelectedThemePreview page=page />
                </Show>
                <ThemeSelectedEditor page=page catalog=catalog />
                <details class="j-theme-tool-group">
                    <summary>"Theme authoring guide"</summary>
                    <div class="j-theme-tool-body">
                        <section
                            class="j-card j-theme-authoring-guidance"
                            aria-labelledby="theme-authoring-guidance-heading"
                        >
                            <h2 id="theme-authoring-guidance-heading">
                                "Author themes from a repository"
                            </h2>
                            <p class="j-sub">
                                "Keep your theme source, preview, and release artifacts together, then use Jaunder to validate and package them."
                            </p>
                            <a
                                href="https://github.com/jaunder-org/jaunder/blob/main/docs/themes.md"
                                target="_blank"
                            >
                                "Read the theme repository guide"
                            </a>
                        </section>
                    </div>
                </details>
                <details class="j-theme-tool-group">
                    <summary>"Published versions"</summary>
                    <div class="j-theme-tool-body">
                        <ThemeSavedVersions page=page catalog=catalog selection=selection />
                    </div>
                </details>
            </section>
        </div>
    }
}

fn editor_heading(page: ThemePageState, catalog: ThemeCatalogResource) -> String {
    match page.selected.get() {
        Some(ThemeChoice::New) => "Create theme".into(),
        Some(ThemeChoice::Custom(id)) => catalog
            .get()
            .and_then(Result::ok)
            .and_then(|entries| entries.into_iter().find(|entry| entry.id == id))
            .map_or_else(
                || "Theme editor".into(),
                |entry| format!("Edit {}", entry.name),
            ),
        _ => "Theme editor".into(),
    }
}

#[component]
fn SelectedThemePreview(page: ThemePageState) -> impl IntoView {
    let target = Signal::derive(move || {
        page_state::preview_target(page.scope.get(), page.selected.get(), page.refresh.track())
    });
    // Preview shares expensive-operation admission with writes, so reads are explicit,
    // not eager work triggered by every card, Save, publication or catalog refresh.
    let preview = Action::new(|target: &page_state::PreviewTarget| {
        let target = *target;
        async move {
            (
                target,
                api::preview_installed(target.scope, target.theme)
                    .await
                    .and_then(|preview| styled_preview_document(&preview)),
            )
        }
    });
    view! {
        <Show when=move || target.get().is_some()>
            <section class="j-theme-editor-preview" aria-label="Theme preview">
                <p class="j-form-help">
                    "Built-in theme · read-only. Copy it to edit your own version."
                </p>
                <button
                    type="button"
                    class="j-btn"
                    disabled=move || page.pending.get() || preview.pending().get()
                    on:click=move |_| {
                        if let Some(target) = target
                            .get_untracked()
                            .filter(|_| {
                                !page.pending.get_untracked() && !preview.pending().get_untracked()
                            })
                        {
                            preview.dispatch(target);
                        }
                    }
                >
                    "Preview"
                </button>
                <Show when=move || preview.pending().get()>
                    <p class="j-loading" aria-live="polite">
                        "Loading saved theme preview…"
                    </p>
                </Show>
                <Show when=move || {
                    !preview.pending().get()
                        && page_state::preview_is_current(
                            target.get(),
                            preview.value().get().map(|(target, _)| target),
                        )
                }>
                    {move || match preview.value().get() {
                        Some((_, Ok(preview))) => {
                            view! {
                                <iframe
                                    class="j-theme-preview"
                                    title="Saved theme preview"
                                    sandbox=""
                                    srcdoc=preview
                                ></iframe>
                            }
                                .into_any()
                        }
                        Some((_, Err(error))) => {
                            view! {
                                <p class="error" role="alert">
                                    {format!(
                                        "Saved theme preview unavailable: {error}. Your theme choices are unchanged.",
                                    )}
                                </p>
                            }
                                .into_any()
                        }
                        None => ().into_any(),
                    }}
                </Show>
            </section>
        </Show>
    }
}

#[component]
fn ThemeViewNavigation(availability: ScopeAvailability, page: ThemePageState) -> impl IntoView {
    view! {
        <Show when=move || availability.permits(OwnershipScope::Site)>
            <div class="j-theme-scope-options">
                <div class="j-theme-scope-tabs" role="tablist" aria-label="Theme setting">
                    <ThemeScopeTab page=page target=OwnershipScope::Author label="User" />
                    <ThemeScopeTab page=page target=OwnershipScope::Site label="Site" />
                </div>
            </div>
        </Show>
    }
}

fn theme_tab_id(scope: OwnershipScope) -> &'static str {
    match scope {
        OwnershipScope::Author => "theme-tab-user",
        OwnershipScope::Site => "theme-tab-site",
    }
}

#[component]
fn ThemeScopeTab(
    page: ThemePageState,
    target: OwnershipScope,
    label: &'static str,
) -> impl IntoView {
    view! {
        <button
            type="button"
            class="j-btn"
            role="tab"
            id=theme_tab_id(target)
            aria-controls="theme-workspace-panel"
            aria-selected=move || aria_pressed(page.scope.get() == target)
            tabindex=move || if page.scope.get() == target { "0" } else { "-1" }
            on:click=move |_| page.select_scope(target)
            on:keydown=move |event| theme_tab_key(page, target, &event)
        >
            {label}
        </button>
    }
}

fn theme_tab_key(page: ThemePageState, current: OwnershipScope, event: &web_sys::KeyboardEvent) {
    let Some(next) = tab_destination(page.pending.get_untracked(), current, &event.key()) else {
        return;
    };
    event.prevent_default();
    page.select_scope(next);
    // The owner-keyed workspace remounts; focus the newly mounted tab.
    request_animation_frame(move || {
        if let Some(tab) = document().get_element_by_id(theme_tab_id(next)) {
            use wasm_bindgen::JsCast;
            if let Some(tab) = tab.dyn_ref::<web_sys::HtmlElement>()
                && tab.focus().is_err()
            {
                tracing::warn!("Unable to focus theme setting tab");
            }
        }
    });
}

fn zip_import_button(file: NodeRef<leptos::html::Input>, page: ThemePageState) -> impl IntoView {
    view! {
        <div class="j-theme-upload">
            <input
                type="file"
                accept=".zip,application/zip"
                aria-label="Theme ZIP file"
                node_ref=file
                hidden
                on:change=move |_| import_package_zip(file, page)
            />
            <button
                type="button"
                class="j-btn"
                data-theme-upload
                on:click=move |_| {
                    if let Some(input) = file.get() {
                        input.click();
                    }
                }
            >
                "Upload theme ZIP"
            </button>
        </div>
    }
}

fn copy_card(page: ThemePageState, choice: ThemeChoice) {
    if let Some(source) = choice.selection() {
        copy_theme(page, source);
    }
}

fn copy_theme(page: ThemePageState, source: PublicThemeSelection) {
    let scope = page.scope.get_untracked();
    if !page.begin_mutation() {
        return;
    }
    page.feedback_in_import.set(true);
    page.status.set(Some("Copying saved theme…".into()));
    let lifetime = page.lifetime();
    task::spawn_local(async move {
        let result = api::copy_theme(scope, source).await;
        if !lifetime.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        if page.scope.get_untracked() == scope {
            set_mutation_feedback(page, MutationStage::Copied, &result);
            if let Ok(MutationOutcome::Confirmed(entry)) = result {
                page.selected.set(Some(ThemeChoice::Custom(entry.id)));
                page.status.set(Some(format!(
                    "Copied {}. Saved privately; choose its tile to Use it.",
                    entry.name
                )));
            }
            // One resource revalidation, never a second mutation or an inferred destination.
            page.refresh.notify();
        }
        page.pending.set(false);
    });
}

type ThemeSelectionResource = Resource<Result<Option<PublicThemeSelection>, WebError>>;
type ThemePresentationResource = Resource<Result<Option<ThemePresentation>, WebError>>;
type OwnerMediaResource = Resource<Result<Vec<MediaItem>, WebError>>;

fn owner_resources(
    scope: RwSignal<OwnershipScope>,
    refresh: Invalidator,
) -> (ThemeCatalogResource, ThemeSelectionResource) {
    let catalog = Resource::new(
        move || (scope.get(), refresh.track()),
        |(scope, _)| async move { api::list(scope).await },
    );
    let selection = Resource::new(
        move || (scope.get(), refresh.track()),
        |(scope, _)| async move { api::get_selection(scope).await },
    );
    (catalog, selection)
}

fn presentation_resources(
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    media_page: RwSignal<u32>,
) -> (ThemePresentationResource, OwnerMediaResource) {
    let presentation = Resource::new(
        move || (scope.get(), selected.get(), refresh.track()),
        |(scope, selected, _)| async move {
            match selected {
                Some(id) => api::get_presentation(scope, id).await.map(Some),
                None => Ok(None),
            }
        },
    );
    let media = Resource::new(
        move || (refresh.track(), media_page.get()),
        |(_, page)| async move {
            crate::media::list_mine(
                None,
                Some(PageSize::default()),
                Some(media_page_offset(page)),
            )
            .await
        },
    );
    (presentation, media)
}

fn import_package_zip(package_file: NodeRef<leptos::html::Input>, page: ThemePageState) {
    let scope = page.scope;
    let refresh = page.refresh;
    let status = page.status;
    if page.pending.get_untracked() {
        return;
    }
    page.feedback_in_import.set(true);
    page.feedback.set(Feedback::Error);
    // crap:allow: Browser File and FormData APIs are wasm-only; host coverage cannot execute the upload handler.
    let Some(file) = package_file
        .get()
        .and_then(|input| input.files())
        .and_then(|files| files.get(0))
    else {
        status.set(Some("Choose a theme ZIP file first.".into()));
        return;
    };
    let captured_scope = scope.get_untracked();
    let scope_name = match captured_scope {
        OwnershipScope::Author => "author",
        OwnershipScope::Site => "site",
    };
    let Ok(form) = leptos::web_sys::FormData::new() else {
        status.set(Some("Could not prepare the package upload.".into()));
        return;
    };
    if form.append_with_str("scope", scope_name).is_err()
        || form.append_with_blob("archive", &file).is_err()
    {
        status.set(Some("Could not prepare the package upload.".into()));
        return;
    }
    page.pending.set(true);
    page.feedback.set(Feedback::Progress);
    status.set(Some("Uploading theme…".into()));
    task::spawn_local(async move {
        match api::import_zip(form.into()).await {
            Ok(MutationOutcome::Confirmed(entry)) => {
                if page.scope.get_untracked() == captured_scope {
                    page.selected.set(Some(ThemeChoice::Custom(entry.id)));
                    page.feedback.set(Feedback::Success);
                    refresh.notify();
                    status.set(Some(format!(
                        "Uploaded {}. Saved privately; choose its tile to Use it.",
                        entry.name
                    )));
                }
            }
            result => {
                set_mutation_feedback(page, MutationStage::Import, &result);
                reconcile_owner(captured_scope, refresh, status, None).await;
            }
        }
        if let Some(input) = package_file.get() {
            input.set_value("");
        }
        page.pending.set(false);
    });
}

fn settle<T>(
    page: ThemePageState,
    result: Result<MutationOutcome<T>, WebError>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    settle_revalidation(page, page_state::revalidation(result), refresh, status);
}

fn settle_context<T>(
    page: ThemePageState,
    scope: OwnershipScope,
    target: ThemeId,
    result: Result<MutationOutcome<T>, WebError>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    let outcome = page.context_revalidation(scope, target, result);
    settle_revalidation(page, outcome, refresh, status);
}

fn settle_revalidation(
    page: ThemePageState,
    outcome: Revalidation,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    match outcome {
        Revalidation::Confirmed => {
            page.feedback.set(Feedback::Success);
            refresh.notify();
            status.set(Some("Saved. Checking available themes…".into()));
        }
        Revalidation::Indeterminate => {
            page.feedback.set(Feedback::Error);
            refresh.notify();
            status.set(Some("The server could not confirm this change. Checking available themes and your current setting; review them before continuing. No automatic retry ran.".into()));
        }
        Revalidation::Failed(message) => {
            page.feedback.set(Feedback::Error);
            refresh.notify();
            status.set(Some(format!("The change was not confirmed. Check available themes and your current setting before continuing. {message}")));
        }
    }
}

#[component]
fn ThemeCurrent(
    catalog: ThemeCatalogResource,
    selection: ThemeSelectionResource,
    scope: RwSignal<OwnershipScope>,
) -> impl IntoView {
    view! {
        <div
            class="j-theme-current"
            class:j-theme-current-accessible=move || matches!(selection.get(), Some(Ok(_)))
            aria-label="Current theme setting"
        >
            <p
                aria-label="Current theme"
                data-theme-selection=move || page_state::selection_control_token(
                    selection.get().as_ref(),
                )
            >
                {move || page_state::current_theme_summary(
                    scope.get(),
                    selection.get().as_ref(),
                    catalog.get().as_ref(),
                )}
            </p>
        </div>
    }
}

#[component]
fn ThemeSelectedEditor(page: ThemePageState, catalog: ThemeCatalogResource) -> impl IntoView {
    let selected = Signal::derive(move || page.selected.get().and_then(ThemeChoice::custom_id));
    let scope = page.scope;
    view! {
        <ThemeReadOnlyEditor page=page />
        <div>
            <For
                each=move || {
                    page.selected
                        .get()
                        .filter(|choice| {
                            matches!(choice, ThemeChoice::New | ThemeChoice::Custom(_))
                        })
                        .map(|choice| (scope.get(), choice.custom_id()))
                        .into_iter()
                        .collect::<Vec<_>>()
                }
                key=|target| (target.0 == OwnershipScope::Site, target.1)
                children=move |_| {
                    view! {
                        <ThemeEditor
                            page=page
                            scope=scope
                            selected=selected
                            refresh=page.refresh
                            status=page.status
                            catalog=catalog
                        />
                    }
                }
            />
        </div>
    }
}

/// Renders the asynchronous catalog resource without adding state branches to
/// the catalog card itself.
fn catalog_content(
    catalog: ThemeCatalogResource,
    page: ThemePageState,
    selection: ThemeSelectionResource,
) -> impl IntoView {
    let retained = RwSignal::new(Vec::<CatalogEntry>::new());
    // A failed reread must expose its error without removing explicit recovery actions.
    // This local signal is discarded with the owner-keyed workspace.
    Effect::new(move |_| {
        if let Some(Ok(entries)) = catalog.get() {
            retained.set(entries);
        }
    });
    view! {
        <For
            each=move || retained.get()
            key=|entry| (entry.id, entry.name.clone())
            children=move |entry| {
                view! {
                    <ThemeChoiceCard
                        page=page
                        selection=selection
                        choice=ThemeChoice::Custom(entry.id)
                        name=entry.name
                        custom=true
                    />
                }
            }
        />
        {move || match catalog.get() {
            None => {
                view! {
                    <li>
                        <p class="j-loading">"Loading available themes…"</p>
                    </li>
                }
                    .into_any()
            }
            Some(Ok(_)) => ().into_any(),
            Some(Err(error)) => {
                view! {
                    <li>
                        <p class="error" role="alert">
                            {error.to_string()}
                        </p>
                    </li>
                }
                    .into_any()
            }
        }}
    }
}

#[component]
fn ThemeReadyCards(page: ThemePageState, selection: ThemeSelectionResource) -> impl IntoView {
    view! {
        <Show when=move || page.scope.get() == OwnershipScope::Author>
            <ThemeChoiceCard
                page=page
                selection=selection
                choice=ThemeChoice::FollowSite
                name="Follow site theme".into()
                custom=false
            />
        </Show>
        <For
            each=move || built_in_themes()
            key=|theme| *theme
            children=move |theme| {
                view! {
                    <ThemeChoiceCard
                        page=page
                        selection=selection
                        choice=ThemeChoice::Bundled(theme)
                        name=theme.to_string()
                        custom=false
                    />
                }
            }
        />
    }
}

#[component]
fn ThemeChoiceCard(
    page: ThemePageState,
    selection: ThemeSelectionResource,
    choice: ThemeChoice,
    name: String,
    custom: bool,
) -> impl IntoView {
    let use_name = if choice == ThemeChoice::FollowSite {
        "Use site theme".into()
    } else {
        format!("Use {name}")
    };
    let edit_name = if custom {
        format!("Edit {name}")
    } else {
        format!("Preview {name}")
    };
    let copy_name = format!("Copy {name}");
    let editable = choice.selection().is_some();
    let token = page_state::selection_token(choice.selection());
    view! {
        <li
            class="j-theme-choice"
            data-theme-card=token
            data-current=move || aria_pressed(
                page_state::choice_is_current(choice, page.scope.get(), selection.get().as_ref()),
            )
            data-editing=move || aria_pressed(page.selected.get() == Some(choice))
        >
            <button
                type="button"
                class="j-theme-choice-use"
                aria-label=use_name
                aria-pressed=move || aria_pressed(
                    page_state::choice_is_current(
                        choice,
                        page.scope.get(),
                        selection.get().as_ref(),
                    ),
                )
                disabled=move || {
                    page.pending.get() || (!custom && !matches!(selection.get(), Some(Ok(_))))
                }
                on:click=move |_| use_card(page, selection, choice)
            >
                <span class="j-theme-swatch" aria-hidden="true"></span>
                <span class="j-theme-choice-name">{name}</span>
                <span class="j-theme-use-label" aria-hidden="true">
                    "Use"
                </span>
            </button>
            <Show when=move || editable>
                <button
                    type="button"
                    class="j-theme-edit j-theme-copy-action"
                    aria-label=copy_name.clone()
                    title="Copy saved theme; unsaved edits aren’t included"
                    on:click=move |_| copy_card(page, choice)
                >
                    <span aria-hidden="true">
                        <Icon path=Icons::COPY />
                    </span>
                </button>
                <button
                    type="button"
                    class=if custom { "j-theme-edit j-theme-catalog-item" } else { "j-theme-edit" }
                    aria-label=edit_name.clone()
                    aria-controls="theme-editor"
                    aria-expanded=move || aria_pressed(page.selected.get() == Some(choice))
                    on:click=move |_| page.select_choice(choice)
                >
                    <span aria-hidden="true">
                        <Icon path=if custom {
                            Icons::EDIT
                        } else {
                            "M2 10s3-6 8-6 8 6 8 6-3 6-8 6-8-6-8-6z M10 7a3 3 0 1 0 0 6 3 3 0 0 0 0-6"
                        } />
                    </span>
                </button>
            </Show>
        </li>
    }
}

fn use_card(page: ThemePageState, selection: ThemeSelectionResource, choice: ThemeChoice) {
    match choice {
        ThemeChoice::Custom(id) => use_target(page, id),
        _ => select_ready_theme(page, selection, choice.selection()),
    }
}

#[component]
fn ThemeReadOnlyEditor(page: ThemePageState) -> impl IntoView {
    view! {
        {move || match page.selected.get() {
            None => {
                view! { <p class="j-sub">"Choose a theme above to view its content."</p> }
                    .into_any()
            }
            Some(ThemeChoice::New | ThemeChoice::Custom(_)) => ().into_any(),
            Some(choice) => {
                view! {
                    <p class="j-form-help">
                        {match choice {
                            ThemeChoice::FollowSite => {
                                "Follows the site theme. There is no user stylesheet to edit."
                            }
                            _ => {
                                "Built-in theme · read-only. Preview its appearance, or Copy it to make your own theme."
                            }
                        }}
                    </p>
                    <button type="button" class="j-btn" disabled>
                        "Save"
                    </button>
                }
                    .into_any()
            }
        }}
    }
}

#[component]
fn ThemeSavedVersions(
    page: ThemePageState,
    catalog: ThemeCatalogResource,
    selection: ThemeSelectionResource,
) -> impl IntoView {
    let disabled = move || page.pending.get() || !matches!(selection.get(), Some(Ok(_)));
    view! {
        <Show when=move || {
            catalog
                .get()
                .is_some_and(|result| {
                    result.is_ok_and(|entries| entries.iter().any(|entry| entry.published))
                })
        }>
            <section class="j-card" aria-label="Published theme versions">
                <div class="j-form-body">
                    <label class="j-form-field">
                        <span class="j-form-label">"Use published version"</span>
                        <select
                            class="j-form-input"
                            aria-label="Use published version"
                            disabled=disabled
                            prop:value=move || {
                                page_state::saved_version_control_token(
                                    selection.get().as_ref(),
                                    catalog.get().as_ref(),
                                )
                            }
                            on:change=move |event| {
                                let token = event_target_value(&event);
                                select_ready_theme(
                                    page,
                                    selection,
                                    page_state::selection_from_token(&token),
                                );
                            }
                        >
                            <option value="" disabled>
                                "Choose a ready saved theme"
                            </option>
                            <For
                                each=move || {
                                    catalog
                                        .get()
                                        .and_then(Result::ok)
                                        .unwrap_or_default()
                                        .into_iter()
                                        .filter(|entry| entry.published)
                                        .collect::<Vec<_>>()
                                }
                                key=|entry| entry.id
                                children=move |entry| {
                                    view! {
                                        <option value=entry.id.to_string()>{entry.name}</option>
                                    }
                                }
                            />
                        </select>
                        <span class="j-form-help">
                            "Uses the published version. Unsaved and unpublished edits are not included."
                        </span>
                    </label>
                </div>
            </section>
        </Show>
    }
}

fn select_ready_theme(
    page: ThemePageState,
    selection: ThemeSelectionResource,
    next: Option<PublicThemeSelection>,
) {
    if !matches!(selection.get_untracked(), Some(Ok(_))) || !page.begin_mutation() {
        return;
    }
    let scope = page.scope.get_untracked();
    page.status.set(Some("Applying your theme…".into()));
    task::spawn_local(async move {
        let result = api::select(scope, next).await;
        set_mutation_feedback(page, MutationStage::Apply { published: false }, &result);
        reconcile_owner(scope, page.refresh, page.status, None).await;
        page.pending.set(false);
    });
}

#[component]
fn ThemeEditor(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    catalog: ThemeCatalogResource,
) -> impl IntoView {
    let captured_scope = scope.get_untracked();
    let captured_target = selected.get_untracked();
    let media_page = RwSignal::new(0_u32);
    let (presentation, owner_media) = presentation_resources(scope, selected, refresh, media_page);
    let draft = Resource::new(
        move || (),
        move |()| async move {
            match captured_target {
                Some(id) => api::get_draft(captured_scope, id).await,
                None => Ok(super::Draft {
                    manifest: Vec::new(),
                    stylesheet: Vec::new(),
                    assets: Vec::new(),
                }),
            }
        },
    );
    view! {
        <Show
            when=move || {
                matches!(page.selected.get(), Some(ThemeChoice::New | ThemeChoice::Custom(_)))
            }
            fallback=|| {
                view! {
                    <p class="j-sub">
                        "Choose a catalog entry to edit its draft, presentation, or publication."
                    </p>
                }
            }
        >
            <div class="j-theme-editor" aria-label="Selected theme editor">
                // This boundary is keyed by owner/target; Media refetches must keep its loaded form visible.
                <Transition fallback=|| {
                    view! { <p class="j-loading">"Loading stylesheet…"</p> }
                }>
                    {move || Suspend::new(async move {
                        match load_editor(
                                page,
                                captured_scope,
                                captured_target,
                                draft,
                                presentation,
                                catalog,
                            )
                            .await
                        {
                            Ok((editor, source)) => {
                                view! {
                                    <ThemeNormalEditor
                                        page=page
                                        editor=editor
                                        source=source
                                        owner_media=owner_media
                                        presentation=presentation
                                        media_page=media_page
                                    />
                                    <Show when=move || editor.target.get().is_some()>
                                        <ThemeExpertTools
                                            page=page
                                            scope=scope
                                            selected=selected
                                            refresh=refresh
                                            status=status
                                            draft=draft
                                            editor=editor
                                            presentation=presentation
                                            owner_media=owner_media
                                            media_page=media_page
                                        />
                                    </Show>
                                }
                                    .into_any()
                            }
                            Err(error) => {
                                view! {
                                    <p class="error" role="alert">
                                        {error.to_string()}
                                    </p>
                                }
                                    .into_any()
                            }
                        }
                    })}
                </Transition>
            </div>
        </Show>
    }
}

async fn load_editor(
    page: ThemePageState,
    scope: OwnershipScope,
    target: Option<ThemeId>,
    draft: Resource<Result<super::Draft, WebError>>,
    presentation: ThemePresentationResource,
    catalog: ThemeCatalogResource,
) -> Result<(EditorState, super::Draft), WebError> {
    let lifetime = page.lifetime();
    let (source, current) = load_editor_source(&lifetime, draft, presentation).await?;
    let name = load_editor_name(target, catalog).await?;
    require_editor_lifetime(&lifetime)?;
    page.loaded_editor(scope, target, name, source, current.as_ref())
}

async fn load_editor_source(
    lifetime: &std::sync::atomic::AtomicBool,
    draft: Resource<Result<super::Draft, WebError>>,
    presentation: ThemePresentationResource,
) -> Result<(super::Draft, Option<ThemePresentation>), WebError> {
    let source = draft.await?;
    require_editor_lifetime(lifetime)?;
    let current = presentation.await?;
    require_editor_lifetime(lifetime)?;
    Ok((source, current))
}

async fn load_editor_name(
    target: Option<ThemeId>,
    catalog: ThemeCatalogResource,
) -> Result<String, WebError> {
    match target {
        Some(id) => editor_catalog_name(id, &catalog.await?),
        None => Ok(String::new()),
    }
}

#[component]
fn ThemeNormalEditor(
    page: ThemePageState,
    editor: EditorState,
    source: super::Draft,
    owner_media: OwnerMediaResource,
    presentation: ThemePresentationResource,
    media_page: RwSignal<u32>,
) -> impl IntoView {
    let assets = StoredValue::new(source.assets);
    view! {
        <form
            class="j-theme-normal-form"
            aria-label="Theme editor"
            on:submit=move |event| {
                event.prevent_default();
                save_normal_editor(page, editor);
            }
        >
            <p class="j-form-help">"Update this theme’s name, style and images together."</p>
            <div class="j-theme-normal-fields">
                <label class="j-form-field j-theme-field-wide">
                    <span class="j-form-label">"Name"</span>
                    <input
                        class="j-form-input"
                        required
                        aria-label="Name"
                        disabled=move || page.pending.get()
                        aria-describedby="theme-name-help"
                        prop:value=move || editor.name.get()
                        on:input=move |event| {
                            editor.name.set(event_target_value(&event));
                            editor.changed();
                        }
                    />
                    <span class="j-form-help" id="theme-name-help">
                        "This name appears in your theme list."
                    </span>
                </label>
                <label class="j-form-field j-theme-field-wide">
                    <span class="j-form-label">"Style (CSS)"</span>
                    <textarea
                        class="j-form-input j-theme-code"
                        aria-label="Style (CSS)"
                        disabled=move || page.pending.get()
                        aria-describedby="theme-style-help"
                        prop:value=move || editor.stylesheet.get()
                        on:input=move |event| {
                            editor.stylesheet.set(event_target_value(&event));
                            editor.changed();
                        }
                        spellcheck="false"
                        rows="8"
                    ></textarea>
                    <span class="j-form-help" id="theme-style-help">
                        "Write or paste CSS. Theme files and assets stay unchanged."
                    </span>
                </label>
                <NormalImagePicker
                    editor=editor
                    role=ThemeImageRole::Logo
                    pending=page.pending
                    assets=assets
                    owner_media=owner_media
                    presentation=presentation
                />
                <NormalImagePicker
                    editor=editor
                    role=ThemeImageRole::Header
                    pending=page.pending
                    assets=assets
                    owner_media=owner_media
                    presentation=presentation
                />
            </div>
            <ThemeMediaPagination owner_media=owner_media media_page=media_page />
            {move || presentation_status(presentation, owner_media)}
            <NormalEditorPreview page=page editor=editor />
            <div class="j-form-actions j-theme-normal-actions">
                <button
                    type="button"
                    class="j-btn"
                    disabled=move || page.pending.get() || editor.preview_pending.get()
                    on:click=move |_| {
                        editor.preview.dispatch(editor.snapshot());
                    }
                >
                    "Preview"
                </button>
                <button type="submit" class="j-btn is-primary" disabled=move || page.pending.get()>
                    "Save changes"
                </button>
                <span class="j-form-help" aria-live="polite">
                    {move || {
                        if editor.input() == editor.saved.get() { "" } else { "Unsaved changes" }
                    }}
                </span>
            </div>
        </form>
    }
}

#[component]
fn NormalImagePicker(
    editor: EditorState,
    role: ThemeImageRole,
    pending: RwSignal<bool>,
    assets: StoredValue<Vec<super::ThemeAssetInput>>,
    owner_media: OwnerMediaResource,
    presentation: ThemePresentationResource,
) -> impl IntoView {
    let patch = match role {
        ThemeImageRole::Logo => editor.logo,
        ThemeImageRole::Header => editor.header,
    };
    let label = match role {
        ThemeImageRole::Logo => "Logo",
        ThemeImageRole::Header => "Header image",
    };
    view! {
        <label class="j-form-field">
            <span class="j-form-label">{label}</span>
            <select
                class="j-form-input"
                aria-label=label
                disabled=move || pending.get()
                aria-describedby=move || {
                    (role == ThemeImageRole::Header && editor.rotating.get()
                        && matches!(patch.get(), ThemeEditorPatch::Set(_)))
                        .then_some("theme-header-replacement-help")
                }
                prop:value=move || image_patch_token(&patch.get())
                on:change=move |event| stage_image(
                    editor,
                    patch,
                    &event_target_value(&event),
                    owner_media,
                )
            >
                <Show when=move || editor.target.get().is_some()>
                    <option value="keep">
                        {move || kept_image_label(
                            role,
                            presentation.get().and_then(Result::ok).flatten(),
                        )}
                    </option>
                </Show>
                <option value="package-default">"Theme default"</option>
                <option value="none">"No image"</option>
                {assets
                    .get_value()
                    .into_iter()
                    .filter(|asset| asset.mime.starts_with("image/"))
                    .map(|asset| {
                        view! {
                            <option value=format!(
                                "package-asset:{}",
                                asset.path,
                            )>{format!("Theme image: {}", asset.path)}</option>
                        }
                    })
                    .collect_view()}
                {move || {
                    let visible = image_media(owner_media)
                        .iter()
                        .map(media_input)
                        .collect::<Vec<_>>();
                    normal_media_options(&patch.get(), &visible)
                        .into_iter()
                        .map(|option| {
                            let selected = option.value == image_patch_token(&patch.get());
                            // Reused option nodes must follow the staged identity on every page rebuild.
                            view! {
                                <option value=option.value prop:selected=selected>
                                    {option.label}
                                </option>
                            }
                        })
                        .collect_view()
                }}
            </select>
            <Show when=move || {
                role == ThemeImageRole::Header && editor.rotating.get()
                    && matches!(patch.get(), ThemeEditorPatch::Set(_))
            }>
                <span class="j-form-help" id="theme-header-replacement-help">
                    "This replaces the rotating header images."
                </span>
            </Show>
        </label>
    }
}

fn stage_image(
    editor: EditorState,
    patch: RwSignal<ThemeEditorPatch<ThemeBindingInput>>,
    token: &str,
    owner_media: OwnerMediaResource,
) {
    let next = match token {
        "keep" => Some(ThemeEditorPatch::Keep),
        _ => token
            .strip_prefix("package-asset:")
            .map(|path| ThemeBindingInput::PackageAsset(path.to_owned()))
            .or_else(|| match token {
                "package-default" => Some(ThemeBindingInput::PackagedDefault),
                "none" => Some(ThemeBindingInput::ExplicitAbsent),
                _ => image_media(owner_media)
                    .iter()
                    .map(media_input)
                    .find(|media| staged_media_token(media) == token)
                    .map(ThemeBindingInput::Media),
            })
            .map(ThemeEditorPatch::Set),
    };
    if let Some(next) = next {
        patch.set(next);
        editor.changed();
    }
}

#[component]
fn NormalEditorPreview(page: ThemePageState, editor: EditorState) -> impl IntoView {
    view! {
        <section class="j-theme-editor-preview" aria-label="Current form preview">
            <Show when=move || {
                editor.preview_value.get().is_none() && !editor.preview_pending.get()
            }>
                <p class="j-form-help">
                    "Preview your current unsaved style and image choices before saving."
                </p>
            </Show>
            <Show when=move || editor.preview_pending.get()>
                <p class="j-loading" role="status">
                    "Preparing preview…"
                </p>
            </Show>
            {move || {
                editor.generation.track();
                editor.name.track();
                editor.stylesheet.track();
                editor.logo.track();
                editor.header.track();
                editor
                    .preview_value
                    .get()
                    .map(|(snapshot, result)| {
                        if page.editor_is_current(&snapshot) && editor.matches(&snapshot) {
                            match result.and_then(|preview| styled_preview_document(&preview)) {
                                Ok(document) => {
                                    view! {
                                        <div>
                                            <p class="j-form-help">
                                                {format!("Preview · {}", snapshot.input.name)}
                                            </p>
                                            <iframe
                                                class="j-theme-preview"
                                                title="Isolated theme preview"
                                                sandbox=""
                                                srcdoc=document
                                            ></iframe>
                                        </div>
                                    }
                                        .into_any()
                                }
                                Err(error) => {
                                    view! {
                                        <p class="error" role="alert">
                                            {format!("Preview failed: {error}")}
                                        </p>
                                    }
                                        .into_any()
                                }
                            }
                        } else {
                            view! {
                                <p class="j-form-help" role="status">
                                    "Preview is out of date. Preview again to see your current changes."
                                </p>
                            }
                                .into_any()
                        }
                    })
            }}
        </section>
    }
}

fn save_normal_editor(page: ThemePageState, editor: EditorState) {
    let snapshot = editor.snapshot();
    if !page.editor_is_current(&snapshot) || !page.begin_mutation() {
        return;
    }
    page.status.set(Some("Saving changes…".into()));
    let lifetime = page.lifetime();
    task::spawn_local(async move {
        let result =
            api::save_editor(snapshot.scope, snapshot.target, snapshot.input.clone()).await;
        if !lifetime.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        if !page.editor_is_current(&snapshot) {
            page.pending.set(false);
            return;
        }
        finish_normal_save(page, editor, &snapshot, &result, &lifetime).await;
    });
}

async fn finish_normal_save(
    page: ThemePageState,
    editor: EditorState,
    snapshot: &page_state::EditorSnapshot,
    result: &Result<MutationOutcome<super::ThemeEditorSaveResult>, WebError>,
    lifetime: &std::sync::atomic::AtomicBool,
) {
    if let Some(id) = page.save_feedback(editor, snapshot, result) {
        // One diagnostic reread, never adopt the uncertain ID or replay the write.
        let diagnostic = api::get_draft(snapshot.scope, id).await;
        if !lifetime.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        page.save_diagnostic(&diagnostic);
    }
    // Revalidate owner catalog/selection without inventing an uncertain target ID.
    page.refresh.notify();
    page.pending.set(false);
}

#[component]
fn ThemeExpertTools(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    draft: Resource<Result<super::Draft, WebError>>,
    editor: EditorState,
    presentation: ThemePresentationResource,
    owner_media: OwnerMediaResource,
    media_page: RwSignal<u32>,
) -> impl IntoView {
    view! {
        <details class="j-theme-tool-group j-theme-expert-tools">
            <summary>"Theme tools"</summary>
            <div class="j-theme-tool-body">
                <p class="j-form-help">
                    "Separate actions for rotating images, theme files and publication. These tools are not part of Save changes."
                </p>
                <details class="j-theme-tool-group">
                    <summary>"Images"</summary>
                    <div class="j-theme-tool-body">
                        <ThemePresentationEditor
                            page=page
                            scope=scope
                            selected=selected
                            refresh=refresh
                            status=status
                            presentation=presentation
                            owner_media=owner_media
                            media_page=media_page
                        />
                    </div>
                </details>
                <details class="j-theme-tool-group">
                    <summary>"Theme Package (developer tools)"</summary>
                    <div class="j-theme-tool-body">
                        <p class="j-form-help">
                            "Developer tools: Save complete draft package saves the manifest, shared CSS and assets separately. It does not publish or Use the theme."
                        </p>
                        <a
                            href="https://github.com/jaunder-org/jaunder/blob/main/docs/themes.md"
                            target="_blank"
                        >
                            "Theme Package authoring documentation"
                        </a>
                        <DraftPackageEditor
                            page=page
                            scope=scope
                            selected=selected
                            refresh=refresh
                            status=status
                            draft=draft
                            stylesheet_source=editor.stylesheet
                        />
                    </div>
                </details>
                <details class="j-theme-tool-group">
                    <summary>"Manage Theme"</summary>
                    <div class="j-theme-tool-body j-theme-actions">
                        <button
                            type="button"
                            class="j-btn"
                            on:click=move |_| export_selected(page, scope, selected, status)
                        >
                            "Export ZIP"
                        </button>
                        <button
                            type="button"
                            class="j-btn"
                            on:click=move |_| publish_selected(
                                page,
                                scope,
                                selected,
                                refresh,
                                status,
                            )
                        >
                            "Publish"
                        </button>
                        <button
                            type="button"
                            class="j-btn is-danger"
                            on:click=move |_| remove_selected(
                                page,
                                scope,
                                selected,
                                refresh,
                                status,
                            )
                        >
                            "Delete theme"
                        </button>
                    </div>
                </details>
            </div>
        </details>
    }
}

/// Reuse the host-resolved immutable application asset, not a guessed path or
/// the private document's computed styles. Missing shell metadata is an error.
fn styled_preview_document(preview: &super::ThemePreview) -> Result<String, WebError> {
    let document = leptos::prelude::document();
    let link = document
        .query_selector("head link[data-jaunder-application-stylesheet]")
        .map_err(|_| WebError::server_message("Unable to read application stylesheet"))?
        .ok_or_else(|| WebError::server_message("Application stylesheet is missing"))?;
    let href = link
        .get_attribute("href")
        .ok_or_else(|| WebError::server_message("Application stylesheet URL is missing"))?;
    let url = href
        .parse::<common::root_relative_url::RootRelativeUrl>()
        .map_err(|_| WebError::server_message("Application stylesheet URL is invalid"))?;
    Ok(preview_document(&preview.css, &preview.html, &url))
}

/// Load structure before theme CSS, as in the public shell. Stylesheet bytes
/// cannot terminate the raw-text element; `\3C ` escapes `<` in CSS strings too.
fn preview_document(
    css: &str,
    html: &str,
    application_stylesheet_url: &common::root_relative_url::RootRelativeUrl,
) -> String {
    let base = maud::html! {
        link rel="stylesheet" href=(application_stylesheet_url) data-jaunder-application-stylesheet;
    }
    .into_string();
    format!(
        "<!doctype html><html><head>{base}<style>{}</style></head><body>{html}</body></html>",
        html_style_text(css)
    )
}

fn html_style_text(css: &str) -> Cow<'_, str> {
    const STYLE_END: &str = "</style";
    let is_style_end = |offset| {
        css[offset..]
            .get(..STYLE_END.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(STYLE_END))
    };
    let Some(first) = css
        .match_indices('<')
        .map(|(offset, _)| offset)
        .find(|&offset| is_style_end(offset))
    else {
        return Cow::Borrowed(css);
    };

    let mut escaped = String::with_capacity(css.len());
    let mut copied_through = 0;
    let mut next = first;
    loop {
        escaped.push_str(&css[copied_through..next]);
        escaped.push_str(r"\3C ");
        escaped.push_str(&css[next + 1..next + STYLE_END.len()]);
        copied_through = next + STYLE_END.len();

        let Some(offset) = css[copied_through..]
            .match_indices('<')
            .map(|(offset, _)| copied_through + offset)
            .find(|&offset| is_style_end(offset))
        else {
            escaped.push_str(&css[copied_through..]);
            return Cow::Owned(escaped);
        };
        next = offset;
    }
}

fn export_selected(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    status: RwSignal<Option<String>>,
) {
    // crap:allow: Browser download initiation is wasm-only; host coverage cannot execute the export handler.
    if let Some(id) = selected.get_untracked() {
        let scope = scope.get_untracked();
        spawn_theme_operation(page, async move {
            match api::export(scope, id).await {
                Ok(package) => match download_package(&package.filename, &package.bytes) {
                    Ok(()) => {
                        page.feedback.set(Feedback::Success);
                        status.set(Some("Theme Package download started.".into()));
                    }
                    Err(message) => {
                        page.feedback.set(Feedback::Error);
                        status.set(Some(format!("Download could not start: {message}. Try exporting again after checking browser downloads.")));
                    }
                },
                Err(error) => {
                    page.feedback.set(Feedback::Error);
                    status.set(Some(format!("Export failed: {error}. Reopen Themes to review the draft before exporting again.")));
                }
            }
        });
    }
}

#[component]
fn ThemeFeedback(status: RwSignal<Option<String>>, feedback: RwSignal<Feedback>) -> impl IntoView {
    view! {
        <Show when=move || status.get().is_some()>
            <p
                class="j-theme-status"
                class:success=move || feedback.get() == Feedback::Success
                class:error=move || feedback.get() == Feedback::Error
                role="status"
                aria-live="polite"
                aria-atomic="true"
            >
                {move || status.get()}
            </p>
        </Show>
    }
}

fn set_mutation_feedback<T>(
    page: ThemePageState,
    stage: MutationStage,
    result: &Result<MutationOutcome<T>, WebError>,
) {
    let receipt = page_state::mutation_receipt(result);
    page.feedback
        .set(page_state::mutation_feedback(stage, &receipt));
    page.status
        .set(Some(page_state::mutation_message(stage, &receipt)));
}

// Browser scheduling only: the admission decision lives in the host-tested page state.
fn spawn_theme_operation(
    page: ThemePageState,
    future: impl std::future::Future<Output = ()> + 'static,
) {
    if !page.begin_mutation() {
        return;
    }
    page.status.set(Some("Working…".into()));
    let lifetime = page.lifetime();
    task::spawn_local(async move {
        future.await;
        if lifetime.load(std::sync::atomic::Ordering::Acquire) {
            page.pending.set(false);
        }
    });
}

fn use_target(page: ThemePageState, id: ThemeId) {
    if page.begin_mutation() {
        dispatch_use(page, page.scope.get_untracked(), id);
    }
}

fn dispatch_use(page: ThemePageState, scope: OwnershipScope, id: ThemeId) {
    page.status.set(Some("Getting your theme ready…".into()));
    let lifetime = page.lifetime();
    task::spawn_local(async move {
        let published = api::publish(scope, id).await;
        if !lifetime.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        set_mutation_feedback(page, MutationStage::Publish, &published);
        if page_state::publication_confirmed(&published) {
            let applied = api::select(scope, Some(PublicThemeSelection::Custom(id))).await;
            if !lifetime.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            set_mutation_feedback(page, MutationStage::Apply { published: true }, &applied);
        }
        reconcile_live_owner(page, scope, id, &lifetime).await;
    });
}

/// Each reread belongs to the mounted page; sent writes may still commit after teardown.
async fn reconcile_live_owner(
    page: ThemePageState,
    scope: OwnershipScope,
    id: ThemeId,
    lifetime: &std::sync::atomic::AtomicBool,
) {
    let catalog = api::list(scope).await;
    if !lifetime.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let selection = api::get_selection(scope).await;
    if !lifetime.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let message_from_rereads =
        page_state::reconciliation_message(scope, &catalog, &selection, Some(id));
    page.status.update(|message| {
        if let Some(message) = message {
            message.push(' ');
            message.push_str(&message_from_rereads);
        }
    });
    page.refresh.notify();
    page.pending.set(false);
}

async fn reconcile_owner(
    scope: OwnershipScope,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    target: Option<ThemeId>,
) {
    let catalog = api::list(scope).await;
    let selection = api::get_selection(scope).await;
    let message_from_rereads =
        page_state::reconciliation_message(scope, &catalog, &selection, target);
    status.update(|message| {
        if let Some(message) = message {
            message.push(' ');
            message.push_str(&message_from_rereads);
        }
    });
    refresh.notify();
}

fn publish_selected(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    if let Some(id) = selected.get_untracked() {
        let scope = scope.get_untracked();
        spawn_theme_operation(page, async move {
            status.set(Some("Publishing theme…".into()));
            let result = api::publish(scope, id).await;
            set_mutation_feedback(page, MutationStage::PublishOnly, &result);
            reconcile_owner(scope, refresh, status, Some(id)).await;
        });
    }
}

fn remove_selected(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    if let Some(id) = selected.get_untracked() {
        let scope = scope.get_untracked();
        spawn_theme_operation(page, async move {
            page.selected.set(None);
            settle(page, api::remove(scope, id).await, refresh, status);
        });
    }
}

#[component]
fn DraftPackageEditor(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    draft: Resource<Result<super::Draft, WebError>>,
    stylesheet_source: RwSignal<String>,
) -> impl IntoView {
    let draft_resource = draft;
    view! {
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading draft package…"</p> }
        }>
            {move || Suspend::new(async move {
                match draft.await {
                    Ok(draft) => {
                        view! {
                            <DraftPackageForm
                                page=page
                                scope=scope
                                selected=selected
                                draft=draft
                                draft_resource=draft_resource
                                refresh=refresh
                                status=status
                                stylesheet_source=stylesheet_source
                            />
                        }
                            .into_any()
                    }
                    Err(error) => {
                        view! {
                            <p class="error" role="alert">
                                {error.to_string()}
                            </p>
                        }
                            .into_any()
                    }
                }
            })}
        </Suspense>
    }
}

#[component]
fn DraftPackageForm(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    draft: super::Draft,
    draft_resource: Resource<Result<super::Draft, WebError>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    stylesheet_source: RwSignal<String>,
) -> impl IntoView {
    let manifest = NodeRef::<leptos::html::Textarea>::new();
    let assets_editor = NodeRef::<leptos::html::Textarea>::new();
    let (manifest_text, assets_text) = draft_package_editor_text(&draft);
    let save_package = move |_| {
        let Some(id) = selected.get_untracked() else {
            return;
        };
        let manifest = manifest
            .get()
            .map(|input| input.value())
            .unwrap_or_default();
        let stylesheet = stylesheet_source.get_untracked();
        let assets = assets_editor
            .get()
            .map(|input| input.value())
            .unwrap_or_default();
        match page_state::draft_from_editor(manifest, stylesheet, &assets) {
            Ok(package) => {
                dispatch_package_save(
                    page,
                    scope.get_untracked(),
                    id,
                    package,
                    refresh,
                    status,
                    draft_resource,
                );
            }
            Err(error) => {
                page.feedback_in_import.set(false);
                page.feedback.set(Feedback::Error);
                status.set(Some(error));
            }
        }
    };
    view! {
        <fieldset class="j-theme-presentation">
            <legend>"Complete draft package"</legend>
            <p class="j-form-help">
                "Edit package defaults in the manifest and add, remove, or edit asset path, MIME type, and bytes in the lossless JSON asset editor."
            </p>
            <label class="j-form-field">
                <span class="j-form-label">"theme.json"</span>
                <textarea
                    class="j-form-input j-theme-code"
                    node_ref=manifest
                    prop:value=manifest_text
                    spellcheck="false"
                ></textarea>
            </label>
            <label class="j-form-field">
                <span class="j-form-label">"style.css"</span>
                <textarea
                    class="j-form-input j-theme-code"
                    prop:value=move || stylesheet_source.get()
                    on:input=move |event| stylesheet_source.set(event_target_value(&event))
                    spellcheck="false"
                ></textarea>
            </label>
            <label class="j-form-field">
                <span class="j-form-label">"Package assets JSON"</span>
                <textarea
                    class="j-form-input j-theme-code"
                    node_ref=assets_editor
                    prop:value=assets_text
                    spellcheck="false"
                ></textarea>
            </label>
            <button type="button" class="j-btn" on:click=save_package>
                "Save complete draft package"
            </button>
        </fieldset>
    }
}

// Expert package persistence is separate from normal Save and never publishes or selects.
fn dispatch_package_save(
    page: ThemePageState,
    scope: OwnershipScope,
    id: ThemeId,
    package: super::Draft,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    draft_resource: Resource<Result<super::Draft, WebError>>,
) {
    let lifetime = page.lifetime();
    spawn_theme_operation(page, async move {
        let result = api::import_package(scope, id, package).await;
        if !lifetime.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let context_changed = result.is_ok();
        settle_context(page, scope, id, result, refresh, status);
        if context_changed {
            // Reload asset choices/defaults, never reseed the cached normal buffer.
            draft_resource.refetch();
        }
    });
}

#[component]
fn ThemePresentationEditor(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    presentation: ThemePresentationResource,
    owner_media: OwnerMediaResource,
    media_page: RwSignal<u32>,
) -> impl IntoView {
    let header_media_draft = RwSignal::new(HeaderMediaDraft::default());
    Effect::new(move |_| {
        let current = presentation.get().and_then(Result::ok).flatten();
        header_media_draft.update(|draft| draft.sync(selected.get(), current.as_ref()));
    });
    view! {
        <fieldset class="j-theme-presentation">
            <legend>"Rotating header images"</legend>
            <p class="j-form-help">
                "Save header pool and Shuffle assignments are separate explicit actions. Use the normal form above for Logo and Header image choices."
            </p>
            <ThemeMediaPagination owner_media=owner_media media_page=media_page />
            <ThemeHeaderPoolEditor
                page=page
                scope=scope
                selected=selected
                refresh=refresh
                status=status
                presentation=presentation
                owner_media=owner_media
                header_media_draft=header_media_draft
            />
        </fieldset>
    }
}

#[component]
fn ThemeMediaPagination(
    owner_media: OwnerMediaResource,
    media_page: RwSignal<u32>,
) -> impl IntoView {
    let previous = move |_| media_page.update(|page| *page = page.saturating_sub(1));
    let next = move |_| media_page.update(|page| *page = page.saturating_add(1));
    view! {
        <div class="j-theme-actions" aria-label="Uploaded image pages">
            <button
                type="button"
                class="j-btn"
                disabled=move || media_page.get() == 0
                on:click=previous
            >
                "Previous images"
            </button>
            <span aria-live="polite">{move || format!("Image page {}", media_page.get() + 1)}</span>
            <Show when=move || owner_media.get().is_none()>
                <span role="status">"Loading images…"</span>
            </Show>
            <button
                type="button"
                class="j-btn"
                disabled=move || {
                    owner_media
                        .get()
                        .and_then(Result::ok)
                        .is_none_or(|items| items.len() < MEDIA_PAGE_SIZE)
                }
                on:click=next
            >
                "Next images"
            </button>
        </div>
    }
}

#[component]
fn ThemeHeaderPoolEditor(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    presentation: ThemePresentationResource,
    owner_media: OwnerMediaResource,
    header_media_draft: RwSignal<HeaderMediaDraft>,
) -> impl IntoView {
    view! {
        <label class="j-form-field">
            <span class="j-form-label">"Header pool package asset paths"</span>
            <textarea
                class="j-form-input j-theme-code"
                prop:value=move || header_media_draft.get().package_paths().to_owned()
                on:input=move |event| {
                    header_media_draft
                        .update(|draft| draft.set_package_paths(event_target_value(&event)));
                }
            ></textarea>
        </label>
        <ThemeHeaderMediaEditor owner_media=owner_media header_media_draft=header_media_draft />
        {move || presentation_status(presentation, owner_media)}
        <ThemeHeaderPoolActions
            page=page
            scope=scope
            selected=selected
            refresh=refresh
            status=status
            header_media_draft=header_media_draft
        />
    }
}

#[component]
fn ThemeHeaderMediaEditor(
    owner_media: OwnerMediaResource,
    header_media_draft: RwSignal<HeaderMediaDraft>,
) -> impl IntoView {
    let media_picker = NodeRef::<leptos::html::Select>::new();
    let add_header_media = move |_| {
        let token = media_picker
            .get()
            .map(|select| select.value())
            .unwrap_or_default();
        if let Some(media) = selected_media(&token, owner_media) {
            header_media_draft.update(|draft| draft.add(media));
        }
    };
    view! {
        <div class="j-form-field">
            <span class="j-form-label">"Header pool Media"</span>
            <div class="j-theme-actions">
                <select class="j-form-input" node_ref=media_picker aria-label="Media to add">
                    <option value="">"Choose uploaded image"</option>
                    {move || media_options(owner_media, false)}
                </select>
                <button type="button" class="j-btn" on:click=add_header_media>
                    "Add Media"
                </button>
            </div>
            <ul class="j-theme-media-pool" aria-label="Selected header Media">
                <For
                    each=move || header_media_draft.get().entries().to_vec()
                    key=media_key
                    children=move |entry| {
                        let remove = entry.clone();
                        view! {
                            <li>
                                <span>{entry.filename.to_string()}</span>
                                <button
                                    type="button"
                                    class="j-btn"
                                    aria-label=format!("Remove {} from header pool", entry.filename)
                                    on:click=move |_| {
                                        header_media_draft.update(|draft| draft.remove(&remove));
                                    }
                                >
                                    "Remove"
                                </button>
                            </li>
                        }
                    }
                />
            </ul>
        </div>
    }
}

#[component]
fn ThemeHeaderPoolActions(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    header_media_draft: RwSignal<HeaderMediaDraft>,
) -> impl IntoView {
    let pool = move |_| {
        let draft = header_media_draft.get_untracked();
        update_pool(
            page,
            scope,
            selected,
            refresh,
            status,
            draft.package_paths(),
            draft.entries().to_vec(),
        );
    };
    let shuffle = move |_| shuffle_presentation(page, scope, selected, refresh, status);
    view! {
        <button type="button" class="j-btn" on:click=pool>
            "Save header pool"
        </button>
        <button type="button" class="j-btn" on:click=shuffle>
            "Shuffle assignments"
        </button>
    }
}

fn selected_media(token: &str, owner_media: OwnerMediaResource) -> Option<ThemeMediaInput> {
    image_media(owner_media)
        .into_iter()
        .find(|item| item.url.as_ref() == token)
        .map(|item| media_input(&item))
}

fn media_options(owner_media: OwnerMediaResource, prefix_label: bool) -> Vec<AnyView> {
    image_media(owner_media)
        .into_iter()
        .map(|item| {
            let value = item.url.to_string();
            let label = if prefix_label {
                format!("Media: {}", item.filename)
            } else {
                item.filename.to_string()
            };
            view! { <option value=value>{label}</option> }.into_any()
        })
        .collect()
}

fn image_media(owner_media: OwnerMediaResource) -> Vec<MediaItem> {
    owner_media
        .get()
        .and_then(Result::ok)
        .unwrap_or_default()
        .into_iter()
        .filter(|item| item.content_type.as_ref().starts_with("image/"))
        .collect()
}

fn media_input(item: &MediaItem) -> ThemeMediaInput {
    ThemeMediaInput {
        source: item.source,
        sha256: item.sha256.clone(),
        filename: item.filename.clone(),
    }
}

fn media_key(media: &ThemeMediaInput) -> String {
    format!("{}:{}:{}", media.source, media.sha256, media.filename)
}

fn presentation_status(
    presentation: ThemePresentationResource,
    owner_media: OwnerMediaResource,
) -> Option<impl IntoView> {
    if let Some(Err(error)) = presentation.get() {
        return Some(view! {
            <p class="error" role="alert">
                {error.to_string()}
            </p>
        });
    }
    if let Some(Err(error)) = owner_media.get() {
        return Some(view! {
            <p class="error" role="alert">
                {error.to_string()}
            </p>
        });
    }
    None
}

fn update_pool(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    paths: &str,
    media: Vec<ThemeMediaInput>,
) {
    let Some(id) = selected.get_untracked() else {
        return;
    };
    let entries = paths
        .lines()
        .filter(|path| !path.trim().is_empty())
        .map(|path| ThemePoolInput::PackageAsset(path.trim().to_owned()))
        .chain(media.into_iter().map(ThemePoolInput::Media))
        .collect();
    let current_scope = scope.get_untracked();
    let seed = match fresh_seed() {
        Ok(seed) => seed,
        Err(error) => {
            page.feedback_in_import.set(false);
            page.feedback.set(Feedback::Error);
            status.set(Some(error));
            return;
        }
    };
    let lifetime = page.lifetime();
    spawn_theme_operation(page, async move {
        let result = api::replace_pool(current_scope, id, entries, seed).await;
        if lifetime.load(std::sync::atomic::Ordering::Acquire) {
            settle_context(page, current_scope, id, result, refresh, status);
        }
    });
}

fn shuffle_presentation(
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    selected: Signal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    let Some(id) = selected.get_untracked() else {
        return;
    };
    let current_scope = scope.get_untracked();
    let seed = match fresh_seed() {
        Ok(seed) => seed,
        Err(error) => {
            page.feedback_in_import.set(false);
            page.feedback.set(Feedback::Error);
            status.set(Some(error));
            return;
        }
    };
    let lifetime = page.lifetime();
    spawn_theme_operation(page, async move {
        let result = api::shuffle(current_scope, id, seed).await;
        if lifetime.load(std::sync::atomic::Ordering::Acquire) {
            settle_context(page, current_scope, id, result, refresh, status);
        }
    });
}

fn fresh_seed() -> Result<[u8; 32], String> {
    let window = leptos::web_sys::window()
        .ok_or_else(|| "Could not access browser entropy for the shuffle.".to_owned())?;
    let crypto = window
        .crypto()
        .map_err(|_| "Could not access browser entropy for the shuffle.".to_owned())?;
    let mut seed = [0; 32];
    crypto
        .get_random_values_with_u8_array(&mut seed)
        .map_err(|_| "Could not generate browser entropy for the shuffle.".to_owned())?;
    Ok(seed)
}

fn download_package(filename: &str, bytes: &[u8]) -> Result<(), String> {
    // crap:allow: Browser Blob, URL, and anchor APIs are wasm-only; host coverage cannot execute package downloads.
    use wasm_bindgen::JsCast;

    let values = js_sys::Array::new();
    values.push(&js_sys::Uint8Array::from(bytes).into());
    let blob = leptos::web_sys::Blob::new_with_u8_array_sequence(&values)
        .map_err(|_| "Could not create the package download.".to_owned())?;
    let url = leptos::web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|_| "Could not create the package download.".to_owned())?;
    let document = leptos::web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| "Could not start the package download.".to_owned())?;
    let anchor = document
        .create_element("a")
        .map_err(|_| "Could not start the package download.".to_owned())?
        .dyn_into::<leptos::web_sys::HtmlAnchorElement>()
        .map_err(|_| "Could not start the package download.".to_owned())?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    anchor.click();
    leptos::web_sys::Url::revoke_object_url(&url)
        .map_err(|_| "Could not finish the package download.".to_owned())
}

fn built_in_themes() -> impl Iterator<Item = Theme> {
    Theme::VARIANTS.iter().copied()
}

fn aria_pressed(pressed: bool) -> &'static str {
    if pressed { "true" } else { "false" }
}

#[cfg(test)]
mod tests {
    use super::{html_style_text, preview_document};

    #[test]
    fn preview_stylesheet_cannot_close_its_html_style_element() {
        let css = r#"
.preview::after { content: "</StYlE><img src=https://example.invalid>"; }
.preview { color: rebeccapurple; }
/* </STYLE><img src=https://example.invalid> */
"#;
        let url = "/theme/application?x=1&y=2".parse().unwrap();
        let document = preview_document(css, r#"<main class="preview">Preview</main>"#, &url);
        assert!(document.contains(
            r#"<head><link rel="stylesheet" href="/theme/application?x=1&amp;y=2" data-jaunder-application-stylesheet><style>"#
        ));
        let escaped_css = html_style_text(css);

        assert_eq!(
            escaped_css,
            r#"
.preview::after { content: "\3C /StYlE><img src=https://example.invalid>"; }
.preview { color: rebeccapurple; }
/* \3C /STYLE><img src=https://example.invalid> */
"#
        );
        assert!(
            document.contains(
                r#"<style>
.preview::after { content: "\3C /StYlE>"#
            ),
            "{document}"
        );
        assert!(!document.contains("</StYlE>"), "{document}");
        assert!(!document.contains("</STYLE>"), "{document}");
        assert!(
            document.contains(".preview { color: rebeccapurple; }"),
            "{document}"
        );
        assert!(
            document.ends_with(
                r#"</style></head><body><main class="preview">Preview</main></body></html>"#
            ),
            "{document}"
        );
    }
}
