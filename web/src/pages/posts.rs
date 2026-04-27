use crate::{
    auth::current_user,
    pages::{
        signal_read::read_signal,
        ui::{format_post_time, Avatar, Topbar},
    },
    posts::{
        get_post, get_post_preview, list_drafts, list_user_posts, CreatePost, CreatePostResult,
        DeletePost, DraftSummary, ListUserPosts, PostResponse, PublishPost, PublishPostResult,
        TimelinePostSummary, UpdatePost, UpdatePostResult,
    },
};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[component]
pub fn CreatePostPage() -> impl IntoView {
    let create_post_action = ServerAction::<CreatePost>::new();
    let current_user = Resource::new(|| (), |_| current_user());
    let body = RwSignal::new(String::new());
    let char_count = move || read_signal!(body).len();

    view! {
        <Topbar title="New post".to_string() sub="Long-form".to_string() />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match current_user.await {
                    Ok(Some(username)) => {
                        view! {
                            <ActionForm action=create_post_action>
                                <div class="j-compose-grid">
                                    <div class="j-compose-body">
                                        <div style="display:flex;gap:14px">
                                            <Avatar name=username.clone() size=40 />
                                            <div style="flex:1;min-width:0">
                                                <div style="font-size:13px;color:var(--muted);margin-bottom:10px;\
                                                font-family:var(--font-meta)">{username}</div>
                                                <input
                                                    type="text"
                                                    name="title"
                                                    placeholder="Title"
                                                    style="width:100%;font-size:20px;font-weight:600;\
                                                    border:none;outline:none;background:transparent;\
                                                    color:var(--ink);margin-bottom:14px;\
                                                    font-family:var(--font-body)"
                                                />
                                                <textarea
                                                    name="body"
                                                    placeholder="Write something\u{2026}"
                                                    class="j-compose-editor"
                                                    style="width:100%;border:none;outline:none;\
                                                    background:transparent;resize:vertical"
                                                    rows="16"
                                                    prop:value=body
                                                    on:input=move |ev| { body.set(event_target_value(&ev)) }
                                                ></textarea>
                                            </div>
                                        </div>
                                    </div>
                                    <aside class="j-compose-aside">
                                        <div>
                                            <div class="j-sb-head" style="padding:0 0 10px">
                                                "Options"
                                            </div>
                                            <div
                                                class="j-field-row"
                                                style="grid-template-columns:auto 1fr"
                                            >
                                                <label class="j-field-label" for="compose-format">
                                                    "Format"
                                                </label>
                                                <select
                                                    id="compose-format"
                                                    name="format"
                                                    class="j-field-val"
                                                >
                                                    <option value="markdown" selected=true>
                                                        "Markdown"
                                                    </option>
                                                    <option value="org">"Org"</option>
                                                </select>
                                            </div>
                                            <div
                                                class="j-field-row"
                                                style="grid-template-columns:auto 1fr"
                                            >
                                                <label class="j-field-label" for="compose-slug">
                                                    "Slug"
                                                </label>
                                                <input
                                                    id="compose-slug"
                                                    type="text"
                                                    name="slug_override"
                                                    placeholder="auto"
                                                    class="j-field-val"
                                                />
                                            </div>
                                        </div>
                                        <div style="margin-top:auto;display:flex;align-items:center;gap:8px">
                                            <span class="j-count" style="margin-left:0">
                                                {char_count}
                                            </span>
                                            <span class="j-spacer"></span>
                                            <button
                                                class="j-btn"
                                                type="submit"
                                                name="publish"
                                                value="false"
                                            >
                                                "Draft"
                                            </button>
                                            <button
                                                class="j-btn is-primary"
                                                type="submit"
                                                name="publish"
                                                value="true"
                                            >
                                                "Publish \u{2192}"
                                            </button>
                                        </div>
                                    </aside>
                                </div>
                            </ActionForm>
                            {move || {
                                create_post_action
                                    .value()
                                    .get()
                                    .map(|result: Result<CreatePostResult, ServerFnError>| {
                                        match result {
                                            Ok(created) => {
                                                let message = if created.published_at.is_some() {
                                                    "Post published."
                                                } else {
                                                    "Draft saved."
                                                };
                                                let slug_value = created.slug.clone();
                                                let slug_for_attr = slug_value.clone();
                                                view! {
                                                    <div class="success" style="padding:16px 32px">
                                                        <p>{message}</p>
                                                        <p data-test="slug-value" data-slug=slug_for_attr>
                                                            "Slug: "
                                                            {slug_value}
                                                        </p>
                                                        <a
                                                            data-test="preview-link"
                                                            href=created.preview_url.clone()
                                                        >
                                                            "Preview draft"
                                                        </a>
                                                        {created
                                                            .permalink
                                                            .as_ref()
                                                            .map(|href| {
                                                                view! {
                                                                    <a data-test="permalink-link" href=href.clone()>
                                                                        "View permalink"
                                                                    </a>
                                                                }
                                                            })}
                                                    </div>
                                                }
                                                    .into_any()
                                            }
                                            Err(err) => {
                                                view! {
                                                    <p class="error" style="padding:16px 32px">
                                                        {err.to_string()}
                                                    </p>
                                                }
                                                    .into_any()
                                            }
                                        }
                                    })
                            }}
                        }
                            .into_any()
                    }
                    Ok(None) => {
                        view! {
                            <div style="padding:32px">
                                <p>"You must be logged in to create a post."</p>
                                <p>
                                    <a href="/login" class="j-btn is-primary">
                                        "Sign in"
                                    </a>
                                </p>
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

#[component]
pub fn PostPage() -> impl IntoView {
    let delete_action = ServerAction::<DeletePost>::new();
    let params = use_params_map();

    let post_data = move || {
        let params = params.get();
        let raw_username = params.get("username").unwrap_or_default();
        let username = raw_username.strip_prefix('~').map(str::to_string);
        let year = params
            .get("year")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or_default();
        let month = params
            .get("month")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or_default();
        let day = params
            .get("day")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or_default();
        let slug = params.get("slug").unwrap_or_default();

        (username, year, month, day, slug)
    };

    let post = Resource::new(
        post_data,
        |(username, year, month, day, slug): (Option<String>, i32, u32, u32, String)| async move {
            let username = match username {
                Some(value) if !value.is_empty() => value,
                _ => return Err(ServerFnError::new("Invalid permalink")),
            };

            get_post(username, year, month, day, slug).await
        },
    );

    view! {
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched_post) => {
                        let banner = fetched_post.is_draft.then_some("Draft - visible only to you");
                        let post_id = fetched_post.post_id;
                        let is_author = fetched_post.is_author;
                        let article = render_post_article(fetched_post, banner);
                        view! {
                            {article}
                            {is_author
                                .then(|| render_delete_form(
                                    delete_action,
                                    post_id,
                                    "Delete this post?",
                                ))}
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {render_delete_result(delete_action, "Post deleted.", "/", "Go to home")}
    }
}

#[component]
pub fn UserTimelinePage() -> impl IntoView {
    let params = use_params_map();
    let username = Memo::new(move |_| {
        params
            .get()
            .get("username")
            .unwrap_or_default()
            .strip_prefix('~')
            .unwrap_or_default()
            .to_string()
    });

    let initial_page = Resource::new(
        move || username.get(),
        |username| async move {
            if username.is_empty() {
                return Err(ServerFnError::new("Invalid username"));
            }
            list_user_posts(username, None, None, Some(50)).await
        },
    );

    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    let next_cursor_created_at = RwSignal::new(None::<String>);
    let next_cursor_post_id = RwSignal::new(None::<i64>);
    let has_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let initial_loaded = RwSignal::new(false);

    let load_more_action = ServerAction::<ListUserPosts>::new();

    Effect::new_isomorphic(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(page) => {
                    timeline.set(page.posts);
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    error.set(None);
                    initial_loaded.set(true);
                }
                Err(err) => {
                    error.set(Some(err.to_string()));
                    timeline.set(Vec::new());
                    has_more.set(false);
                    initial_loaded.set(true);
                }
            }
        }
    });

    Effect::new_isomorphic(move |_| {
        if let Some(result) = load_more_action.value().get() {
            match result {
                Ok(page) => {
                    timeline.update(|rows| rows.extend(page.posts));
                    next_cursor_created_at.set(page.next_cursor_created_at);
                    next_cursor_post_id.set(page.next_cursor_post_id);
                    has_more.set(page.has_more);
                    error.set(None);
                }
                Err(err) => error.set(Some(err.to_string())),
            }
        }
    });

    let on_load_more = move |_| {
        let username = username.get_untracked();
        if username.is_empty() || !has_more.get_untracked() {
            return;
        }
        load_more_action.dispatch(ListUserPosts {
            username,
            cursor_created_at: next_cursor_created_at.get_untracked(),
            cursor_post_id: next_cursor_post_id.get_untracked(),
            limit: Some(50),
        });
    };

    let display_username = move || read_signal!(username);
    let read_error = move || read_signal!(error);
    let read_initial_loaded = move || read_signal!(initial_loaded);
    let read_timeline = move || read_signal!(timeline);
    let read_has_more = move || read_signal!(has_more);
    let read_pending = move || read_signal!(load_more_action.pending());

    view! {
        <h1>{move || format!("Posts by {}", display_username())}</h1>
        {move || {
            if let Some(err) = read_error() {
                return view! { <p class="error">{err}</p> }.into_any();
            }
            if !read_initial_loaded() {
                return view! { <p>"Loading..."</p> }.into_any();
            }
            let rows = read_timeline();
            if rows.is_empty() {
                return view! { <p>"No posts yet."</p> }.into_any();
            }

            view! {
                <ul>{rows.into_iter().map(render_timeline_post_row).collect::<Vec<_>>()}</ul>
                {move || {
                    read_has_more()
                        .then(|| {
                            view! {
                                <button on:click=on_load_more disabled=read_pending>
                                    {move || {
                                        if read_pending() { "Loading..." } else { "Load more" }
                                    }}
                                </button>
                            }
                        })
                }}
            }
                .into_any()
        }}
    }
}

#[component]
pub fn DraftPreviewPage() -> impl IntoView {
    let delete_action = ServerAction::<DeletePost>::new();
    let publish_action = ServerAction::<PublishPost>::new();
    let params = use_params_map();

    let preview = Resource::new(
        move || params.get(),
        |params| async move {
            let post_id = params
                .get("post_id")
                .and_then(|v| v.parse::<i64>().ok())
                .ok_or_else(|| ServerFnError::new("Invalid preview"))?;
            get_post_preview(post_id).await
        },
    );

    view! {
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match preview.await {
                    Ok(post) => {
                        let post_id = post.post_id;
                        let article = render_post_article(
                            post,
                            Some("Draft preview – visible only to you"),
                        );
                        view! {
                            {article}
                            <div style="display:flex;gap:8px;padding:16px 32px">
                                <ActionForm action=publish_action>
                                    <input type="hidden" name="post_id" value=post_id />
                                    <button
                                        type="submit"
                                        class="j-btn is-primary"
                                        onclick="return confirm('Publish this draft?')"
                                    >
                                        "Publish \u{2192}"
                                    </button>
                                </ActionForm>
                                {render_delete_form(delete_action, post_id, "Delete this draft?")}
                            </div>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            publish_action
                .value()
                .get()
                .map(|result: Result<PublishPostResult, ServerFnError>| match result {
                    Ok(published) => {
                        view! {
                            <p class="success">
                                "Post published. " <a href=published.permalink>"View post"</a>
                            </p>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
        {render_delete_result(delete_action, "Draft deleted.", "/drafts", "Go to drafts")}
    }
}

#[component]
pub fn EditPostPage() -> impl IntoView {
    let params = use_params_map();
    let update_post_action = ServerAction::<UpdatePost>::new();

    let post = Resource::new(
        move || {
            params
                .get()
                .get("post_id")
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(-1)
        },
        get_post_preview,
    );

    view! {
        <Topbar title="Edit Post".to_string() sub="".to_string() />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched) => {
                        let post_id = fetched.post_id;
                        let is_published = fetched.published_at.is_some();
                        let is_markdown = fetched.format == "markdown";
                        let is_org = fetched.format == "org";
                        let current_slug = fetched.slug.clone();
                        view! {
                            <ActionForm action=update_post_action>
                                <div class="j-edit-form-grid">
                                    <div class="j-edit-form-body">
                                        <input type="hidden" name="post_id" value=post_id />
                                        <div class="j-edit-form-field">
                                            <label class="j-edit-form-label" for="edit-title">
                                                "Title"
                                            </label>
                                            <input
                                                id="edit-title"
                                                class="j-edit-form-input"
                                                type="text"
                                                name="title"
                                                prop:value=fetched.title.unwrap_or_default()
                                            />
                                        </div>
                                        <div class="j-edit-form-field j-edit-form-field--body">
                                            <label class="j-edit-form-label" for="edit-body">
                                                "Body"
                                            </label>
                                            <textarea
                                                id="edit-body"
                                                class="j-edit-form-textarea"
                                                name="body"
                                                rows="20"
                                            >
                                                {fetched.body}
                                            </textarea>
                                        </div>
                                    </div>
                                    <aside class="j-edit-form-aside">
                                        <div>
                                            <div class="j-sb-head" style="padding:0 0 10px">
                                                "Options"
                                            </div>
                                            <div
                                                class="j-field-row"
                                                style="grid-template-columns:auto 1fr"
                                            >
                                                <label class="j-field-label" for="edit-format">
                                                    "Format"
                                                </label>
                                                <select id="edit-format" name="format" class="j-field-val">
                                                    <option value="markdown" selected=is_markdown>
                                                        "Markdown"
                                                    </option>
                                                    <option value="org" selected=is_org>
                                                        "Org"
                                                    </option>
                                                </select>
                                            </div>
                                            {(!is_published)
                                                .then(|| {
                                                    view! {
                                                        <div
                                                            class="j-field-row"
                                                            style="grid-template-columns:auto 1fr"
                                                        >
                                                            <label class="j-field-label" for="edit-slug">
                                                                "Slug"
                                                            </label>
                                                            <input
                                                                id="edit-slug"
                                                                type="text"
                                                                name="slug_override"
                                                                class="j-field-val"
                                                                prop:value=current_slug
                                                            />
                                                        </div>
                                                    }
                                                })}
                                        </div>
                                        <div class="j-edit-form-actions">
                                            {if is_published {
                                                view! {
                                                    <button
                                                        class="j-btn is-primary"
                                                        type="submit"
                                                        name="publish"
                                                        value="true"
                                                    >
                                                        "Save"
                                                    </button>
                                                }
                                                    .into_any()
                                            } else {
                                                view! {
                                                    <button
                                                        class="j-btn is-primary"
                                                        type="submit"
                                                        name="publish"
                                                        value="true"
                                                    >
                                                        "Publish \u{2192}"
                                                    </button>
                                                    <button
                                                        class="j-btn"
                                                        type="submit"
                                                        name="publish"
                                                        value="false"
                                                    >
                                                        "Save Draft"
                                                    </button>
                                                }
                                                    .into_any()
                                            }}
                                        </div>
                                    </aside>
                                </div>
                            </ActionForm>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            update_post_action
                .value()
                .get()
                .map(|result: Result<UpdatePostResult, ServerFnError>| match result {
                    Ok(updated) => {
                        let message = if updated.published_at.is_some() {
                            "Post updated."
                        } else {
                            "Draft saved."
                        };
                        let slug_value = updated.slug.clone();
                        let slug_for_attr = slug_value.clone();
                        view! {
                            <div class="success">
                                <p>{message}</p>
                                <p data-test="slug-value" data-slug=slug_for_attr>
                                    "Slug: "
                                    {slug_value}
                                </p>
                                <a data-test="preview-link" href=updated.preview_url.clone()>
                                    "Preview draft"
                                </a>
                                {updated
                                    .permalink
                                    .as_ref()
                                    .map(|href| {
                                        view! {
                                            <a data-test="permalink-link" href=href.clone()>
                                                "View permalink"
                                            </a>
                                        }
                                    })}
                            </div>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
    }
}

#[component]
pub fn DraftsPage() -> impl IntoView {
    let publish_action = ServerAction::<PublishPost>::new();
    let delete_action = ServerAction::<DeletePost>::new();
    let drafts = Resource::new(
        move || {
            (
                publish_action.version().get(),
                delete_action.version().get(),
            )
        },
        |_| list_drafts(None, None, Some(50)),
    );

    view! {
        <h1>"Drafts"</h1>
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match drafts.await {
                    Ok(list) => {
                        if list.is_empty() {
                            return view! { <p>"You have no drafts."</p> }.into_any();
                        }

                        view! {
                            <ul>
                                {list
                                    .into_iter()
                                    .map(|draft| render_draft_row(
                                        draft,
                                        publish_action,
                                        delete_action,
                                    ))
                                    .collect::<Vec<_>>()}
                            </ul>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            publish_action
                .value()
                .get()
                .map(|result: Result<PublishPostResult, ServerFnError>| match result {
                    Ok(published) => {
                        view! {
                            <p class="success">
                                "Post published. " <a href=published.permalink>"View permalink"</a>
                            </p>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
        {move || {
            delete_action
                .value()
                .get()
                .map(|result| match result {
                    Ok(()) => view! { <p class="success">"Draft deleted."</p> }.into_any(),
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
    }
}

fn render_draft_row(
    draft: DraftSummary,
    publish_action: ServerAction<PublishPost>,
    delete_action: ServerAction<DeletePost>,
) -> impl IntoView {
    let post_id = draft.post_id;
    let label = draft.title.clone().unwrap_or(draft.summary_label.clone());
    view! {
        <li>
            <strong>{label}</strong>
            " ("
            {draft.slug}
            ") "
            <a href=draft.preview_url>"Preview"</a>
            " "
            <a href=draft.edit_url>"Edit"</a>
            " "
            <a href=draft.permalink>"Permalink"</a>
            " "
            <div class="j-draft-actions">
                <ActionForm action=publish_action>
                    <input type="hidden" name="post_id" value=post_id />
                    <button type="submit" class="j-btn">
                        "Publish"
                    </button>
                </ActionForm>
                <ActionForm action=delete_action>
                    <input type="hidden" name="post_id" value=post_id />
                    <button
                        type="submit"
                        class="j-btn"
                        onclick="return confirm('Delete this draft?')"
                    >
                        "Delete"
                    </button>
                </ActionForm>
            </div>
        </li>
    }
}

fn render_timeline_post_row(post: TimelinePostSummary) -> impl IntoView {
    let title = post.title.clone();
    let permalink = post.permalink.clone();
    view! {
        <li data-test="timeline-item">
            {title
                .map(|title| {
                    view! {
                        <h2>
                            <a href=permalink>{title}</a>
                        </h2>
                    }
                })} <p class="metadata">"Published on " {post.published_at}</p>
            <div class="content" inner_html=post.rendered_html></div>
        </li>
    }
}

fn render_post_article(post: PostResponse, banner: Option<&'static str>) -> AnyView {
    let PostResponse {
        title,
        username,
        rendered_html,
        created_at,
        published_at,
        ..
    } = post;
    let profile_href = format!("/~{}/", username);
    let username_display = username.clone();
    let display_time = published_at
        .as_deref()
        .map(format_post_time)
        .unwrap_or_else(|| format_post_time(&created_at));

    // Inject a template <h1> only when the rendered body does not already open with one.
    // Markdown and org `* Heading` both produce <h1> in rendered_html; #+TITLE: does not.
    // This is a mild encapsulation violation — see design doc for rationale.
    let template_title = match title {
        Some(ref t) if !rendered_html.trim_start().starts_with("<h1") => Some(t.clone()),
        _ => None,
    };

    view! {
        <article>
            {template_title.map(|title| view! { <h1>{title}</h1> })}
            <p class="metadata">
                "By " <a href=profile_href>{username_display}</a> " on " {display_time}
            </p> {banner.map(|text| view! { <p class="draft-banner">{text}</p> })}
            <div class="content" inner_html=rendered_html></div>
        </article>
    }
    .into_any()
}

fn render_delete_form(
    delete_action: ServerAction<DeletePost>,
    post_id: i64,
    confirm_msg: &'static str,
) -> impl IntoView {
    view! {
        <ActionForm action=delete_action>
            <input type="hidden" name="post_id" value=post_id />
            <button type="submit" onclick=format!("return confirm('{confirm_msg}')")>
                "Delete"
            </button>
        </ActionForm>
    }
}

fn render_delete_result(
    delete_action: ServerAction<DeletePost>,
    success_msg: &'static str,
    success_href: &'static str,
    success_link_text: &'static str,
) -> impl IntoView {
    move || {
        delete_action.value().get().map(|result| match result {
            Ok(()) => view! { <p class="success">{success_msg} " " <a href=success_href>{success_link_text}</a></p> }
            .into_any(),
            Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
        })
    }
}
