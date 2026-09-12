use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{self, use_navigate, use_params_map};

use crate::error::WebError;
use crate::forms::Field;
use crate::posts;
use crate::posts::{
    ComposeState, EditPublicationState, NamedAudienceState, PublicationIntent, SavedPost,
};
use crate::taglist::TagCtx;
use crate::topbar::Topbar;
use common::ids::PostId;
use common::post_body::PostBody;
use common::root_relative_url::RootRelativeUrl;
use common::seed::{AuthoredPost, PageSeed};
use common::slug::Slug;
use common::time::{self, UtcInstant};
use common::{MutationOutcome, permalink_route::PermalinkRoute};

use super::audience;
use super::composers::{ComposeOptions, ComposerActions, ComposerCore};
use super::display::PostCard;
use super::support;

/// First-paint view for [`PostPage`]'s `Suspense`: the projector-seeded content
/// (flash-free) when the server painted this permalink, or a spinner while the
/// reactive fetch runs (client-side navigation, no seed).
fn permalink_first_paint(
    seed_post: Option<AuthoredPost>,
    theme: RwSignal<common::theme::PublishedThemePresentation>,
) -> AnyView {
    match seed_post {
        Some(seed) => {
            let title = format!("Post by {}", seed.post.username);
            let html = posts::render::permalink_article(&seed.post);
            view! {
                <Topbar title=title />
                {move || {
                    crate::app::render_theme_header(&theme.get())
                        .inject_into(leptos::html::div().class("j-contents"))
                }}
                <div class="j-scroll">
                    <div class="j-page">
                        {html.inject_into(leptos::html::div().class("j-contents"))}
                    </div>
                </div>
            }
            .into_any()
        }
        None => view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any(),
    }
}

/// `PostPage`'s resource key: the decoded permalink route plus a refetch tick. The tick
/// is folded into the key (not merely tracked) so an in-place publish — same URL, so the
/// route is unchanged — still changes the key and forces a re-fetch (#592). Named so the
/// fetcher's parameter reads as one thing.
type PermalinkFetchKey = (Option<PermalinkRoute>, u32);

/// The projector seed is valid only for the exact typed permalink now mounted.
fn permalink_seed_matches_route(seed: &AuthoredPost, route: Option<&PermalinkRoute>) -> bool {
    route.is_some_and(|route| {
        route
            == &PermalinkRoute {
                username: seed.post.username.clone(),
                date: jiff::tz::Offset::UTC
                    .to_datetime(seed.post.display_time().value())
                    .date()
                    .into(),
                slug: seed.post.slug.clone(),
            }
    })
}
fn permalink_unpublish_callback(refetch: RwSignal<u32>) -> Callback<SavedPost> {
    let location = hooks::use_location();
    let navigate = use_navigate();
    Callback::new(move |unpublished: SavedPost| {
        posts::refetch_unpublished_post_if_needed(
            &location.pathname.get_untracked(),
            &unpublished.permalink,
            || refetch.update(|v| *v += 1),
        );
        navigate(
            &unpublished.permalink,
            NavigateOptions {
                replace: true,
                ..NavigateOptions::default()
            },
        );
    })
}

/// Server-classified publication notice for an author-visible Scheduled Post.
#[component]
fn ScheduledNotice(at: Option<UtcInstant>) -> impl IntoView {
    view! {
        {move || {
            at.map(|at| {
                view! {
                    <p class="success">
                        {format!(
                            "Scheduled for {} local time",
                            time::local_datetime_from_utc(at).replace('T', " "),
                        )}
                    </p>
                }
            })
        }}
    }
}

#[component]
pub fn PostPage() -> impl IntoView {
    let presentation = crate::app::theme_presentation();
    let theme = crate::app::public_theme();
    // The projector seed is a fallback only for its exact typed permalink route.
    let seed_post = match use_context::<Option<PageSeed>>().flatten() {
        Some(PageSeed::Permalink(post)) => Some(post),
        _ => None,
    };

    let params = use_params_map();

    let route = move || {
        let params = params.get();
        // Keep the public API boundary typed (ADR-0063 §4).
        posts::parse_permalink_route(
            params.get("username").as_deref(),
            params.get("year").as_deref(),
            params.get("month").as_deref(),
            params.get("day").as_deref(),
            params.get("slug").as_deref(),
        )
    };
    let fallback_route = route;

    // Force a refetch after an in-place mutation such as same-URL publication (#592).
    let refetch = RwSignal::new(0u32);
    let post = Resource::new(
        move || (route(), refetch.get()),
        move |(route, _): PermalinkFetchKey| async move {
            posts::permalink_destination(route, |route| {
                posts::get(route.username, route.date, route.slug)
            })
            .await
        },
    );
    Effect::new(move |_| {
        if post.try_get().flatten().is_none() {
            presentation.begin_navigation();
        }
    });

    let on_unpublish = permalink_unpublish_callback(refetch);
    // Publish-only: refetch this page in place when navigation is a no-op (#592).
    let on_publish = Callback::new(move |()| refetch.update(|v| *v += 1));

    view! {
        <Suspense fallback=move || {
            let current_route = fallback_route();
            let matching_seed = seed_post
                .clone()
                .filter(|seed| permalink_seed_matches_route(seed, current_route.as_ref()));
            permalink_first_paint(matching_seed, theme)
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok((fetched_theme, fetched)) => {
                        match presentation.adopt(fetched_theme).await {
                            Ok(crate::app::ThemeAdoption::Applied) => {
                                let scheduled_at = posts::scheduled_publication_at(
                                    fetched.post.post.published_at,
                                    fetched.fetched_at,
                                );
                                let banner = fetched
                                    .post
                                    .post
                                    .is_draft()
                                    .then_some("Draft - visible only to you".to_string());
                                let tag_context = TagCtx::ForUser(
                                    fetched.post.post.username.clone(),
                                );
                                // Both bound before the `view!`: the props are borrows
                                // now, so an inline temporary would be dropped inside
                                // the macro expansion (E0716).
                                view! {
                                    <Topbar title=format!(
                                        "Post by {}",
                                        fetched.post.post.username,
                                    ) />
                                    {move || {
                                        crate::app::render_theme_header(&theme.get())
                                            .inject_into(leptos::html::div().class("j-contents"))
                                    }}
                                    <div class="j-scroll">
                                        <div class="j-page">
                                            <ScheduledNotice at=scheduled_at />
                                            <PostCard
                                                post=&fetched.post.post
                                                banner=banner.as_deref()
                                                tag_context=&tag_context
                                                on_unpublish=on_unpublish
                                                on_publish=on_publish
                                            />
                                        </div>
                                    </div>
                                }
                                    .into_any()
                            }
                            Ok(crate::app::ThemeAdoption::Superseded) => {
                                view! { <p class="j-loading">"Loading\u{2026}"</p> }.into_any()
                            }
                            Err(error) => {
                                view! { <p class="error">{error.to_string()}</p> }.into_any()
                            }
                        }
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
    .into_any()
}

#[component]
pub fn EditPostPage() -> impl IntoView {
    let params = use_params_map();
    let update_post_action = ServerAction::<posts::Update>::new();
    // The editor edits the same seven fields the composer does and dispatches the
    // same `PostInputs` payload, so it reuses that bundle (#301). The slug field is
    // page-level here — unlike the composer, where only the full shape has one.
    let state = ComposeState::new();
    let slug_field = Field::<Slug>::optional();
    let named = audience::load_named_audiences();
    let lifecycle = posts::EditLifecycleState::new();
    // The redirect-on-publish effect reacts to the client-only ServerAction
    // dispatch. Whether a settled update redirects at all, and to where, is the
    // host-tested `publish_redirect` (#306), leaving this the bare `Effect`
    // `on_settled_ok` wraps.
    let navigate = use_navigate();
    support::on_settled_ok(
        move || posts::publish_redirect(update_post_action.value().get()),
        move |permalink: RootRelativeUrl| navigate(&permalink, NavigateOptions::default()),
    );
    // The host-tested lifecycle state owns the transition; this Effect is only
    // reactive wiring from the server action settlement.
    support::on_settled(
        move || update_post_action.value().get(),
        move |settlement: Result<MutationOutcome<SavedPost>, WebError>| {
            lifecycle.adopt_settlement(&settlement);
        },
    );

    // A missing or unparseable `post_id` is honest absence, not a real id: derive
    // `Option<PostId>` and let the host-tested `with_post_id` short-circuit `None`
    // to a client-side not-found, rather than minting a sentinel id and paying a
    // round-trip that only ever returns not-found (#487).
    let post_id_param = move || {
        params
            .get()
            .get("post_id")
            .and_then(|v| v.parse::<PostId>().ok())
    };
    let post = Resource::new(post_id_param, |post_id| {
        posts::with_post_id(post_id, posts::get_preview)
    });
    // Seeded into the editable `audience` picker inside the `Suspense` block below
    // (awaited alongside `post`, not via a standalone Effect, since the page
    // already suspends on `post`). On a fetch error the Public default survives
    // (the `Ok`-only guard mirrors the dissolved post-resolve Effect). The intent
    // comment lives here, outside `view!`, because leptosfmt relocates comments
    // inside the macro.
    let current_audience = Resource::new(post_id_param, |post_id| {
        posts::with_post_id(post_id, posts::get_audience_selection)
    });

    view! {
        <Topbar title="Edit Post" sub="Long-form" />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched) => {
                        state.seed_from(&fetched.post);
                        slug_field.set_value(fetched.post.post.slug.as_ref());
                        let publication = EditPublicationState::from_loaded(
                            posts::loaded_publication(
                                fetched.post.post.published_at,
                                fetched.fetched_at,
                            ),
                            state.publish_at,
                        );
                        if let Ok(selection) = current_audience.await {
                            state.audience.set(selection);
                        }
                        let post_id = fetched.post.post.post_id;
                        // The slug is not part of the bundle (the compact shape has
                        // none) — see `seed_from`. A confirmed Unpublish is
                        // authoritative for the new Draft branch, while the fields
                        // already hold the exact values sent in that atomic update.
                        view! {
                            {move || {
                                let current_publication = lifecycle
                                    .current_publication(publication, state.publish_at);
                                view! {
                                    <EditPostForm
                                        state=state
                                        slug_field=slug_field
                                        post_id=post_id
                                        publication=current_publication
                                        action=update_post_action
                                        named=named
                                    />
                                }
                            }}
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        <EditSaveOutcome action=update_post_action />
    }
}

/// The editor's form: body column, publication-aware options, media, and save controls.
/// Split out of [`EditPostPage`] (#301), which keeps only the fetch and its branch.
#[component]
fn EditPostForm(
    state: ComposeState,
    /// Page-level rather than held by [`ComposeState`] — see
    /// [`ComposeState::seed_from`].
    slug_field: Field<Slug>,
    post_id: PostId,
    /// Publication branch and branch-specific signals fixed when the response loaded.
    publication: EditPublicationState,
    /// The named-audience load shared by the picker and the action gate.
    named: RwSignal<NamedAudienceState>,
    action: ServerAction<posts::Update>,
) -> impl IntoView {
    // The body/field gate also waits for a real named-audience load. Repeating
    // the pure guard in the callback prevents a direct invocation from
    // dispatching while Loading or Failed.
    let also_blocked = Signal::derive(move || {
        !slug_field.is_valid()
            || !state.summary_field.is_valid()
            || state.audience.with(|selection| {
                named.with(|state| state.selection_for_submit(selection).is_none())
            })
    });
    let loaded_publication = publication.loaded();
    let publication_time = publication.publication_time();
    let (save_disabled, unpublish_disabled, schedule_error, dispatch_update) =
        posts::edit_submit_gate(
            state.body,
            also_blocked,
            publication,
            Callback::new(move |(body, publication): (PostBody, PublicationIntent)| {
                if state.audience.with(|selection| {
                    named.with(|state| state.selection_for_submit(selection).is_some())
                }) {
                    action.dispatch(posts::Update {
                        post_id,
                        post: state.inputs(body, publication, slug_field.parsed()),
                    });
                }
            }),
        );
    view! {
        <div class="j-compose-grid">
            <div class="j-compose-body">
                <ComposerCore
                    state=state
                    actions=ComposerActions::Save {
                        publication: loaded_publication,
                        disabled: save_disabled,
                        unpublish_disabled,
                        on_save: dispatch_update,
                    }
                    rows=20
                    placeholder="Write something\u{2026}"
                    textarea_class="j-edit-form-textarea"
                />
            </div>
            <aside class="j-compose-aside">
                <ComposeOptions
                    state=state
                    slug_field=slug_field
                    publication=loaded_publication
                    publication_time=publication_time
                    schedule_error=schedule_error
                    creation_schedule=None
                    named=named
                />
            </aside>
        </div>
    }
}

/// The save summary under the editor: the draft-saved block (slug + permalink) when a
/// draft save is confirmed, a "Redirecting…" notice when confirmed publication is about
/// to navigate away, or error-like refresh guidance when the commit is indeterminate.
/// A server error is rendered directly, and nothing appears until the action has a value.
///
/// Split out of [`EditPostPage`] (#306); this component owns that outcome decision.
#[component]
fn EditSaveOutcome(action: ServerAction<posts::Update>) -> impl IntoView {
    view! {
        {move || {
            action
                .value()
                .get()
                .map(|result: Result<MutationOutcome<SavedPost>, WebError>| match result {
                    Ok(MutationOutcome::Confirmed(updated)) if updated.published_at.is_none() => {
                        let slug_value = updated.slug.to_string();
                        let slug_for_attr = slug_value.clone();
                        view! {
                            <div class="j-save-summary">
                                <p class="success">"Draft saved."</p>
                                <p data-test="slug-value" data-slug=slug_for_attr>
                                    "Slug: "
                                    {slug_value}
                                </p>
                                <a data-test="permalink-link" href=updated.permalink.to_string()>
                                    "View post"
                                </a>
                            </div>
                        }
                            .into_any()
                    }
                    Ok(MutationOutcome::Confirmed(_)) => {
                        view! { <p>"Redirecting\u{2026}"</p> }.into_any()
                    }
                    Ok(MutationOutcome::CommitIndeterminate(_)) => {
                        view! {
                            <p class="error">
                                "The post may have been saved, but its status could not be confirmed. Refresh to check."
                            </p>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
    }
}
