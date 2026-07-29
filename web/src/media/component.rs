//! The media vertical's wasm-only UI (ADR-0070): the upload control and its
//! browser file-picker glue. Declared `#[cfg(target_arch = "wasm32")] mod component;`
//! in `media/mod.rs`, so this file is wasm-only by its `mod` declaration and
//! carries no cfg gates of its own; it calls browser APIs directly. The upload
//! itself goes through the [`super::upload`] multipart `#[server]` fn.

use leptos::prelude::*;

use common::pagination::{PageOffset, PageSize};
use common::root_relative_url::RootRelativeUrl;

use super::{format_bytes, list_mine, upload, usage, Delete, DeleteMediaResult, MediaItem};
use crate::error::WebError;
use crate::topbar::Topbar;

/// A media upload control: a button that opens the file picker and immediately
/// uploads the chosen file via the [`super::upload`] multipart `#[server]`
/// fn (no navigation).
///
/// `on_uploaded` / `on_error`, when provided, fire with the media URL or a
/// human-readable error. When `show_result` is set the widget also renders the
/// uploaded URL (read-only, click-to-select) and any error inline below the button
/// — the self-contained mode the compose form uses. (This merges the former
/// `MediaUploadButton` primitive and `MediaPanel` wrapper into one component.)
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
    let uploading = RwSignal::new(false);
    let last_media_url = RwSignal::new(Option::<RootRelativeUrl>::None);
    let upload_error = RwSignal::new(Option::<String>::None);
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

        uploading.set(true);

        spawn_local(async move {
            let result = upload(form_data).await;
            uploading.set(false);
            match result {
                Ok(resp) => {
                    let url = resp.url;
                    if let Some(cb) = on_uploaded {
                        cb.run(url.clone());
                    }
                    if show_result {
                        last_media_url.set(Some(url));
                        upload_error.set(None);
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    if let Some(cb) = on_error {
                        cb.run(msg.clone());
                    }
                    if show_result {
                        upload_error.set(Some(msg));
                    }
                }
            }
        });
    };

    view! {
        <input type="file" node_ref=file_input style="display:none" on:change=on_file_change />
        <button type="button" class="j-btn" disabled=move || uploading.get() on:click=open_picker>
            {move || if uploading.get() { "Uploading\u{2026}" } else { "Attach media" }}
        </button>
        {move || show_result.then(|| last_media_url.get()).flatten().map(uploaded_url_view)}
        {move || {
            show_result
                .then(|| upload_error.get())
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

#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
#[component]
pub fn MediaPage() -> impl IntoView {
    let delete_action = ServerAction::<Delete>::new();
    let upload_version = RwSignal::new(0u32);

    let usage = Resource::new(
        move || (delete_action.version().get(), upload_version.get()),
        |_: (usize, u32)| usage(),
    );

    let media_list = Resource::new(
        move || (delete_action.version().get(), upload_version.get()),
        |_: (usize, u32)| list_mine(None, Some(PageSize::default()), Some(PageOffset::default())),
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
            <Suspense fallback=|| {
                view! { <p class="j-loading">"Loading usage\u{2026}"</p> }
            }>
                {move || Suspend::new(async move {
                    match usage.await {
                        Ok(u) => {
                            #[expect(
                                clippy::cast_precision_loss,
                                reason = "display-only storage-usage percentage; byte \
                                          counts < 2^52 are exact in f64 and the result \
                                          is clamped to 100"
                            )]
                            let pct = if u.quota_bytes.value() > 0 {
                                (u.used_bytes.value() as f64 / u.quota_bytes.value() as f64 * 100.0)
                                    .min(100.0)
                            } else {
                                0.0
                            };
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
                                        "background:#4a9eff;border-radius:4px;height:8px;width:{pct:.1}%",
                                    ) />
                                </div>
                            }
                                .into_any()
                        }
                        Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                    }
                })}
            </Suspense>

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
                                            .map(|item| render_media_row(&item, delete_action))
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

            {move || {
                delete_action
                    .value()
                    .get()
                    .map(|result: Result<DeleteMediaResult, WebError>| match result {
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
                            }
                                .into_any()
                        }
                        Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                    })
            }}
        </div>
    }
}

fn render_media_row(item: &MediaItem, delete_action: ServerAction<Delete>) -> impl IntoView {
    // Same reason as `filename` below: `RootRelativeUrl` is not an `IntoAttributeValue`,
    // so the `href` gets its `str` view here.
    let url = item.url.to_string();
    // `Filename` implements neither Leptos `IntoView` nor `IntoAttributeValue`, so
    // render it as an owned String for the link text and the hidden form field
    // (mirroring `item.sha256.to_string()` below).
    let filename = item.filename.to_string();
    // The ActionForm hidden field needs an owned String; `ContentHash: Display`.
    let sha256 = item.sha256.to_string();
    let source = item.source.to_string();
    let size_label = format_bytes(item.size_bytes);
    let created_at = item.created_at.to_string();

    view! {
        <tr>
            <td>
                <a href=url target="_blank">
                    {filename.clone()}
                </a>
            </td>
            <td>{item.content_type.to_string()}</td>
            <td>{size_label}</td>
            <td>{source.clone()}</td>
            <td>{created_at}</td>
            <td>
                <ActionForm action=delete_action>
                    <input type="hidden" name="sha256" value=sha256 />
                    <input type="hidden" name="filename" value=filename />
                    <input type="hidden" name="source" value=source />
                    <button
                        type="submit"
                        class="j-btn is-danger"
                        onclick="return confirm('Delete this media item?')"
                    >
                        "Delete"
                    </button>
                </ActionForm>
            </td>
        </tr>
    }
}
