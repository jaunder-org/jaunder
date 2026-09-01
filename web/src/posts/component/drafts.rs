use leptos::prelude::*;

use crate::auth;
use crate::error::WebError;
use crate::posts;
use crate::posts::{Delete, DraftRowDisplay, Publish, SavedPost, UnpublishedPost};
use crate::topbar::Topbar;
use common::{MutationOutcome, pagination::PageSize, seed::Page};

#[component]
pub fn DraftsPage() -> impl IntoView {
    let publish_action = ServerAction::<Publish>::new();
    let delete_action = ServerAction::<Delete>::new();
    let drafts = Resource::new(
        move || {
            (
                publish_action.version().get(),
                delete_action.version().get(),
            )
        },
        |_| posts::list_drafts(None, Some(PageSize::default())),
    );

    view! {
        <Topbar title="Drafts" sub="Unpublished posts" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        view! {
                            <DraftList
                                drafts=drafts.await
                                publish_action=publish_action
                                delete_action=delete_action
                            />
                        }
                    })}
                </Suspense>
                {move || {
                    publish_action
                        .value()
                        .get()
                        .map(|result: Result<MutationOutcome<SavedPost>, WebError>| match result {
                            Ok(MutationOutcome::Confirmed(published)) => {
                                view! {
                                    <p class="success">
                                        "Post published. "
                                        <a href=published.permalink.to_string()>"View permalink"</a>
                                    </p>
                                }
                                    .into_any()
                            }
                            Ok(MutationOutcome::CommitIndeterminate(_)) => {
                                view! {
                                    <p class="error">
                                        "The post may have been published, but its status could not be confirmed. Refresh to check."
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
                        .map(|result: Result<MutationOutcome<()>, WebError>| match result {
                            Ok(MutationOutcome::Confirmed(())) => {
                                view! { <p class="success">"Draft deleted."</p> }.into_any()
                            }
                            Ok(MutationOutcome::CommitIndeterminate(())) => {
                                view! {
                                    <p class="error">
                                        "The draft may have been deleted, but its status could not be confirmed. Refresh to check."
                                    </p>
                                }
                                    .into_any()
                            }
                            Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                        })
                }}
            </div>
        </div>
    }
}

#[component]
pub fn ScheduledPage() -> impl IntoView {
    // Match the create-post gate: the scheduled listing is an authenticated management
    // surface, so wait for the server-confirmed session before calling the row API.
    let session = auth::use_session();

    view! {
        <Topbar title="Scheduled" sub="Posts queued for publication" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match session.reconcile.await {
                            Ok(Some(_)) => {
                                let scheduled = posts::list_scheduled(
                                        None,
                                        Some(PageSize::default()),
                                    )
                                    .await;
                                view! { <ScheduledList scheduled=scheduled /> }.into_any()
                            }
                            Ok(None) => {
                                view! {
                                    <div data-test="scheduled-auth-required">
                                        <p>"You must be logged in to manage Scheduled Posts."</p>
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
            </div>
        </div>
    }
}

#[component]
fn ScheduledList(scheduled: Result<Page<UnpublishedPost>, WebError>) -> impl IntoView {
    match scheduled {
        Ok(page) => {
            if page.posts.is_empty() {
                view! {
                    <div data-test="scheduled-empty">
                        <p>"You have no Scheduled Posts."</p>
                        <p>
                            <a
                                data-test="scheduled-compose-link"
                                href="/posts/new"
                                class="j-btn is-primary"
                            >
                                "Compose a Post"
                            </a>
                        </p>
                    </div>
                }
                .into_any()
            } else {
                view! {
                    <ul class="j-draft-list" data-test="scheduled-list">
                        {page.posts.into_iter().map(render_scheduled_row).collect::<Vec<_>>()}
                    </ul>
                }
                .into_any()
            }
        }
        Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
    }
}

fn render_scheduled_row(scheduled: UnpublishedPost) -> impl IntoView {
    let DraftRowDisplay { label, .. } = posts::draft_row_display(&scheduled);
    let go_live = scheduled.post.published_at.map_or_else(
        || "Scheduled time unavailable".to_owned(),
        |when| when.to_string(),
    );
    view! {
        <li data-test="scheduled-row">
            <div class="j-draft-row">
                <div class="j-draft-row-content">
                    <strong>
                        <a href=String::from(scheduled.edit_url.clone())>{label}</a>
                    </strong>
                    <span class="j-badge j-badge-scheduled">
                        "Scheduled for " <time data-test="scheduled-go-live">{go_live}</time>
                    </span>
                </div>
                <div class="j-draft-actions">
                    <a
                        class="j-btn"
                        data-test="scheduled-edit-link"
                        href=String::from(scheduled.edit_url)
                    >
                        "Edit"
                    </a>
                </div>
            </div>
        </li>
    }
}

/// The resolved drafts list: the rows, the empty-state line, or the fetch error.
///
/// A **subcomponent** rather than a plain view-returning fn, so the branch on the awaited
/// result stays measured by the thin-component guard instead of hiding in an unmeasured
/// helper (#306). It takes the already-awaited result, so [`DraftsPage`]'s `Suspense` still
/// owns the awaiting.
#[component]
fn DraftList(
    drafts: Result<Page<UnpublishedPost>, WebError>,
    publish_action: ServerAction<Publish>,
    delete_action: ServerAction<Delete>,
) -> impl IntoView {
    match drafts {
        Ok(page) => {
            if page.posts.is_empty() {
                view! { <p>"You have no drafts."</p> }.into_any()
            } else {
                view! {
                    <ul class="j-draft-list">
                        {page
                            .posts
                            .into_iter()
                            .map(|draft| render_draft_row(draft, publish_action, delete_action))
                            .collect::<Vec<_>>()}
                    </ul>
                }
                .into_any()
            }
        }
        Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
    }
}

fn render_draft_row(
    draft: UnpublishedPost,
    publish_action: ServerAction<Publish>,
    delete_action: ServerAction<Delete>,
) -> impl IntoView {
    let post_id = i64::from(draft.post.post_id);
    // Pure title + scheduled-badge-text computation (host-tested in `super::parse`);
    // only the `view!` markup stays here.
    let DraftRowDisplay {
        label,
        scheduled_badge,
    } = posts::draft_row_display(&draft);
    let scheduled_badge = scheduled_badge.map(|text| {
        view! { <span class="j-badge j-badge-scheduled">{text}</span> }
    });
    view! {
        <li>
            <div class="j-draft-row">
                <div class="j-draft-row-content">
                    <strong>{label}</strong>
                    " ("
                    {draft.post.slug.to_string()}
                    ") "
                    {scheduled_badge}
                    " "
                    <a href=String::from(draft.post.permalink)>"Permalink"</a>
                </div>
                <div class="j-draft-actions">
                    <a class="j-btn" href=String::from(draft.edit_url)>
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
