use crate::{
    error::WebError,
    media::{list_my_media, media_usage, DeleteMedia, DeleteMediaResult, MediaItem},
    pages::ui::Topbar,
    pages::MediaUploadButton,
};
use leptos::prelude::*;

#[expect(
    clippy::cast_precision_loss,
    reason = "byte counts < 2^52 convert to f64 exactly; larger values only affect a \
              human-readable one-decimal display, so any loss is immaterial"
)]
fn format_bytes(bytes: i64) -> String {
    const KB: i64 = 1_024;
    const MB: i64 = 1_024 * KB;
    const GB: i64 = 1_024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[allow(clippy::too_many_lines)]
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
        <Topbar title="Media".to_string() sub="Your uploads".to_string() />
        <div class="j-page">
            <div class="j-sb-head" style="margin-bottom:8px">
                "Upload"
            </div>
            <div style="margin-bottom:24px">
                <MediaUploadButton
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
                                            .map(|item| render_media_row(item, delete_action))
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

#[allow(clippy::needless_pass_by_value)]
// cov:ignore-start
fn render_media_row(item: MediaItem, delete_action: ServerAction<DeleteMedia>) -> impl IntoView {
    let url = item.url.clone();
    let filename = item.filename.clone();
    let sha256 = item.sha256.clone();
    let source = item.source.clone();
    let size_label = format_bytes(item.size_bytes);
    let created_at = item.created_at.clone();
    // cov:ignore-stop

    // cov:ignore-start
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
// cov:ignore-stop

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_displays_bytes_below_kb() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_displays_kb_range() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn format_bytes_displays_mb_range() {
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 2), "2.0 MB");
    }

    #[test]
    fn format_bytes_displays_gb_range() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }
}
