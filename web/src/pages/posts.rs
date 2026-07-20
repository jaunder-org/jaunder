use crate::forms::Field;
use crate::media::MediaUpload;
use crate::subscriptions::{is_subscribed_to, SubscribeTo, UnsubscribeFrom};
use crate::tags::TagSummary;
use crate::{
    auth::current_user,
    error::WebError,
    feed_discovery::{FeedDiscovery, RsdDiscovery},
    pages::{
        signal_read::read_signal,
        ui::{
            publish_at_from_local, AudiencePicker, ComposerFields, PostCard, PostCreateForm,
            PostDisplay, TagContext, TagInput, Topbar,
        },
    },
    posts::{
        get_post, get_post_preview, list_drafts, list_posts_by_tag, list_user_posts,
        list_user_posts_by_tag, post_audience_selection, AudienceSelection, CreatePostResult,
        DeletePost, DraftSummary, ListPostsByTag, ListUserPosts, ListUserPostsByTag, PublishPost,
        PublishPostResult, TimelinePostSummary, UpdatePost, UpdatePostResult,
    },
};
use common::feed::FeedSurface;
use common::ids::PostId;
use common::pagination::PageSize;
use common::time::UtcInstant;
use common::visibility::AudienceBase;
use common::{slug::Slug, tag::Tag, username::Username};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[component]
pub fn CreatePostPage() -> impl IntoView {
    let current_user = crate::server_resource(|| (), |()| current_user());
    let last_result: RwSignal<Option<CreatePostResult>> = RwSignal::new(None);

    view! {
        <Topbar title="New post" sub="Long-form" />
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
                                        let slug_value = created.slug.to_string();
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

/// First-paint view for [`PostPage`]'s `Suspense`: the projector-seeded content
/// (flash-free) when the server painted this permalink, or a spinner while the
/// reactive fetch runs (client-side navigation, no seed).
// cov:ignore-start
fn permalink_first_paint(seed_post: Option<crate::posts::PostResponse>) -> AnyView {
    match seed_post {
        Some(post) => {
            // cov:ignore-stop
            // Just the article — this fallback sits inside the reactive PostPage's
            // own `j-scroll`/`j-page`. `display:contents` keeps the host wrapper out
            // of the layout so it coincides with the projector's permalink page.
            // cov:ignore-start
            let html = crate::render::permalink_article(&post);
            view! { <div style="display:contents" inner_html=html></div> }.into_any()
            // cov:ignore-stop
        }
        // cov:ignore-start
        None => view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any(),
        // cov:ignore-stop
    }
} // cov:ignore

#[component]
pub fn PostPage() -> impl IntoView {
    // Public projector seed (#178/#179): the content the server painted for this
    // permalink. Adopted as the `Suspense` fallback below so first paint shows
    // real content (flash-free) instead of a spinner. The reactive fetch still
    // runs and takes over — restoring the author's edit/delete affordances when
    // the viewer owns the post — so this *enhances* rather than *replaces*.
    let seed_post = match use_context::<Option<crate::render::PageSeed>>().flatten() {
        Some(crate::render::PageSeed::Permalink(post)) => Some(post),
        _ => None,
    };

    let params = use_params_map();

    let post_data = move || {
        let params = params.get();
        let raw_username = params.get("username").unwrap_or_default();
        let username = raw_username
            .strip_prefix('~')
            .and_then(|s| s.parse::<Username>().ok());
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
        // Parse the slug segment client-side so get_post takes a typed Slug
        // (ADR-0063 §4). A '~'-prefixed URL whose slug won't parse names no real
        // post, so the fetch is skipped below and the client 404s — no server
        // round-trip. The public projector backstops bare URLs with the SPA shell.
        let slug = params.get("slug").and_then(|s| s.parse::<Slug>().ok());
        (username, year, month, day, slug)
    };

    let post = crate::server_resource(
        post_data,
        |(username, year, month, day, slug): (Option<Username>, i32, u32, u32, Option<Slug>)| async move {
            let Some(username) = username else {
                // This is not a post permalink segment (it didn't start with '~').
                // It may be a server-handled URL (e.g. /media/…) that the SPA
                // router matched here because it has the same number of segments.
                // Reload the page so the server can handle it properly.
                if let Some(window) = web_sys::window() {
                    if let Ok(href) = window.location().href() {
                        let _ = window.location().replace(&href);
                    }
                }
                return Err(WebError::validation("Invalid permalink"));
            };
            // A '~'-prefixed permalink with an unparseable slug is a malformed
            // permalink (not a server URL): 404 client-side without calling the
            // server, matching the pre-typing behavior where get_post rejected it.
            let Some(slug) = slug else {
                return Err(WebError::validation("Invalid permalink"));
            };
            get_post(username, year, month, day, slug).await
        },
    );

    let on_unpublish = Callback::new(move |()| {
        if let Some(window) = web_sys::window() {
            let _ = window.location().replace("/drafts");
        }
    });

    view! {
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=move || permalink_first_paint(
                    seed_post.clone(),
                )>
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
                                    summary: fetched.summary.clone(),
                                    slug: fetched.slug.clone(),
                                    rendered_html: fetched.rendered_html.clone(),
                                    created_at: fetched.created_at,
                                    published_at: fetched
                                        .published_at
                                        .unwrap_or(fetched.created_at),
                                    permalink: fetched.permalink.clone().unwrap_or_default(),
                                    is_author: fetched.is_author,
                                    tags: fetched.tags.clone(),
                                };
                                let username_for_tags = fetched.username.clone();
                                view! {
                                    <PostCard
                                        post=summary
                                        banner=banner
                                        tag_context=TagContext::ForUser(username_for_tags)
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
    .into_any()
}

/// Subscribe / Unsubscribe control shown on a user's profile (timeline) page.
///
/// Hidden when the viewer is logged out or is viewing their own profile.
/// Otherwise renders Subscribe when not subscribed and Unsubscribe when
/// subscribed, querying state via `is_subscribed_to`.
#[component]
fn SubscribeButton(username: Username) -> impl IntoView {
    let subscribe = ServerAction::<SubscribeTo>::new();
    let unsubscribe = ServerAction::<UnsubscribeFrom>::new();

    // Re-query after either action mutates the subscription.
    let username_for_state = username.clone();
    let state = crate::server_resource(
        move || (subscribe.version().get(), unsubscribe.version().get()),
        move |_| {
            let username = username_for_state.clone();
            async move {
                let viewer = current_user().await.ok().flatten();
                let subscribed = is_subscribed_to(username.clone()).await.unwrap_or(false);
                (viewer, subscribed)
            }
        },
    );

    let profile_username = username;

    view! {
        <Suspense fallback=|| ()>
            {move || {
                let username = profile_username.clone();
                Suspend::new(async move {
                    let (viewer, subscribed) = state.await;
                    let show = match &viewer {
                        Some(name) => *name != username,
                        None => false,
                    };
                    if !show {
                        return ().into_any();
                    }
                    if subscribed {
                        view! {
                            <ActionForm action=unsubscribe>
                                <input
                                    type="hidden"
                                    name="author_username"
                                    value=username.to_string()
                                />
                                <button type="submit" class="j-btn">
                                    "Unsubscribe"
                                </button>
                            </ActionForm>
                        }
                            .into_any()
                    } else {
                        view! {
                            <ActionForm action=subscribe>
                                <input
                                    type="hidden"
                                    name="author_username"
                                    value=username.to_string()
                                />
                                <button type="submit" class="j-btn is-primary">
                                    "Subscribe"
                                </button>
                            </ActionForm>
                        }
                            .into_any()
                    }
                })
            }}
        </Suspense>
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
#[component]
pub fn UserTimelinePage() -> impl IntoView {
    let params = use_params_map();
    // Parse the `~username` route segment into `Username` once, at the source; an
    // invalid segment is `None` and every consumer handles the absence.
    let username = Memo::new(move |_| {
        params
            .get()
            .get("username")
            .unwrap_or_default()
            .strip_prefix('~')
            .and_then(|s| s.parse::<Username>().ok())
    });

    let mutate_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = crate::server_resource(
        move || (username.get(), mutate_version.get()),
        |(username, _)| async move {
            let username = username.ok_or_else(|| WebError::validation("Invalid username"))?;
            list_user_posts(username, None, None, Some(PageSize::default())).await
        },
    );

    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    let next_cursor_created_at = RwSignal::new(None::<UtcInstant>);
    let next_cursor_post_id = RwSignal::new(None::<PostId>);
    let has_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let initial_loaded = RwSignal::new(false);

    // Public projector seed (#178/#179): if the server painted this profile,
    // adopt its posts as the initial state so first paint shows content (no
    // Loading flash). Guarded on the username so a client-side navigation to a
    // *different* profile ignores the initial URL's seed; the reactive fetch
    // still runs and takes over.
    if let Some(crate::render::PageSeed::Profile {
        username: seed_user,
        page,
    }) = use_context::<Option<crate::render::PageSeed>>().flatten()
    {
        if username.get_untracked().as_ref() == Some(&seed_user) {
            next_cursor_created_at.set(page.next_cursor_created_at);
            next_cursor_post_id.set(page.next_cursor_post_id);
            has_more.set(page.has_more);
            timeline.set(page.posts);
            initial_loaded.set(true);
        }
    }

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
        let Some(username) = username.get_untracked() else {
            return;
        };
        if !has_more.get_untracked() {
            return;
        }
        load_more_action.dispatch(ListUserPosts {
            username,
            cursor_created_at: next_cursor_created_at.get_untracked(),
            cursor_post_id: next_cursor_post_id.get_untracked(),
            limit: Some(PageSize::default()),
        });
    };

    // The heading shows the canonical (parsed, lowercased) username, or empty for an
    // invalid segment — the page renders a validation error in that case anyway.
    let display_username = move || username.get().map(String::from).unwrap_or_default();
    let read_error = move || read_signal!(error);
    let read_initial_loaded = move || read_signal!(initial_loaded);
    let read_timeline = move || read_signal!(timeline);
    let read_has_more = move || read_signal!(has_more);
    let read_pending = move || read_signal!(load_more_action.pending());

    view! {
        {move || {
            username
                .get()
                .map(|username| {
                    view! {
                        <FeedDiscovery surface=FeedSurface::User {
                            username: username.clone(),
                        } />
                        <RsdDiscovery username=username />
                    }
                })
        }}
        <Topbar title=move || format!("Posts by {}", display_username()) sub="User timeline" />
        <div class="j-scroll">
            <div class="j-page">
                {move || {
                    username.get().map(|username| view! { <SubscribeButton username=username /> })
                }}
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
                                    let username_for_tags = post.username.clone();
                                    view! {
                                        <PostCard
                                            post=post
                                            banner=None
                                            tag_context=TagContext::ForUser(username_for_tags)
                                            on_mutate=on_mutate
                                        />
                                    }
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

#[component]
pub fn DraftPreviewPage() -> impl IntoView {
    let delete_action = ServerAction::<DeletePost>::new();
    let publish_action = ServerAction::<PublishPost>::new();
    let params = use_params_map();

    let preview = crate::server_resource(
        move || params.get(),
        |params| async move {
            let post_id = params
                .get("post_id")
                .and_then(|v| v.parse::<PostId>().ok())
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
                                    summary: fetched.summary.clone(),
                                    slug: fetched.slug.clone(),
                                    rendered_html: fetched.rendered_html.clone(),
                                    created_at: fetched.created_at,
                                    published_at: fetched
                                        .published_at
                                        .unwrap_or(fetched.created_at),
                                    permalink: fetched.permalink.clone().unwrap_or_default(),
                                    is_author: true,
                                    tags: fetched.tags.clone(),
                                };
                                let username_for_tags = fetched.username.clone();
                                view! {
                                    <PostDisplay
                                        post=summary
                                        banner=Some(
                                            "Draft preview – visible only to you".to_string(),
                                        )
                                        tag_context=TagContext::ForUser(username_for_tags)
                                    >
                                        <div class="j-post-acts">
                                            <ActionForm action=publish_action>
                                                <input
                                                    type="hidden"
                                                    name="post_id"
                                                    value=i64::from(post_id)
                                                />
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

#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
#[component]
pub fn EditPostPage() -> impl IntoView {
    let params = use_params_map();
    let update_post_action = ServerAction::<UpdatePost>::new();
    let body = RwSignal::new(String::new());
    let format = RwSignal::new("markdown".to_string());
    let slug_field = Field::<Slug>::optional();
    let summary = RwSignal::new(String::new());
    // Optional scheduled-publish time for an unpublished/draft post (naive
    // local wall-clock from a `datetime-local` control); empty publishes now.
    let publish_at = RwSignal::new(String::new());
    let post_tags: RwSignal<Vec<TagSummary>> = RwSignal::new(Vec::new());
    // Pre-selected with the post's current targeting (defaults to Public until
    // it resolves).
    let audience = RwSignal::new(AudienceSelection {
        base: AudienceBase::Public,
        named: Vec::new(),
    });
    // ServerAction dispatches happen only on the client; this redirect-on-publish
    // effect only ever fires there. `Effect::new_isomorphic` would needlessly
    // schedule on the server.
    Effect::new(move |_| {
        if let Some(Ok(ref updated)) = update_post_action.value().get() {
            if updated.published_at.is_some() {
                if let Some(ref permalink) = updated.permalink {
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().replace(permalink);
                    }
                }
            }
        }
    });

    // A missing or unparseable `post_id` is honest absence, not a real id: derive
    // `Option<PostId>` and short-circuit `None` to a client-side not-found in each
    // fetcher, rather than minting a sentinel id and paying a round-trip that only
    // ever returns not-found (#487).
    let post_id_param = move || {
        params
            .get()
            .get("post_id")
            .and_then(|v| v.parse::<PostId>().ok())
    };
    let post = crate::server_resource(post_id_param, |maybe_id| async move {
        match maybe_id {
            Some(id) => get_post_preview(id).await,
            None => Err(WebError::not_found("Post")),
        }
    });
    let current_audience = crate::server_resource(post_id_param, |maybe_id| async move {
        match maybe_id {
            Some(id) => post_audience_selection(id).await,
            None => Err(WebError::not_found("Post")),
        }
    });
    // Client-only: copying the resolved Resource into `audience` must not run
    // during SSR, where the future can resolve after the per-request reactive
    // owner is disposed (web-style-guide.md §9). The picker is seeded with the
    // post's current targeting on the client after hydration.
    Effect::new(move |_| {
        if let Some(Ok(selection)) = current_audience.get() {
            audience.set(selection);
        }
    });

    view! {
        <Topbar title="Edit Post" sub="Long-form" />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched) => {
                        body.set(String::from(fetched.body.clone()));
                        format.set(fetched.format.clone());
                        slug_field.value.set(fetched.slug.to_string());
                        summary.set(fetched.summary.clone().unwrap_or_default());
                        post_tags.set(fetched.tags.clone());
                        let post_id = fetched.post_id;
                        let is_published = fetched.published_at.is_some();
                        let dispatch_update = move |publish: bool| {
                            update_post_action
                                .dispatch(UpdatePost {
                                    post_id,
                                    body: body.get().into(),
                                    format: format.get(),
                                    slug_override: slug_field.parsed(),
                                    publish,
                                    publish_at: publish_at_from_local(&publish_at.get()),
                                    tags: Some(
                                        post_tags.get().into_iter().map(|t| t.display).collect(),
                                    ),
                                    summary: common::text::non_empty_owned(summary.get()),
                                    audience: Some(audience.get()),
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
                                                            prop:value=slug_field.value
                                                            on:input=move |ev| {
                                                                let v = event_target_value(&ev);
                                                                slug_field.value.set(v.clone());
                                                                slug_field.error.set(slug_field.error_for(&v));
                                                            }
                                                            on:blur=move |_| slug_field.touch()
                                                        />
                                                        {move || {
                                                            slug_field
                                                                .is_touched()
                                                                .then(|| slug_field.error.get())
                                                                .flatten()
                                                                .map(|msg| view! { <p class="error">{msg}</p> })
                                                        }}
                                                    </div>
                                                    // Optional schedule for a draft: a future
                                                    // time schedules it; empty publishes now.
                                                    <div style="margin-top:10px">
                                                        <label class="j-field-label" for="edit-publish-at">
                                                            "Publish at (optional)"
                                                        </label>
                                                        <input
                                                            id="edit-publish-at"
                                                            type="datetime-local"
                                                            class="j-field-val"
                                                            prop:value=publish_at
                                                            on:input=move |ev| {
                                                                publish_at.set(event_target_value(&ev));
                                                            }
                                                        />
                                                    </div>
                                                }
                                            })}
                                        <div style="margin-top:10px">
                                            <label class="j-field-label" for="edit-summary">
                                                "Summary"
                                            </label>
                                            <textarea
                                                id="edit-summary"
                                                name="summary"
                                                placeholder="Optional summary or excerpt"
                                                class="j-field-val"
                                                rows=3
                                                prop:value=summary
                                                on:input=move |ev| {
                                                    summary.set(event_target_value(&ev));
                                                }
                                            />
                                        </div>
                                        <div style="margin-top:10px">
                                            <TagInput tags=post_tags />
                                        </div>
                                        <div style="margin-top:10px">
                                            <AudiencePicker selection=audience />
                                        </div>
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
                                        <MediaUpload show_result=true />
                                    </div>
                                    <div class="j-edit-form-actions">
                                        {if is_published {
                                            view! {
                                                <button
                                                    class="j-btn is-primary"
                                                    type="button"
                                                    name="publish"
                                                    value="true"
                                                    prop:disabled=move || !slug_field.is_valid()
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
                                                    prop:disabled=move || !slug_field.is_valid()
                                                    on:click=move |_| dispatch_update(false)
                                                >
                                                    "Save draft"
                                                </button>
                                                <button
                                                    class="j-btn is-primary"
                                                    type="button"
                                                    name="publish"
                                                    value="true"
                                                    prop:disabled=move || !slug_field.is_valid()
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
                        let slug_value = updated.slug.to_string();
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

#[component]
pub fn DraftsPage() -> impl IntoView {
    let publish_action = ServerAction::<PublishPost>::new();
    let delete_action = ServerAction::<DeletePost>::new();
    let drafts = crate::server_resource(
        move || {
            (
                publish_action.version().get(),
                delete_action.version().get(),
            )
        },
        |_| list_drafts(None, None, Some(PageSize::default())),
    );

    view! {
        <Topbar title="Drafts" sub="Unpublished posts" />
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

// cov:ignore-start
fn render_draft_row(
    draft: DraftSummary,
    publish_action: ServerAction<PublishPost>,
    delete_action: ServerAction<DeletePost>,
) -> impl IntoView {
    let post_id = i64::from(draft.post_id);
    let label = draft
        .title
        .clone()
        .map(String::from)
        .unwrap_or(draft.summary_label.clone());
    // cov:ignore-stop
    // A scheduled post (future `published_at`) carries `scheduled_at`; mark it
    // distinctly from a true draft so the author can tell the two apart on this
    // shared "not-yet-live" surface. Full management UI is out of scope (#15).
    // cov:ignore-start
    let scheduled_badge = draft.scheduled_at.map(|when| {
        view! { <span class="j-badge j-badge-scheduled">{format!("Scheduled for {when}")}</span> }
    });
    view! {
        <li>
            <div class="j-draft-row">
                <div class="j-draft-row-content">
                    <strong>{label}</strong>
                    " ("
                    {draft.slug.to_string()}
                    ") "
                    {scheduled_badge}
                    " "
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
// cov:ignore-stop

// cov:ignore-start
fn render_delete_form(
    delete_action: ServerAction<DeletePost>,
    post_id: PostId,
    confirm_msg: &'static str,
) -> impl IntoView {
    let post_id = i64::from(post_id);
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
// cov:ignore-stop

// cov:ignore-start
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
// cov:ignore-stop

/// Site-wide listing of posts carrying a tag, at `/tags/:tag`.
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
#[component]
pub fn SiteTagPage() -> impl IntoView {
    let params = use_params_map();
    // Parse the `:tag` route segment into a canonical `Tag` once, at the source
    // (ADR-0063 §4); an unparseable segment is `None`, so the fetch below is
    // skipped and the client 404s — mirroring the `PostPage` slug parse.
    // `Tag::from_str` lowercases, so the heading and the projected render coincide.
    let tag = Memo::new(move |_| params.get().get("tag").and_then(|s| s.parse::<Tag>().ok()));

    let mutate_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = crate::server_resource(
        move || (tag.get(), mutate_version.get()),
        |(tag, _)| async move {
            let Some(tag) = tag else {
                return Err(WebError::validation("Invalid tag"));
            };
            list_posts_by_tag(tag, None, None, Some(PageSize::default())).await
        },
    );

    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    let next_cursor_created_at = RwSignal::new(None::<UtcInstant>);
    let next_cursor_post_id = RwSignal::new(None::<PostId>);
    let has_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let initial_loaded = RwSignal::new(false);

    // Public projector seed (#178/#179): adopt the seeded posts for a matching
    // tag so first paint shows content (guarded so a client-side nav to a
    // different tag ignores the initial URL's seed); the reactive fetch still runs.
    if let Some(crate::render::PageSeed::SiteTag {
        tag: seed_tag,
        page,
    }) = use_context::<Option<crate::render::PageSeed>>().flatten()
    {
        if tag.get_untracked().as_ref() == Some(&seed_tag) {
            next_cursor_created_at.set(page.next_cursor_created_at);
            next_cursor_post_id.set(page.next_cursor_post_id);
            has_more.set(page.has_more);
            timeline.set(page.posts);
            initial_loaded.set(true);
        }
    }

    let load_more_action = ServerAction::<ListPostsByTag>::new();

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
        let Some(tag_value) = tag.get_untracked() else {
            return;
        };
        if !has_more.get_untracked() {
            return;
        }
        load_more_action.dispatch(ListPostsByTag {
            tag: tag_value,
            cursor_created_at: next_cursor_created_at.get_untracked(),
            cursor_post_id: next_cursor_post_id.get_untracked(),
            limit: Some(PageSize::default()),
        });
    };

    // The canonical tag for the heading (a newtype is not `IntoRender`), or empty
    // for an unparseable segment — the page renders a validation error anyway.
    let read_tag = move || read_signal!(tag).map(|t| t.to_string()).unwrap_or_default();
    let read_error = move || read_signal!(error);
    let read_initial_loaded = move || read_signal!(initial_loaded);
    let read_timeline = move || read_signal!(timeline);
    let read_has_more = move || read_signal!(has_more);
    let read_pending = move || read_signal!(load_more_action.pending());

    view! {
        {move || {
            view! {
                {tag
                    .get()
                    .map(|tag| view! { <FeedDiscovery surface=FeedSurface::SiteTag { tag } /> })}
            }
        }}
        <Topbar
            title=Signal::derive(move || format!("#{}", read_tag()))
            sub="Posts on this instance"
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
                        return view! { <p>"No posts with this tag yet."</p> }.into_any();
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

/// Per-user listing of posts carrying a tag, at `/~:username/tags/:tag`.
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view fn; length is inherent to the view! markup — splitting into \
              sub-components would fragment the page without real benefit"
)]
#[component]
pub fn UserTagPage() -> impl IntoView {
    let params = use_params_map();
    // Parse the `~username` route segment into `Username` once, at the source; an
    // invalid segment is `None` and every consumer handles the absence.
    let username = Memo::new(move |_| {
        params
            .get()
            .get("username")
            .unwrap_or_default()
            .strip_prefix('~')
            .and_then(|s| s.parse::<Username>().ok())
    });
    // Parse the `:tag` route segment into a canonical `Tag` once, at the source
    // (ADR-0063 §4); an unparseable segment is `None`, so the fetch below is
    // skipped and the client 404s — mirroring the `PostPage` slug parse.
    // `Tag::from_str` lowercases, so the heading and the projected render coincide.
    let tag = Memo::new(move |_| params.get().get("tag").and_then(|s| s.parse::<Tag>().ok()));

    let mutate_version = RwSignal::new(0u32);
    let on_mutate = Callback::new(move |()| mutate_version.update(|v| *v += 1));

    let initial_page = crate::server_resource(
        move || (username.get(), tag.get(), mutate_version.get()),
        |(username, tag, _)| async move {
            let username = username.ok_or_else(|| WebError::validation("Invalid username"))?;
            let Some(tag) = tag else {
                return Err(WebError::validation("Invalid tag"));
            };
            list_user_posts_by_tag(username, tag, None, None, Some(PageSize::default())).await
        },
    );

    let timeline = RwSignal::new(Vec::<TimelinePostSummary>::new());
    let next_cursor_created_at = RwSignal::new(None::<UtcInstant>);
    let next_cursor_post_id = RwSignal::new(None::<PostId>);
    let has_more = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let initial_loaded = RwSignal::new(false);

    // Public projector seed (#178/#179): adopt the seeded posts for a matching
    // username+tag so first paint shows content; the reactive fetch still runs.
    if let Some(crate::render::PageSeed::UserTag {
        username: seed_user,
        tag: seed_tag,
        page,
    }) = use_context::<Option<crate::render::PageSeed>>().flatten()
    {
        if username.get_untracked().as_ref() == Some(&seed_user)
            && tag.get_untracked().as_ref() == Some(&seed_tag)
        {
            next_cursor_created_at.set(page.next_cursor_created_at);
            next_cursor_post_id.set(page.next_cursor_post_id);
            has_more.set(page.has_more);
            timeline.set(page.posts);
            initial_loaded.set(true);
        }
    }

    let load_more_action = ServerAction::<ListUserPostsByTag>::new();

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
        let Some(username_value) = username.get_untracked() else {
            return;
        };
        let Some(tag_value) = tag.get_untracked() else {
            return;
        };
        if !has_more.get_untracked() {
            return;
        }
        load_more_action.dispatch(ListUserPostsByTag {
            username: username_value,
            tag: tag_value,
            cursor_created_at: next_cursor_created_at.get_untracked(),
            cursor_post_id: next_cursor_post_id.get_untracked(),
            limit: Some(PageSize::default()),
        });
    };

    // Canonical (parsed, lowercased) username for the heading, or empty for an
    // invalid segment — the page renders a validation error in that case anyway.
    let read_username = move || username.get().map(String::from).unwrap_or_default();
    // The canonical tag for the heading (a newtype is not `IntoRender`), or empty
    // for an unparseable segment — the page renders a validation error anyway.
    let read_tag = move || read_signal!(tag).map(|t| t.to_string()).unwrap_or_default();
    let read_error = move || read_signal!(error);
    let read_initial_loaded = move || read_signal!(initial_loaded);
    let read_timeline = move || read_signal!(timeline);
    let read_has_more = move || read_signal!(has_more);
    let read_pending = move || read_signal!(load_more_action.pending());

    view! {
        {move || {
            view! {
                {username
                    .get()
                    .zip(tag.get())
                    .map(|(username, tag)| {
                        view! {
                            <FeedDiscovery surface=FeedSurface::UserTag {
                                username,
                                tag,
                            } />
                        }
                    })}
            }
        }}
        <Topbar
            title=Signal::derive(move || format!("#{}", read_tag()))
            sub=Signal::derive(move || format!("Posts by ~{}", read_username()))
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
                        return view! { <p>"No posts with this tag yet."</p> }.into_any();
                    }
                    view! {
                        <div>
                            {rows
                                .into_iter()
                                .map(|post| {
                                    let username_for_tags = post.username.clone();
                                    view! {
                                        <PostCard
                                            post=post
                                            banner=None
                                            tag_context=TagContext::ForUser(username_for_tags)
                                            on_mutate=on_mutate
                                        />
                                    }
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
