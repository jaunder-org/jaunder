//! The media vertical's wasm-only UI (ADR-0070): the upload control and its
//! browser file-picker glue. Declared `#[cfg(target_arch = "wasm32")] mod component;`
//! in `media/mod.rs`, so this file is wasm-only by its `mod` declaration and
//! carries no cfg gates of its own; it calls browser APIs directly. The upload
//! itself goes through the [`super::upload`] multipart `#[server]` fn.

use leptos::prelude::*;

use common::pagination::{PageOffset, PageSize};
use common::root_relative_url::RootRelativeUrl;

use super::{
    Delete, DeleteMediaRequest, DeleteResult, Item, UploadCallbacks, UploadState, UsageData,
    format_bytes, get_usage, list_mine, storage_usage_percent, upload,
};
use crate::error::{WebError, WebResult};
use crate::forms::request_submit_gate;
use crate::topbar::Topbar;

/// A media upload control: a button that opens the file picker and immediately
/// uploads the chosen file via the [`super::upload`] multipart `#[server]`
/// fn (no navigation).
///
/// `on_uploaded` / `on_error`, when provided, fire with the media URL or a
/// human-readable error. When `show_result` is set the widget also renders the
/// uploaded URL (read-only, click-to-select) and any error inline below the button
/// — the self-contained mode the compose form uses.
#[component]
pub fn MediaUpload(
    /// Called with the `/media/upload/...` URL when the upload succeeds.
    #[prop(into, optional)]
    on_uploaded: Option<Callback<RootRelativeUrl>>,
    /// Called with an error message when the upload fails.
    #[prop(into, optional)]
    on_error: Option<Callback<String>>,
    /// When true, render the uploaded URL and any error inline below the button.
    #[prop(optional)]
    show_result: bool,
) -> impl IntoView {
    // The signal bundle, the outcome fold, and the notify/record sequencing are all
    // host-compiled and host-tested in `super::upload_state` (#306, ADR-0083); what
    // stays here is the browser wiring that cannot run on the host — the file picker
    // and `spawn_local`.
    let state = UploadState::new(show_result);
    let callbacks = UploadCallbacks {
        on_uploaded,
        on_error,
    };
    let file_input = NodeRef::<leptos::html::Input>::new();

    let open_picker = move |_| {
        if let Some(input) = file_input.get() {
            input.click();
        }
    };

    // The event carries nothing we need — the picked file is read from `file_input`.
    let on_file_change = move |_: leptos::ev::Event| {
        use leptos::task::spawn_local;

        // The browser glue — reading the picked file and wrapping it as multipart — lives
        // in `client::upload` so this crate names no `web_sys` type (#520).
        let Some(form_data) = client::upload::picked_file_multipart(file_input) else {
            return;
        };

        state.begin();

        spawn_local(async move {
            state.settle(upload(form_data).await, callbacks);
        });
    };

    view! {
        <input type="file" node_ref=file_input style="display:none" on:change=on_file_change />
        <button
            type="button"
            class="j-btn"
            disabled=move || state.uploading.get()
            on:click=open_picker
        >
            {move || if state.uploading.get() { "Uploading\u{2026}" } else { "Attach media" }}
        </button>
        {move || show_result.then(|| state.last_media_url.get()).flatten().map(uploaded_url_view)}
        {move || {
            show_result
                .then(|| state.error.get())
                .flatten()
                .map(|msg| {
                    view! {
                        <p class="error" style="margin-top:6px;font-size:12px">
                            {msg}
                        </p>
                    }
                })
        }}
    }
}

/// The read-only, click-to-select "Uploaded URL" box shown below the button in the
/// `show_result` mode. Extracted from [`MediaUpload`] to keep that component within
/// the line budget; a plain view helper (like `render_media_row` in this vertical).
///
/// Takes the newtype and unwraps it here: a newtype is not `IntoAttributeValue`, so the
/// `String` view is taken at the view site rather than the value being carried around
/// stringly. `String::from` moves the inner value out rather than allocating a copy.
fn uploaded_url_view(url: RootRelativeUrl) -> impl IntoView {
    view! {
        <div style="margin-top:8px">
            <div style="font-size:12px;color:#888;margin-bottom:4px">"Uploaded URL:"</div>
            <input
                type="text"
                readonly
                value=String::from(url)
                class="j-field-val"
                style="font-size:12px;cursor:text"
                on:click=move |ev| client::dom::select_event_target_text(&ev)
            />
        </div>
    }
}

#[component]
pub fn MediaPage() -> impl IntoView {
    let delete_action = ServerAction::<Delete>::new();
    let successful_deletes = RwSignal::new(0_u32);
    Effect::new(move |_| {
        if let Some(Ok(result)) = delete_action.value().get()
            && result.deleted
        {
            successful_deletes.update(|version| *version += 1);
        }
    });
    let upload_version = RwSignal::new(0u32);
    // Which item the last Delete click was about. `DeleteResult` carries only the
    // referencing post ids, so the refusal branch cannot otherwise name the item it
    // must offer a force-delete for.
    let delete_target = RwSignal::new(Option::<Item>::None);

    let usage = Resource::new(
        move || (successful_deletes.get(), upload_version.get()),
        |_: (u32, u32)| get_usage(),
    );

    let media_list = Resource::new(
        move || (successful_deletes.get(), upload_version.get()),
        |_: (u32, u32)| list_mine(None, Some(PageSize::default()), Some(PageOffset::default())),
    );

    view! {
        <Topbar title="Media" sub="Your uploads" />
        <div class="j-page">
            <div class="j-sb-head" style="margin-bottom:8px">
                "Upload"
            </div>
            <div style="margin-bottom:24px">
                <MediaUpload
                    on_uploaded=Callback::new(move |_url: RootRelativeUrl| {
                        upload_version.update(|v| *v += 1);
                    })
                    on_error=Callback::new(move |msg: String| {
                        leptos::logging::warn!("upload error: {msg}");
                    })
                />
            </div>
            <MediaUsagePanel usage=usage />
            <MediaListPanel
                media_list=media_list
                delete_action=delete_action
                delete_target=delete_target
            />
            <MediaDeleteOutcome delete_action=delete_action delete_target=delete_target />
        </div>
    }
}

/// The "Storage" section: the used/quota/max-file-size line and the usage bar, or the
/// fetch error. Split out of [`MediaPage`] (#306) so that page's `view!` holds no
/// control flow; this component owns the resolved/failed and the divide-by-zero
/// decisions its `Suspense` body makes.
#[component]
fn MediaUsagePanel(usage: Resource<WebResult<UsageData>>) -> impl IntoView {
    view! {
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading usage\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match usage.await {
                    Ok(u) => {
                        let pct = storage_usage_percent(
                            u.used_bytes.value(),
                            u.quota_bytes.value(),
                        );
                        // Clamping, the zero-quota case and the rounding all live in
                        // the host-tested pure leaf; this is a wasm-only file, so
                        // logic kept here could not be unit tested at all.
                        view! {
                            <div class="j-sb-head" style="margin-bottom:8px">
                                "Storage"
                            </div>
                            <p>
                                {format!(
                                    "{} used of {} quota (max file size: {})",
                                    format_bytes(u.used_bytes),
                                    format_bytes(u.quota_bytes),
                                    format_bytes(u.max_file_size_bytes),
                                )}
                            </p>
                            <div style="background:#eee;border-radius:4px;height:8px;width:300px;margin:8px 0 16px">
                                <div style=format!(
                                    "background:#4a9eff;border-radius:4px;height:8px;width:{pct}%",
                                ) />
                            </div>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}

/// The uploads table, its empty state, and the list fetch error. Split out of
/// [`MediaPage`] (#306); this component owns the resolved/failed and empty/non-empty
/// decisions its `Suspense` body makes.
#[component]
fn MediaListPanel(
    media_list: Resource<WebResult<Vec<Item>>>,
    /// Threaded through to each row's delete form.
    delete_action: ServerAction<Delete>,
    /// Set by a row's Delete click so a refusal can name the item it was about.
    delete_target: RwSignal<Option<Item>>,
) -> impl IntoView {
    view! {
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading media\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match media_list.await {
                    Ok(items) => {
                        if items.is_empty() {
                            return view! { <p>"No media uploaded yet."</p> }.into_any();
                        }
                        view! {
                            <table class="j-table">
                                <thead>
                                    <tr>
                                        <th>"Filename"</th>
                                        <th>"Type"</th>
                                        <th>"Size"</th>
                                        <th>"Source"</th>
                                        <th>"Uploaded"</th>
                                        <th></th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {items
                                        .into_iter()
                                        .map(|item| render_media_row(
                                            &item,
                                            delete_action,
                                            delete_target,
                                        ))
                                        .collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}

/// The outcome line for the most recent delete: confirmation, the "still referenced"
/// refusal naming the posts, or the error. Nothing until the action has a value.
/// Split out of [`MediaPage`] (#306); this component owns that three-way decision.
#[component]
fn MediaDeleteOutcome(
    delete_action: ServerAction<Delete>,
    /// The item the refused delete was about, so the refusal can offer a force.
    delete_target: RwSignal<Option<Item>>,
) -> impl IntoView {
    view! {
        {move || {
            delete_action
                .value()
                .get()
                .map(|result: Result<DeleteResult, WebError>| match result {
                    Ok(r) if r.deleted => {
                        view! { <p class="success">"Media deleted."</p> }.into_any()
                    }
                    Ok(r) => {
                        let ids = r
                            .referenced_in_posts
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ");
                        view! {
                            <p class="error">
                                {format!(
                                    "Cannot delete: referenced in post(s) {ids}. Use force delete to remove anyway.",
                                )}
                            </p>
                            {move || {
                                delete_target
                                    .get()
                                    .map(|item| force_delete_form(&item, delete_action))
                            }}
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
    }
}

/// The force-delete submission rendered after an ordinary delete refuses a
/// referenced item. It dispatches the same aggregate with `force: Some(true)`.
fn force_delete_form(item: &Item, delete_action: ServerAction<Delete>) -> impl IntoView + use<> {
    let display_name = item.filename.decoded().into_owned();
    let request = DeleteMediaRequest {
        sha256: item.sha256.clone(),
        filename: item.filename.clone(),
        source: item.source,
        force: Some(true),
    };
    let (disabled, dispatch_delete) = request_submit_gate(
        delete_action.pending().into(),
        Callback::new(move |()| Some(request.clone())),
        Callback::new(move |request| {
            delete_action.dispatch(Delete { request });
        }),
    );
    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        dispatch_delete.run(());
    };

    view! {
        <form on:submit=submit>
            <button
                type="submit"
                class="j-btn is-danger"
                onclick="return confirm('Delete anyway? Posts that embed this item will keep pointing at it.')"
                prop:disabled=move || disabled.get()
            >
                {format!("Force delete {display_name}")}
            </button>
        </form>
    }
}

/// One row of the media table: the link, metadata, and typed ordinary-delete form.
fn render_media_row(
    item: &Item,
    delete_action: ServerAction<Delete>,
    delete_target: RwSignal<Option<Item>>,
) -> impl IntoView + use<> {
    let url = item.url.to_string();
    let display_name = item.filename.decoded().into_owned();
    let source = item.source.to_string();
    let size_label = format_bytes(item.size_bytes);
    let created_at = item.created_at.to_string();
    let content_type = item.content_type.to_string();
    let target = item.clone();
    let request = DeleteMediaRequest {
        sha256: item.sha256.clone(),
        filename: item.filename.clone(),
        source: item.source,
        force: None,
    };
    let (disabled, dispatch_delete) = request_submit_gate(
        delete_action.pending().into(),
        Callback::new(move |()| Some((target.clone(), request.clone()))),
        Callback::new(move |(target, request)| {
            delete_target.set(Some(target));
            delete_action.dispatch(Delete { request });
        }),
    );
    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        dispatch_delete.run(());
    };

    view! {
        <tr>
            <td>
                <a href=url target="_blank">
                    {display_name}
                </a>
            </td>
            <td>{content_type}</td>
            <td>{size_label}</td>
            <td>{source}</td>
            <td>{created_at}</td>
            <td>
                <form on:submit=submit>
                    <button
                        type="submit"
                        class="j-btn is-danger"
                        onclick="return confirm('Delete this media item?')"
                        prop:disabled=move || disabled.get()
                    >
                        "Delete"
                    </button>
                </form>
            </td>
        </tr>
    }
}
