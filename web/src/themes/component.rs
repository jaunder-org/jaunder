//! Wasm-only private Studio journey for Theme Package management.

use std::str::FromStr;

use common::{
    MutationOutcome,
    ids::ThemeId,
    theme::{PublicThemeSelection, Theme, ThemeImageRole},
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use strum::VariantArray;

use crate::{auth, error::WebError, reactive::Invalidator, topbar::Topbar};

use super::page_state::{Revalidation, ScopeAvailability, ThemePageState, draft_from_editor};
use super::{CatalogEntry, OwnershipScope, ThemeBindingInput, ThemePoolInput, api, revalidation};

/// The authenticated Studio route. It deliberately only renders server-provided preview
/// bytes inside a sandboxed iframe; no draft CSS is ever inserted into Studio's document.
#[component]
pub fn ThemesPage() -> impl IntoView {
    let session = auth::use_session().current;
    let page = ThemePageState::default();
    let scope = page.scope;
    let selected = page.selected;
    let refresh = page.refresh;
    let status = page.status;
    let (catalog, selection) = owner_resources(scope, refresh);
    let rename_name = NodeRef::<leptos::html::Input>::new();

    view! {
        <Topbar title="Themes" sub="Public presentation packages" />
        <div class="j-page j-themes-page" data-theme-management="studio">
            {move || match session.get() {
                None => {
                    view! {
                        <p class="error" role="alert">
                            "Sign in to manage themes."
                        </p>
                    }
                        .into_any()
                }
                Some(user) => {
                    view! {
                        {theme_management_controls(
                            ScopeAvailability::from_session(true, user.is_operator),
                            page,
                            scope,
                            refresh,
                            status,
                        )}
                        <ThemeCatalog
                            catalog=catalog
                            selection=selection
                            scope=scope
                            selected=selected
                            refresh=refresh
                            status=status
                            rename_name=rename_name
                        />
                        {move || {
                            status
                                .get()
                                .map(|message| {
                                    view! {
                                        <p class="j-theme-status" role="status" aria-live="polite">
                                            {message}
                                        </p>
                                    }
                                })
                        }}
                    }
                        .into_any()
                }
            }}
        </div>
    }
}
type ThemeCatalogResource = Resource<Result<Vec<CatalogEntry>, WebError>>;

fn theme_management_controls(
    availability: ScopeAvailability,
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) -> impl IntoView {
    let create_name = NodeRef::<leptos::html::Input>::new();
    let css_name = NodeRef::<leptos::html::Input>::new();
    let css_source = NodeRef::<leptos::html::Textarea>::new();
    let package_name = NodeRef::<leptos::html::Input>::new();
    let package_file = NodeRef::<leptos::html::Input>::new();
    view! {
        {scope_card(availability, page, scope)}
        {create_theme_card(create_name, scope, refresh, status)}
        {css_import_card(css_name, css_source, scope, refresh, status)}
        {zip_import_card(package_name, package_file, scope, refresh, status)}
    }
}

fn scope_card(
    availability: ScopeAvailability,
    page: ThemePageState,
    scope: RwSignal<OwnershipScope>,
) -> impl IntoView {
    view! {
        <section class="j-card j-theme-scope" aria-labelledby="theme-scope-heading">
            <div class="j-card-head">
                <div>
                    <h2 id="theme-scope-heading">"Catalog scope"</h2>
                    <div class="j-sub">
                        "Your author catalog is always available. Site changes remain operator-authorized by the server."
                    </div>
                </div>
            </div>
            <div
                class="j-form-body j-theme-scope-options"
                role="radiogroup"
                aria-label="Theme catalog scope"
            >
                <button
                    type="button"
                    class="j-btn"
                    aria-pressed=move || aria_pressed(scope.get() == OwnershipScope::Author)
                    on:click=move |_| page.select_scope(OwnershipScope::Author)
                >
                    "Author catalog"
                </button>
                {availability
                    .permits(OwnershipScope::Site)
                    .then(|| {
                        view! {
                            <button
                                type="button"
                                class="j-btn"
                                aria-pressed=move || {
                                    aria_pressed(scope.get() == OwnershipScope::Site)
                                }
                                on:click=move |_| page.select_scope(OwnershipScope::Site)
                            >
                                "Site catalog"
                            </button>
                        }
                    })}
            </div>
        </section>
    }
}

fn create_theme_card(
    name: NodeRef<leptos::html::Input>,
    scope: RwSignal<OwnershipScope>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <section class="j-card" aria-labelledby="theme-create-heading">
            <div class="j-card-head">
                <div>
                    <h2 id="theme-create-heading">"Create a theme"</h2>
                    <div class="j-sub">
                        "Create a private draft from package source, or import plain CSS below."
                    </div>
                </div>
            </div>
            <div class="j-form-body">
                <label class="j-form-field">
                    <span class="j-form-label">"Theme name"</span>
                    <input class="j-form-input" node_ref=name />
                </label>
                <button
                    type="button"
                    class="j-btn is-primary"
                    on:click=move |_| {
                        let name = name.get().map(|input| input.value()).unwrap_or_default();
                        let scope = scope.get_untracked();
                        spawn_local(async move {
                            let result = api::import_css(scope, name, Vec::new()).await;
                            settle(result, refresh, status);
                        });
                    }
                >
                    "Create empty draft"
                </button>
            </div>
        </section>
    }
}

fn css_import_card(
    name: NodeRef<leptos::html::Input>,
    stylesheet: NodeRef<leptos::html::Textarea>,
    scope: RwSignal<OwnershipScope>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <section class="j-card" aria-labelledby="theme-import-heading">
            <div class="j-card-head">
                <div>
                    <h2 id="theme-import-heading">"Import CSS"</h2>
                    <div class="j-sub">
                        "A CSS import creates an unselected, zero-asset private draft."
                    </div>
                </div>
            </div>
            <div class="j-form-body">
                <label class="j-form-field">
                    <span class="j-form-label">"Theme name"</span>
                    <input class="j-form-input" node_ref=name />
                </label>
                <label class="j-form-field">
                    <span class="j-form-label">"Stylesheet"</span>
                    <textarea
                        class="j-form-input j-theme-code"
                        node_ref=stylesheet
                        spellcheck="false"
                    ></textarea>
                </label>
                <button
                    type="button"
                    class="j-btn"
                    on:click=move |_| {
                        let name = name.get().map(|input| input.value()).unwrap_or_default();
                        let stylesheet = stylesheet
                            .get()
                            .map(|input| input.value().into_bytes())
                            .unwrap_or_default();
                        let scope = scope.get_untracked();
                        spawn_local(async move {
                            settle(api::import_css(scope, name, stylesheet).await, refresh, status);
                        });
                    }
                >
                    "Import CSS draft"
                </button>
            </div>
        </section>
    }
}

fn zip_import_card(
    name: NodeRef<leptos::html::Input>,
    file: NodeRef<leptos::html::Input>,
    scope: RwSignal<OwnershipScope>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <section class="j-card" aria-labelledby="theme-package-import-heading">
            <div class="j-card-head">
                <div>
                    <h2 id="theme-package-import-heading">"Import Theme Package"</h2>
                    <div class="j-sub">
                        "Import a ZIP package as a new private, unselected draft."
                    </div>
                </div>
            </div>
            <div class="j-form-body">
                <label class="j-form-field">
                    <span class="j-form-label">"Theme name"</span>
                    <input class="j-form-input" node_ref=name />
                </label>
                <label class="j-form-field">
                    <span class="j-form-label">"Theme Package ZIP"</span>
                    <input
                        class="j-form-input"
                        type="file"
                        accept=".zip,application/zip"
                        node_ref=file
                    />
                </label>
                <button
                    type="button"
                    class="j-btn"
                    on:click=move |_| import_package_zip(file, name, scope, refresh, status)
                >
                    "Import ZIP draft"
                </button>
            </div>
        </section>
    }
}

type ThemeSelectionResource = Resource<Result<Option<PublicThemeSelection>, WebError>>;

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

fn import_package_zip(
    package_file: NodeRef<leptos::html::Input>,
    package_name: NodeRef<leptos::html::Input>,
    scope: RwSignal<OwnershipScope>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    let Some(file) = package_file
        .get()
        .and_then(|input| input.files())
        .and_then(|files| files.get(0))
    else {
        status.set(Some("Choose a Theme Package ZIP file first.".into()));
        return;
    };
    let name = package_name
        .get()
        .map(|input| input.value())
        .unwrap_or_default();
    let scope_name = match scope.get_untracked() {
        OwnershipScope::Author => "author",
        OwnershipScope::Site => "site",
    };
    let Ok(form) = leptos::web_sys::FormData::new() else {
        status.set(Some("Could not prepare the package upload.".into()));
        return;
    };
    if form.append_with_str("scope", scope_name).is_err()
        || form.append_with_str("name", &name).is_err()
        || form.append_with_blob("archive", &file).is_err()
    {
        status.set(Some("Could not prepare the package upload.".into()));
        return;
    }
    spawn_local(async move { settle(api::import_zip(form.into()).await, refresh, status) });
}

fn settle<T>(
    result: Result<MutationOutcome<T>, WebError>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    match revalidation(result) {
        Revalidation::Confirmed => {
            refresh.notify();
            status.set(Some("Saved. Catalog state was reloaded.".into()));
        }
        Revalidation::Indeterminate => {
            refresh.notify();
            status.set(Some("The server could not confirm this change. Catalog state was reloaded; review it before continuing.".into()));
        }
        Revalidation::Failed(message) => status.set(Some(message)),
    }
}

#[component]
fn ThemeCatalog(
    catalog: Resource<Result<Vec<CatalogEntry>, WebError>>,
    selection: Resource<Result<Option<PublicThemeSelection>, WebError>>,
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    rename_name: NodeRef<leptos::html::Input>,
) -> impl IntoView {
    view! {
        <section class="j-card" aria-labelledby="theme-catalog-heading">
            <div class="j-card-head">
                <div>
                    <h2 id="theme-catalog-heading">"Catalog and selection"</h2>
                    <div class="j-sub">
                        "Only published custom themes can be selected. Authors may instead inherit the site selection."
                    </div>
                </div>
            </div>
            <div class="j-form-body">
                <ThemeSelection
                    selection=selection
                    catalog=catalog
                    scope=scope
                    refresh=refresh
                    status=status
                />
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading catalog…"</p> }
                }>
                    {move || Suspend::new(async move {
                        match catalog.await {
                            Ok(entries) => {
                                view! {
                                    <ul class="j-theme-catalog" aria-label="Theme catalog">
                                        <For
                                            each=move || entries.clone()
                                            key=|entry| entry.id
                                            children=move |entry| {
                                                let entry_id = entry.id;
                                                view! {
                                                    <li>
                                                        <button
                                                            type="button"
                                                            class="j-theme-catalog-item"
                                                            aria-pressed=move || {
                                                                aria_pressed(selected.get() == Some(entry_id))
                                                            }
                                                            on:click=move |_| selected.set(Some(entry_id))
                                                        >
                                                            {entry.name}
                                                            {if entry.published { " (published)" } else { " (draft)" }}
                                                        </button>
                                                    </li>
                                                }
                                            }
                                        />
                                    </ul>
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
                <ThemeEditor
                    scope=scope
                    selected=selected
                    refresh=refresh
                    status=status
                    rename_name=rename_name
                />
            </div>
        </section>
    }
}

#[component]
fn ThemeSelection(
    catalog: Resource<Result<Vec<CatalogEntry>, WebError>>,
    selection: Resource<Result<Option<PublicThemeSelection>, WebError>>,
    scope: RwSignal<OwnershipScope>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) -> impl IntoView {
    let selected_token =
        Memo::new(move |_| selection_token(selection.get().and_then(Result::ok).flatten()));
    let change = move |event| {
        let token = event_target_value(&event);
        let current_scope = scope.get_untracked();
        let next = if token == "inherit" {
            None
        } else if let Ok(theme) = Theme::from_str(&token) {
            Some(PublicThemeSelection::BuiltIn(theme))
        } else {
            ThemeId::from_str(&token)
                .ok()
                .map(PublicThemeSelection::Custom)
        };
        spawn_local(async move { settle(api::select(current_scope, next).await, refresh, status) });
    };
    view! {
        <div class="j-theme-selection">
            <label class="j-form-field">
                <span class="j-form-label">"Public selection"</span>
                <select class="j-form-input" prop:value=selected_token on:change=change>
                    {move || {
                        let selected_value = selection.get().and_then(Result::ok).flatten();
                        let inherited = scope.get() == OwnershipScope::Author
                            && selected_value.is_none();
                        view! {
                            <option value="inherit" selected=inherited>
                                "Inherit site selection"
                            </option>
                            <For
                                each=move || built_in_themes()
                                key=|theme| *theme
                                children=move |theme| {
                                    view! {
                                        <option value=theme.to_string()>{theme.to_string()}</option>
                                    }
                                }
                            />
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
                        }
                    }}
                </select>
            </label>
        </div>
    }
}

#[component]
fn ThemeEditor(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    rename_name: NodeRef<leptos::html::Input>,
) -> impl IntoView {
    let stylesheet = NodeRef::<leptos::html::Textarea>::new();
    let pool_paths = NodeRef::<leptos::html::Textarea>::new();
    let preview_document = RwSignal::new(None::<String>);
    let rename = move |_| rename_selected(scope, selected, rename_name, refresh, status);
    let save_css = move |_| save_selected_css(scope, selected, stylesheet, refresh, status);
    let preview = move |_| preview_selected(scope, selected, preview_document, status);
    let export = move |_| export_selected(scope, selected, status);
    let publish = move |_| publish_selected(scope, selected, refresh, status);
    let remove = move |_| remove_selected(scope, selected, refresh, status);
    view! {
        <Show
            when=move || selected.get().is_some()
            fallback=|| {
                view! {
                    <p class="j-sub">
                        "Choose a catalog entry to edit its draft, presentation, or publication."
                    </p>
                }
            }
        >
            <div class="j-theme-editor" aria-label="Selected theme editor">
                <label class="j-form-field">
                    <span class="j-form-label">"Rename selected theme"</span>
                    <input class="j-form-input" node_ref=rename_name />
                </label>
                <button type="button" class="j-btn" on:click=rename>
                    "Rename"
                </button>
                <label class="j-form-field">
                    <span class="j-form-label">"Stylesheet source"</span>
                    <textarea
                        class="j-form-input j-theme-code"
                        node_ref=stylesheet
                        spellcheck="false"
                        aria-describedby="theme-css-help"
                    ></textarea>
                    <span id="theme-css-help" class="j-form-help">
                        "Save replaces the editable stylesheet while preserving its manifest and package assets."
                    </span>
                </label>
                <button type="button" class="j-btn" on:click=save_css>
                    "Save CSS"
                </button>
                <ThemePresentationEditor
                    scope=scope
                    selected=selected
                    refresh=refresh
                    status=status
                    pool_paths=pool_paths
                />
                <DraftPackageEditor scope=scope selected=selected refresh=refresh status=status />
                <div class="j-theme-actions">
                    <button type="button" class="j-btn" on:click=preview>
                        "Preview draft"
                    </button>
                    <button type="button" class="j-btn" on:click=export>
                        "Export ZIP"
                    </button>
                    <button type="button" class="j-btn" on:click=publish>
                        "Publish"
                    </button>
                    <button type="button" class="j-btn is-danger" on:click=remove>
                        "Delete theme"
                    </button>
                </div>
                {move || {
                    preview_document
                        .get()
                        .map(|document| {
                            view! {
                                <iframe
                                    class="j-theme-preview"
                                    title="Isolated theme preview"
                                    sandbox=""
                                    srcdoc=document
                                ></iframe>
                            }
                        })
                }}
            </div>
        </Show>
    }
}

fn rename_selected(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    name: NodeRef<leptos::html::Input>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    if let Some(id) = selected.get_untracked() {
        let scope = scope.get_untracked();
        let name = name.get().map(|input| input.value()).unwrap_or_default();
        spawn_local(async move { settle(api::rename(scope, id, name).await, refresh, status) });
    }
}

fn save_selected_css(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    css: NodeRef<leptos::html::Textarea>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    if let Some(id) = selected.get_untracked() {
        let scope = scope.get_untracked();
        let css = css
            .get()
            .map(|input| input.value().into_bytes())
            .unwrap_or_default();
        spawn_local(async move { settle(api::replace_css(scope, id, css).await, refresh, status) });
    }
}

fn preview_selected(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    document: RwSignal<Option<String>>,
    status: RwSignal<Option<String>>,
) {
    if let Some(id) = selected.get_untracked() {
        let scope = scope.get_untracked();
        spawn_local(async move {
            match api::preview(scope, id).await {
                Ok(preview) => {
                    document.set(Some(format!(
                        "<!doctype html><html><head><style>{}</style></head><body>{}</body></html>",
                        preview.css, preview.html
                    )));
                    status.set(Some("Preview updated in an isolated document.".into()));
                }
                Err(error) => status.set(Some(error.to_string())),
            }
        });
    }
}

fn export_selected(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    status: RwSignal<Option<String>>,
) {
    if let Some(id) = selected.get_untracked() {
        let scope = scope.get_untracked();
        spawn_local(async move {
            match api::export(scope, id).await {
                Ok(package) => match download_package(&package.filename, &package.bytes) {
                    Ok(()) => status.set(Some("Theme Package download started.".into())),
                    Err(message) => status.set(Some(message)),
                },
                Err(error) => status.set(Some(error.to_string())),
            }
        });
    }
}

fn publish_selected(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    if let Some(id) = selected.get_untracked() {
        let scope = scope.get_untracked();
        spawn_local(async move { settle(api::publish(scope, id).await, refresh, status) });
    }
}

fn remove_selected(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    if let Some(id) = selected.get_untracked() {
        selected.set(None);
        let scope = scope.get_untracked();
        spawn_local(async move { settle(api::remove(scope, id).await, refresh, status) });
    }
}

#[component]
fn DraftPackageEditor(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) -> impl IntoView {
    let draft = Resource::new(
        move || selected.get(),
        move |id| async move {
            match id {
                Some(id) => Some(api::get_draft(scope.get_untracked(), id).await),
                None => None,
            }
        },
    );
    view! {
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading draft package…"</p> }
        }>
            {move || Suspend::new(async move {
                match draft.await {
                    Some(Ok(draft)) => {
                        view! {
                            <DraftPackageForm
                                scope=scope
                                selected=selected
                                draft=draft
                                refresh=refresh
                                status=status
                            />
                        }
                            .into_any()
                    }
                    Some(Err(error)) => {
                        view! {
                            <p class="error" role="alert">
                                {error.to_string()}
                            </p>
                        }
                            .into_any()
                    }
                    None => ().into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn DraftPackageForm(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    draft: super::Draft,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) -> impl IntoView {
    let manifest = NodeRef::<leptos::html::Textarea>::new();
    let stylesheet = NodeRef::<leptos::html::Textarea>::new();
    let assets_editor = NodeRef::<leptos::html::Textarea>::new();
    let manifest_text = String::from_utf8_lossy(&draft.manifest).into_owned();
    let stylesheet_text = String::from_utf8_lossy(&draft.stylesheet).into_owned();
    let assets_text = serde_json::to_string_pretty(&draft.assets).unwrap_or_default();
    let save_package = move |_| {
        let Some(id) = selected.get_untracked() else {
            return;
        };
        let manifest = manifest
            .get()
            .map(|input| input.value())
            .unwrap_or_default();
        let stylesheet = stylesheet
            .get()
            .map(|input| input.value())
            .unwrap_or_default();
        let assets = assets_editor
            .get()
            .map(|input| input.value())
            .unwrap_or_default();
        match draft_from_editor(manifest, stylesheet, &assets) {
            Ok(package) => {
                let scope = scope.get_untracked();
                spawn_local(async move {
                    settle(
                        api::import_package(scope, id, package).await,
                        refresh,
                        status,
                    );
                });
            }
            Err(error) => status.set(Some(error)),
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
                    node_ref=stylesheet
                    prop:value=stylesheet_text
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

#[component]
fn ThemePresentationEditor(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
    pool_paths: NodeRef<leptos::html::Textarea>,
) -> impl IntoView {
    let pool = move |_| {
        if let Some(id) = selected.get_untracked() {
            let paths = pool_paths
                .get()
                .map(|input| input.value())
                .unwrap_or_default();
            let entries = paths
                .lines()
                .filter(|path| !path.trim().is_empty())
                .map(|path| ThemePoolInput::PackageAsset(path.trim().to_owned()))
                .collect();
            let current_scope = scope.get_untracked();
            spawn_local(async move {
                settle(
                    api::replace_pool(current_scope, id, entries, fresh_seed()).await,
                    refresh,
                    status,
                );
            });
        }
    };
    let shuffle = move |_| {
        if let Some(id) = selected.get_untracked() {
            let current_scope = scope.get_untracked();
            spawn_local(async move {
                settle(
                    api::shuffle(current_scope, id, fresh_seed()).await,
                    refresh,
                    status,
                );
            });
        }
    };
    view! {
        <fieldset class="j-theme-presentation">
            <legend>"Presentation media"</legend>
            <div class="j-theme-actions">
                <button
                    type="button"
                    class="j-btn"
                    on:click=move |_| replace_role(
                        scope,
                        selected,
                        ThemeImageRole::Logo,
                        ThemeBindingInput::PackagedDefault,
                        refresh,
                        status,
                    )
                >
                    "Use package logo default"
                </button>
                <button
                    type="button"
                    class="j-btn"
                    on:click=move |_| replace_role(
                        scope,
                        selected,
                        ThemeImageRole::Logo,
                        ThemeBindingInput::ExplicitAbsent,
                        refresh,
                        status,
                    )
                >
                    "Clear logo"
                </button>
                <button
                    type="button"
                    class="j-btn"
                    on:click=move |_| replace_role(
                        scope,
                        selected,
                        ThemeImageRole::Header,
                        ThemeBindingInput::PackagedDefault,
                        refresh,
                        status,
                    )
                >
                    "Use package header default"
                </button>
                <button
                    type="button"
                    class="j-btn"
                    on:click=move |_| replace_role(
                        scope,
                        selected,
                        ThemeImageRole::Header,
                        ThemeBindingInput::ExplicitAbsent,
                        refresh,
                        status,
                    )
                >
                    "Clear header"
                </button>
            </div>
            <label class="j-form-field">
                <span class="j-form-label">"Header pool package asset paths"</span>
                <textarea class="j-form-input j-theme-code" node_ref=pool_paths></textarea>
            </label>
            <button type="button" class="j-btn" on:click=pool>
                "Save header pool"
            </button>
            <button type="button" class="j-btn" on:click=shuffle>
                "Shuffle assignments"
            </button>
        </fieldset>
    }
}

fn replace_role(
    scope: RwSignal<OwnershipScope>,
    selected: RwSignal<Option<ThemeId>>,
    role: ThemeImageRole,
    input: ThemeBindingInput,
    refresh: Invalidator,
    status: RwSignal<Option<String>>,
) {
    if let Some(id) = selected.get_untracked() {
        let current_scope = scope.get_untracked();
        spawn_local(async move {
            settle(
                api::replace_binding(current_scope, id, role, input).await,
                refresh,
                status,
            );
        });
    }
}

fn fresh_seed() -> [u8; 32] {
    let mut seed = [0; 32];
    let _ = leptos::web_sys::window()
        .and_then(|window| window.crypto().ok())
        .and_then(|crypto| crypto.get_random_values_with_u8_array(&mut seed).ok());
    seed
}

fn download_package(filename: &str, bytes: &[u8]) -> Result<(), String> {
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

fn selection_token(selection: Option<PublicThemeSelection>) -> String {
    match selection {
        None => "inherit".into(),
        Some(PublicThemeSelection::BuiltIn(theme)) => theme.to_string(),
        Some(PublicThemeSelection::Custom(id)) => id.to_string(),
    }
}

fn aria_pressed(pressed: bool) -> &'static str {
    if pressed { "true" } else { "false" }
}
