//! The media vertical's wasm-only UI (ADR-0070): the upload control and its
//! browser `fetch` glue. Declared `#[cfg(target_arch = "wasm32")] mod component;`
//! in `media/mod.rs`, so this file is wasm-only by its `mod` declaration and
//! carries no cfg gates of its own; it calls browser APIs directly. The pure
//! response-parse is factored to the host-tested [`super::api::extract_upload_url`].

use leptos::prelude::*;

use super::{list_my_media, media_usage, DeleteMedia, DeleteMediaResult, MediaItem};
use crate::error::WebError;
use crate::render::format_bytes;
use crate::topbar::Topbar;

/// A media upload control: a button that opens the file picker and immediately
/// uploads the chosen file to `/media/upload` via a JS `fetch` (no navigation).
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
    on_uploaded: Option<Callback<String>>,
    /// Called with an error message when the upload fails.
    #[prop(into, optional)]
    on_error: Option<Callback<String>>,
    /// When true, render the uploaded URL and any error inline below the button.
    #[prop(optional)]
    show_result: bool,
) -> impl IntoView {
    let uploading = RwSignal::new(false);
    let last_media_url = RwSignal::new(Option::<String>::None);
    let upload_error = RwSignal::new(Option::<String>::None);
    let file_input = NodeRef::<leptos::html::Input>::new();

    let open_picker = move |_| {
        if let Some(input) = file_input.get() {
            input.click();
        }
    };

    let on_file_change = move |ev: leptos::ev::Event| {
        use leptos::task::spawn_local;
        use leptos::wasm_bindgen::JsCast;

        let _ = ev;

        let Some(input) = file_input.get() else {
            return;
        };
        let input_el: web_sys::HtmlInputElement = input.unchecked_into();
        let Some(files) = input_el.files() else {
            return;
        };
        let Some(file): Option<web_sys::File> = files.get(0) else {
            return;
        };

        let Ok(form_data) = web_sys::FormData::new() else {
            return;
        };
        if form_data.append_with_blob("file", &file).is_err() {
            return;
        }

        uploading.set(true);

        spawn_local(async move {
            let result = upload_file(form_data).await;
            uploading.set(false);
            match result {
                Ok(url) => {
                    if let Some(cb) = on_uploaded {
                        cb.run(url.clone());
                    }
                    if show_result {
                        last_media_url.set(Some(url));
                        upload_error.set(None);
                    }
                }
                Err(msg) => {
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
fn uploaded_url_view(url: String) -> impl IntoView {
    view! {
        <div style="margin-top:8px">
            <div style="font-size:12px;color:#888;margin-bottom:4px">"Uploaded URL:"</div>
            <input
                type="text"
                readonly
                value=url
                class="j-field-val"
                style="font-size:12px;cursor:text"
                on:click=move |ev| {
                    use leptos::wasm_bindgen::JsCast;
                    let _ = ev
                        .target()
                        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                        .map(|i| i.select());
                }
            />
        </div>
    }
}

async fn upload_file(form_data: web_sys::FormData) -> Result<String, String> {
    // crap:allow: wasm-only browser glue — a `fetch` POST to /media/upload. Not
    // host-instrumentable (`web_sys::window` / `fetch`), so it is uncovered and
    // CRAP source-parses it (a plain async fn, not a CRAP-exempt `#[component]`);
    // its verification is the media-upload e2e. The pure response parse is factored
    // out to the host-tested `super::api::extract_upload_url`.
    use leptos::wasm_bindgen::JsCast;
    use leptos::wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or("no window")?;

    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    let body_val: JsValue = form_data.into();
    opts.set_body(&body_val);

    let request = web_sys::Request::new_with_str_and_init("/media/upload", &opts).map_err(|e| {
        e.as_string()
            .unwrap_or_else(|| "failed to build request".to_string())
    })?;

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "network error".to_string()))?;

    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "unexpected response type".to_string())?;

    if !resp.ok() {
        return Err(format!("upload failed (HTTP {})", resp.status()));
    }

    let text_promise = resp.text().map_err(|_| "failed to read response body")?;
    let text_value: JsValue = JsFuture::from(text_promise)
        .await
        .map_err(|_| "failed to await response text")?;

    let body: String = text_value
        .as_string()
        .ok_or_else(|| "response body is not a string".to_string())?;

    super::api::extract_upload_url(&body)
}

#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
#[component]
pub fn MediaPage() -> impl IntoView {
    let delete_action = ServerAction::<DeleteMedia>::new();
    let upload_version = RwSignal::new(0u32);

    let usage = crate::server_resource(
        move || (delete_action.version().get(), upload_version.get()),
        |_: (usize, u32)| media_usage(),
    );

    let media_list = crate::server_resource(
        move || (delete_action.version().get(), upload_version.get()),
        |_: (usize, u32)| list_my_media(None, Some(50), Some(0)),
    );

    view! {
        <Topbar title="Media" sub="Your uploads" />
        <div class="j-page">
            <div class="j-sb-head" style="margin-bottom:8px">
                "Upload"
            </div>
            <div style="margin-bottom:24px">
                <MediaUpload
                    on_uploaded=Callback::new(move |_url: String| {
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
                            let pct = if u.quota_bytes > 0 {
                                (u.used_bytes as f64 / u.quota_bytes as f64 * 100.0).min(100.0)
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

fn render_media_row(item: &MediaItem, delete_action: ServerAction<DeleteMedia>) -> impl IntoView {
    let url = item.url.clone();
    // `Filename` implements neither Leptos `IntoView` nor `IntoAttributeValue`, so
    // render it as an owned String for the link text and the hidden form field
    // (mirroring `item.sha256.to_string()` below).
    let filename = item.filename.to_string();
    // The ActionForm hidden field needs an owned String; `ContentHash: Display`.
    let sha256 = item.sha256.to_string();
    let source = item.source.clone();
    let size_label = format_bytes(item.size_bytes);
    let created_at = item.created_at.clone();

    view! {
        <tr>
            <td>
                <a href=url target="_blank">
                    {filename.clone()}
                </a>
            </td>
            <td>{item.content_type.clone()}</td>
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
