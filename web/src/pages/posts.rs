use crate::{
    auth::current_user,
    error::WebError,
    pages::{
        signal_read::read_signal,
        ui::{ComposerFields, PostCard, PostCreateForm, PostDisplay, Topbar},
        MediaPanel,
    },
    posts::{
        get_post, get_post_preview, list_drafts, list_user_posts, CreatePostResult, DeletePost,
        DraftSummary, ListUserPosts, PublishPost, PublishPostResult, TimelinePostSummary,
        UpdatePost, UpdatePostResult,
    },
};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[allow(clippy::must_use_candidate)]
#[component]
#[must_use]
pub fn CreatePostPage() -> impl IntoView {
    let current_user = Resource::new(|| (), |()| current_user());
    let last_result: RwSignal<Option<CreatePostResult>> = RwSignal::new(None);

    view! {
        <Topbar title="New post".to_string() sub="Long-form".to_string() />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match current_user.await {
                    Ok(Some(_)) => {
                        view! {
                            <PostCreateForm
                                compact=false
                                rows=16
                                placeholder="Write something\u{2026}"
                                on_success=Callback::new(move |created| {
                                    last_result.set(Some(created));
                                })
                            />
                            {move || {
                                last_result
                                    .get()
                                    .map(|created| {
                                        let message = if created.published_at.is_some() {
                                            "Post published."
                                        } else {
                                            "Draft saved."
                                        };
                                        let slug_value = created.slug.clone();
                                        let slug_for_attr = slug_value.clone();
                                        view! {
                                            <div class="j-save-summary">
                                                <p class="success">{message}</p>
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

#[allow(clippy::must_use_candidate)]
#[component]
#[must_use]
pub fn PostPage() -> impl IntoView {
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
                _ => {
                    // This is not a post permalink segment (it didn't start with '~').
                    // It may be a server-handled URL (e.g. /media/…) that the SPA
                    // router matched here because it has the same number of segments.
                    // Reload the page so the server can handle it properly.
                    #[cfg(target_arch = "wasm32")]
                    if let Some(window) = web_sys::window() {
                        if let Ok(href) = window.location().href() {
                            let _ = window.location().replace(&href);
                        }
                    }
                    return Err(WebError::validation("Invalid permalink"));
                }
            };
            get_post(username, year, month, day, slug).await
        },
    );

    let on_unpublish = Callback::new(move |()| {
        #[cfg(target_arch = "wasm32")]
        if let Some(window) = web_sys::window() {
            let _ = window.location().replace("/drafts");
        }
    });

    view! {
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match post.await {
                            Ok(fetched) => {
                                let banner = fetched
                                    .is_draft
                                    .then_some("Draft - visible only to you".to_string());
                                let summary = TimelinePostSummary {
                                    post_id: fetched.post_id,
                                    username: fetched.username.clone(),
                                    title: fetched.title.clone(),
                                    slug: fetched.slug.clone(),
                                    rendered_html: fetched.rendered_html.clone(),
                                    created_at: fetched.created_at.clone(),
                                    published_at: fetched
                                        .published_at
                                        .clone()
                                        .unwrap_or_else(|| fetched.created_at.clone()),
                                    permalink: fetched.permalink.clone().unwrap_or_default(),
                                    is_author: fetched.is_author,
                                };
                                view! {
                                    <PostCard
                                        post=summary
                                        banner=banner
                                        on_unpublish=on_unpublish
                                    />
                                }
                                    .into_any()
                            }
                            Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                        }
                    })}
                </Suspense>
            </div>
        </div>
    }
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::must_use_candidate)]
#[component]
#[must_use]
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

    let mutate_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = Resource::new(
        move || (username.get(), mutate_version.get()),
        |(username, _)| async move {
            if username.is_empty() {
                return Err(WebError::validation("Invalid username"));
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

    // Client-only: `Effect::new_isomorphic` would race with SSR reactive-owner
    // disposal because the Resource future can resolve on a tokio worker after
    // the per-request owner is gone, panicking on signal access. SSR renders
    // the loading placeholder; signals seed on the client after hydration.
    Effect::new(move |_| {
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

    // ServerAction dispatches happen only on the client, so this effect's body
    // never fires server-side; using `Effect::new` matches that reality.
    Effect::new(move |_| {
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
        <Topbar
            title=move || format!("Posts by {}", display_username())
            sub="User timeline".to_string()
        />
        <div class="j-scroll">
            <div class="j-page">
                {move || {
                    if let Some(err) = read_error() {
                        return view! { <p class="error">{err}</p> }.into_any();
                    }
                    if !read_initial_loaded() {
                        return view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any();
                    }
                    let rows = read_timeline();
                    if rows.is_empty() {
                        return view! { <p>"No posts yet."</p> }.into_any();
                    }
                    view! {
                        <div>
                            {rows
                                .into_iter()
                                .map(|post| {
                                    view! { <PostCard post=post banner=None on_mutate=on_mutate /> }
                                })
                                .collect::<Vec<_>>()}
                        </div>
                        {move || {
                            read_has_more()
                                .then(|| {
                                    view! {
                                        <button on:click=on_load_more disabled=read_pending>
                                            {move || {
                                                if read_pending() { "Loading\u{2026}" } else { "Load more" }
                                            }}
                                        </button>
                                    }
                                })
                        }}
                    }
                        .into_any()
                }}
            </div>
        </div>
    }
}

#[allow(clippy::must_use_candidate)]
#[component]
#[must_use]
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
                .ok_or_else(|| WebError::validation("Invalid preview"))?;
            get_post_preview(post_id).await
        },
    );

    view! {
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match preview.await {
                            Ok(fetched) => {
                                let post_id = fetched.post_id;
                                let summary = TimelinePostSummary {
                                    post_id: fetched.post_id,
                                    username: fetched.username.clone(),
                                    title: fetched.title.clone(),
                                    slug: fetched.slug.clone(),
                                    rendered_html: fetched.rendered_html.clone(),
                                    created_at: fetched.created_at.clone(),
                                    published_at: fetched
                                        .published_at
                                        .clone()
                                        .unwrap_or_else(|| fetched.created_at.clone()),
                                    permalink: fetched.permalink.clone().unwrap_or_default(),
                                    is_author: true,
                                };
                                view! {
                                    <PostDisplay
                                        post=summary
                                        banner=Some(
                                            "Draft preview – visible only to you".to_string(),
                                        )
                                    >
                                        <div class="j-post-acts">
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
                                            {render_delete_form(
                                                delete_action,
                                                post_id,
                                                "Delete this draft?",
                                            )}
                                        </div>
                                    </PostDisplay>
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
                        .map(|result: Result<PublishPostResult, WebError>| match result {
                            Ok(published) => {
                                view! {
                                    <p class="success">
                                        "Post published. "
                                        <a href=published.permalink>"View post"</a>
                                    </p>
                                }
                                    .into_any()
                            }
                            Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                        })
                }}
                {render_delete_result(delete_action, "Draft deleted.", "/drafts", "Go to drafts")}
            </div>
        </div>
    }
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::must_use_candidate)]
#[component]
pub fn EditPostPage() -> impl IntoView {
    let params = use_params_map();
    let update_post_action = ServerAction::<UpdatePost>::new();
    let body = RwSignal::new(String::new());
    let format = RwSignal::new("markdown".to_string());
    let slug_override = RwSignal::new(String::new());
    // ServerAction dispatches happen only on the client; this redirect-on-publish
    // effect only ever fires there. `Effect::new_isomorphic` would needlessly
    // schedule on the server.
    Effect::new(move |_| {
        if let Some(Ok(ref updated)) = update_post_action.value().get() {
            if updated.published_at.is_some() {
                #[cfg(target_arch = "wasm32")]
                if let Some(ref permalink) = updated.permalink {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().replace(permalink);
                    }
                }
            }
        }
    });

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
        <Topbar title="Edit Post".to_string() sub="Long-form".to_string() />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched) => {
                        body.set(fetched.body.clone());
                        format.set(fetched.format.clone());
                        slug_override.set(fetched.slug.clone());
                        let post_id = fetched.post_id;
                        let is_published = fetched.published_at.is_some();
                        let dispatch_update = move |publish: bool| {
                            let slug = slug_override.get();
                            let slug_override_arg = if slug.trim().is_empty() {
                                None
                            } else {
                                Some(slug)
                            };
                            update_post_action
                                .dispatch(UpdatePost {
                                    post_id,
                                    body: body.get(),
                                    format: format.get(),
                                    slug_override: slug_override_arg,
                                    publish,
                                });
                        };
                        view! {
                            <div class="j-edit-form-grid">
                                <div class="j-edit-form-body">
                                    <ComposerFields
                                        body=body
                                        format=format
                                        rows=20
                                        show_seg=false
                                    />
                                </div>
                                <aside class="j-edit-form-aside">
                                    <div>
                                        <div class="j-sb-head" style="padding:0 0 10px">
                                            "Options"
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
                                                            prop:value=slug_override
                                                            on:input=move |ev| {
                                                                slug_override.set(event_target_value(&ev));
                                                            }
                                                        />
                                                    </div>
                                                }
                                            })}
                                        <div class="j-seg" style="margin-top:10px">
                                            <button
                                                type="button"
                                                class=move || {
                                                    if format.get() == "markdown" {
                                                        "j-btn is-selected"
                                                    } else {
                                                        "j-btn"
                                                    }
                                                }
                                                on:click=move |_| { format.set("markdown".to_string()) }
                                            >
                                                "Markdown"
                                            </button>
                                            <button
                                                type="button"
                                                class=move || {
                                                    if format.get() == "org" {
                                                        "j-btn is-selected"
                                                    } else {
                                                        "j-btn"
                                                    }
                                                }
                                                on:click=move |_| format.set("org".to_string())
                                            >
                                                "Org"
                                            </button>
                                        </div>
                                    </div>
                                    <div style="margin-top:16px">
                                        <div class="j-sb-head" style="padding:0 0 10px">
                                            "Media"
                                        </div>
                                        <MediaPanel />
                                    </div>
                                    <div class="j-edit-form-actions">
                                        {if is_published {
                                            view! {
                                                <button
                                                    class="j-btn is-primary"
                                                    type="button"
                                                    name="publish"
                                                    value="true"
                                                    on:click=move |_| dispatch_update(true)
                                                >
                                                    "Save"
                                                </button>
                                            }
                                                .into_any()
                                        } else {
                                            view! {
                                                <button
                                                    class="j-btn"
                                                    type="button"
                                                    name="publish"
                                                    value="false"
                                                    on:click=move |_| dispatch_update(false)
                                                >
                                                    "Save draft"
                                                </button>
                                                <button
                                                    class="j-btn is-primary"
                                                    type="button"
                                                    name="publish"
                                                    value="true"
                                                    on:click=move |_| dispatch_update(true)
                                                >
                                                    "Publish"
                                                </button>
                                            }
                                                .into_any()
                                        }}
                                    </div>
                                </aside>
                            </div>
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
                .map(|result: Result<UpdatePostResult, WebError>| match result {
                    Ok(updated) if updated.published_at.is_none() => {
                        let slug_value = updated.slug.clone();
                        let slug_for_attr = slug_value.clone();
                        view! {
                            <div class="j-save-summary">
                                <p class="success">"Draft saved."</p>
                                <p data-test="slug-value" data-slug=slug_for_attr>
                                    "Slug: "
                                    {slug_value}
                                </p>
                                <a data-test="preview-link" href=updated.preview_url.clone()>
                                    "Preview draft"
                                </a>
                            </div>
                        }
                            .into_any()
                    }
                    Ok(_) => view! { <p>"Redirecting\u{2026}"</p> }.into_any(),
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
    }
}

#[allow(clippy::must_use_candidate)]
#[component]
#[must_use]
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
        <Topbar title="Drafts".to_string() sub="Unpublished posts".to_string() />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match drafts.await {
                            Ok(list) => {
                                if list.is_empty() {
                                    return view! { <p>"You have no drafts."</p> }.into_any();
                                }
                                view! {
                                    <ul class="j-draft-list">
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
                        .map(|result: Result<PublishPostResult, WebError>| match result {
                            Ok(published) => {
                                view! {
                                    <p class="success">
                                        "Post published. "
                                        <a href=published.permalink>"View permalink"</a>
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
            </div>
        </div>
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
            <div class="j-draft-row">
                <div class="j-draft-row-content">
                    <strong>{label}</strong>
                    " ("
                    {draft.slug}
                    ") "
                    <a href=draft.preview_url>"Preview"</a>
                    " "
                    <a href=draft.permalink>"Permalink"</a>
                </div>
                <div class="j-draft-actions">
                    <a class="j-btn" href=draft.edit_url>
                        "Edit"
                    </a>
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
                            class="j-btn is-danger"
                            onclick="return confirm('Delete this draft?')"
                        >
                            "Delete"
                        </button>
                    </ActionForm>
                </div>
            </div>
        </li>
    }
}

fn render_delete_form(
    delete_action: ServerAction<DeletePost>,
    post_id: i64,
    confirm_msg: &'static str,
) -> impl IntoView {
    view! {
        <ActionForm action=delete_action>
            <input type="hidden" name="post_id" value=post_id />
            <button
                type="submit"
                class="j-btn is-danger"
                onclick=format!("return confirm('{confirm_msg}')")
            >
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
