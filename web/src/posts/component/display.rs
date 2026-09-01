use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::avatar::Avatar;
use crate::error::WebError;
use crate::posts;
use crate::posts::{Delete, Publish, SavedPost, Unpublish};
use crate::taglist::TagCtx;
use client::telemetry;
use common::MutationOutcome;
use common::root_relative_url::RootRelativeUrl;
use common::seed::RenderedPost;
use common::username::Username;
use common::{client_telemetry::ClientErrorContext, ids::PostId};

use super::support;

#[component]
pub fn PostDisplay<'a>(
    post: &'a RenderedPost,
    banner: Option<&'a str>,
    /// Linking context for the tag chips in the footer; defaults to
    /// site-wide.
    #[prop(default = &TagCtx::SiteWide)]
    tag_context: &'a TagCtx,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView + use<> {
    let time_label = posts::render::format_post_time(post.display_time());
    // Built once and shared by both arms so the authored content column is the SAME
    // pure, viewer-independent render the projector paints (#181, ADR-0044 D4) — no
    // hand-rebuilt markup and no is_author-driven content change that could diverge
    // and reintroduce a flash. The action column is layered on additively.
    let view = posts::render::PostView {
        username: &post.username,
        title: post.title.as_ref(),
        banner,
        summary: post.summary.as_ref(),
        rendered_html: &post.rendered_html,
        time: &time_label,
        permalink: post.permalink.as_ref(),
        tags: &post.tags,
        tag_ctx: tag_context,
    };
    match children {
        // Anonymous / no-action layout: the WHOLE article inner is produced by the
        // pure `render` layer — the SAME code the public projector server-renders
        // (#179) — and injected via `inner_html`, so a seeded first paint and this
        // reactive re-render are byte-identical (flash-free). "Share the pure fn,
        // not the component" (ADR-0041 §4). The projector only ever renders this
        // anonymous view, so this is the only path that must coincide.
        None => {
            let inner = posts::render::render_post_inner(&view);
            inner
                .inject_into(leptos::html::article().class("j-post"))
                .into_any()
        }
        // Authored layout (own posts, with the action column). The content column is
        // the SAME `render_post_content` the anonymous arm wraps, injected via
        // `inner_html` so it coincides with the projector's paint (#181); only the
        // reactive action column (`children`, carrying edit/delete handlers that
        // `inner_html` can't) overlays it as a sibling — hand-rebuilt reactive
        // markup here would diverge from the projector and reintroduce the flash.
        Some(children) => {
            let inner_content = posts::render::render_post_content(&view);
            view! {
                <article class="j-post">
                    <Avatar name=&post.username size=38 />
                    <div style="min-width:0;display:flex;gap:8px;align-items:flex-start">
                        {inner_content.inject_into(leptos::html::div().style("flex:1;min-width:0"))}
                        {children()}
                    </div>
                </article>
            }
            .into_any()
        }
    }
}

/// `true` when the shared session's username equals `author` (#181/#591): the
/// client-side signal that the viewer owns this post, so its action column shows
/// even though the anonymous seed data has `is_author = false`. `false` on the host
/// build / outside the provider (no context) — the affordance is wasm-only chrome.
/// Uses `use_context` (not `use_session`) so the host build yields `None`→`false`
/// rather than panicking; reads untracked to match the original non-reactive read.
fn marker_matches(author: &Username) -> bool {
    use_context::<crate::auth::SessionContext>()
        .and_then(|ctx| ctx.current.get_untracked())
        .as_ref()
        .map(|user| &user.username)
        == Some(author)
}
fn dispatch_after_confirm(message: &str, context: ClientErrorContext, dispatch: impl FnOnce()) {
    match client::dialog::confirm(message) {
        Ok(outcome) => {
            if outcome.should_dispatch() {
                dispatch();
            }
        }
        Err(error) => {
            let source_kind = error.source_kind();
            telemetry::report_swallowed(telemetry::error_kind(source_kind), context, source_kind);
        }
    }
}
fn primary_post_action(
    is_draft: bool,
    post_id: PostId,
    publish_action: ServerAction<Publish>,
    unpublish_action: ServerAction<Unpublish>,
) -> AnyView {
    if is_draft {
        view! {
            <button
                type="button"
                class="j-btn"
                on:click=move |_| {
                    dispatch_after_confirm(
                        "Publish this draft?",
                        ClientErrorContext::PublishConfirm,
                        || {
                            publish_action.dispatch(Publish { post_id });
                        },
                    );
                }
            >
                "Publish"
            </button>
        }
        .into_any()
    } else {
        view! {
            <button
                type="button"
                class="j-btn"
                on:click=move |_| {
                    unpublish_action.dispatch(Unpublish { post_id });
                }
            >
                "Unpublish"
            </button>
        }
        .into_any()
    }
}

fn mutation_feedback<T>(
    result: Result<MutationOutcome<T>, WebError>,
    indeterminate_message: &'static str,
) -> Option<AnyView> {
    match crate::mutation_feedback::classify(result, indeterminate_message) {
        crate::mutation_feedback::MutationFeedback::Confirmed(_) => None,
        crate::mutation_feedback::MutationFeedback::Error(message) => {
            Some(view! { <p class="error">{message}</p> }.into_any())
        }
    }
}

fn post_action_column(
    is_author: bool,
    edit_url: RootRelativeUrl,
    history_url: String,
    primary_action: AnyView,
    delete_action: ServerAction<Delete>,
    post_id: PostId,
) -> Option<AnyView> {
    is_author.then(move || {
        view! {
            <div class="j-post-acts">
                <a class="j-btn" href=edit_url.to_string()>
                    "Edit"
                </a>
                <a class="j-btn" data-test="post-history-link" href=history_url>
                    "History"
                </a>
                {primary_action}
                <button
                    type="button"
                    class="j-btn is-danger"
                    on:click=move |_| {
                        dispatch_after_confirm(
                            "Delete this post?",
                            ClientErrorContext::DeleteConfirm,
                            || {
                                delete_action.dispatch(Delete { post_id });
                            },
                        );
                    }
                >
                    "Delete"
                </button>
            </div>
        }
        .into_any()
    })
}

fn notify_unpublish_outcome(
    outcome: &MutationOutcome<SavedPost>,
    on_unpublish: Option<Callback<()>>,
    on_mutate: Option<Callback<()>>,
) {
    match outcome {
        MutationOutcome::Confirmed(_) => {
            posts::notify_with_fallback(on_unpublish, on_mutate);
        }
        MutationOutcome::CommitIndeterminate(_) => posts::notify(on_mutate),
    }
}

#[component]
pub fn PostCard<'a>(
    post: &'a RenderedPost,
    banner: Option<&'a str>,
    /// Linking context for the footer tag chips; defaults to site-wide.
    #[prop(default = &TagCtx::SiteWide)]
    tag_context: &'a TagCtx,
    #[prop(optional)] on_mutate: Option<Callback<()>>,
    #[prop(optional)] on_unpublish: Option<Callback<()>>,
    /// Fired only after a successful *publish* (distinct from `on_mutate`, which delete
    /// and unpublish share). The permalink page uses it to refetch itself in place when a
    /// same-URL publish leaves navigation a no-op, without delete/unpublish also
    /// refetching into a not-found (#592).
    #[prop(optional)]
    on_publish: Option<Callback<()>>,
) -> impl IntoView + use<> {
    // The seed/anonymous data has `is_author = false` (the projector paints
    // anonymous-only), so on the Local timeline the owner's own posts would show no
    // action column. Decide it client-side from the auth marker (#181, ADR-0044 D4)
    // so the affordance appears synchronously at mount. The server still authorizes
    // the actual edit/delete by session — the marker only gates visibility.
    let is_author = post.is_author || marker_matches(&post.username);
    let post_id = post.post_id;
    // A draft rendered at its permalink gets a Publish affordance instead of
    // Unpublish (#23): an Unpublish column would be a no-op on an already-
    // unpublished post.
    let is_draft = post.is_draft();
    let edit_url = posts::edit_post_url(post_id);
    let history_url = format!("/posts/{}/history", i64::from(post_id));
    let delete_action = ServerAction::<Delete>::new();
    let unpublish_action = ServerAction::<Unpublish>::new();
    let publish_action = ServerAction::<Publish>::new();
    let deleted = RwSignal::new(false);

    support::on_settled_ok(
        move || delete_action.value().get(),
        move |outcome| {
            match outcome {
                MutationOutcome::Confirmed(()) => deleted.set(true),
                MutationOutcome::CommitIndeterminate(()) => {}
            }
            posts::notify(on_mutate);
        },
    );
    support::on_settled_ok(
        move || unpublish_action.value().get(),
        move |outcome| notify_unpublish_outcome(&outcome, on_unpublish, on_mutate),
    );
    let navigate = use_navigate();
    support::on_settled_ok(
        move || publish_action.value().get(),
        move |outcome| {
            match outcome {
                MutationOutcome::Confirmed(published) => {
                    navigate(&published.permalink, NavigateOptions::default());
                }
                MutationOutcome::CommitIndeterminate(_) => {}
            }
            posts::notify(on_publish);
        },
    );

    let primary_action = primary_post_action(is_draft, post_id, publish_action, unpublish_action);

    let action_col = post_action_column(
        is_author,
        edit_url,
        history_url,
        primary_action,
        delete_action,
        post_id,
    );

    view! {
        {move || {
            deleted.get().then(|| view! { <p class="success">"Post deleted."</p> }.into_any())
        }}
        {move || {
            delete_action
                .value()
                .get()
                .and_then(|result| {
                    mutation_feedback(
                        result,
                        "The post may have been deleted, but its status could not be confirmed. Refresh to check.",
                    )
                })
        }}
        {move || {
            publish_action
                .value()
                .get()
                .and_then(|result| {
                    mutation_feedback(
                        result,
                        "The post may have been published, but its status could not be confirmed. Refresh to check.",
                    )
                })
        }}
        {move || {
            unpublish_action
                .value()
                .get()
                .and_then(|result| {
                    mutation_feedback(
                        result,
                        "The post may have been unpublished, but its status could not be confirmed. Refresh to check.",
                    )
                })
        }}
        <PostDisplay post=post banner=banner tag_context=tag_context>
            {action_col}
        </PostDisplay>
    }
}
