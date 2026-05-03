use crate::{
    error::WebError,
    media::{delete_media, list_my_media, media_usage, DeleteMedia, DeleteMediaResult, MediaItem},
    pages::ui::Topbar,
};
use leptos::prelude::*;

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

#[allow(clippy::must_use_candidate)]
#[component]
pub fn MediaPage() -> impl IntoView {
    let delete_action = ServerAction::<DeleteMedia>::new();

    let usage = Resource::new(move || delete_action.version().get(), |_| media_usage());

    let media_list = Resource::new(
        move || delete_action.version().get(),
        |_| list_my_media(None, Some(50), Some(0)),
    );

    view! {
        <Topbar title="Media".to_string() sub="Your uploads".to_string() />
        <div style="padding:16px 32px">
            <Suspense fallback=|| {
                view! { <p class="j-loading">"Loading usage\u{2026}"</p> }
            }>
                {move || Suspend::new(async move {
                    match usage.await {
                        Ok(u) => {
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
                                <table style="border-collapse:collapse;width:100%">
                                    <thead>
                                        <tr style="text-align:left;border-bottom:1px solid #ccc">
                                            <th style="padding:8px">"Filename"</th>
                                            <th style="padding:8px">"Type"</th>
                                            <th style="padding:8px">"Size"</th>
                                            <th style="padding:8px">"Source"</th>
                                            <th style="padding:8px">"Uploaded"</th>
                                            <th style="padding:8px"></th>
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
                                .map(|id| id.to_string())
                                .collect::<Vec<_>>()
                                .join(", ");
                            view! {
                                <p class="error">
                                    {format!(
                                        "Cannot delete: referenced in post(s) {}. Use force delete to remove anyway.",
                                        ids,
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

fn render_media_row(item: MediaItem, delete_action: ServerAction<DeleteMedia>) -> impl IntoView {
    let url = item.url.clone();
    let filename = item.filename.clone();
    let sha256 = item.sha256.clone();
    let source = item.source.clone();
    let size_label = format_bytes(item.size_bytes);
    let created_at = item.created_at.clone();

    view! {
        <tr style="border-bottom:1px solid #eee">
            <td style="padding:8px">
                <a href=url target="_blank">
                    {filename.clone()}
                </a>
            </td>
            <td style="padding:8px">{item.content_type.clone()}</td>
            <td style="padding:8px">{size_label}</td>
            <td style="padding:8px">{source.clone()}</td>
            <td style="padding:8px">{created_at}</td>
            <td style="padding:8px">
                <ActionForm action=delete_action>
                    <input type="hidden" name="sha256" value=sha256 />
                    <input type="hidden" name="filename" value=filename />
                    <input type="hidden" name="source" value=source />
                    <button
                        type="submit"
                        class="j-btn is-ghost"
                        onclick="return confirm('Delete this media item?')"
                    >
                        "Delete"
                    </button>
                </ActionForm>
            </td>
        </tr>
    }
}
