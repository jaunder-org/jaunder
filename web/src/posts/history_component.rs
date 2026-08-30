//! Owner-only Post Revision history pages.
//!
//! This sibling keeps the history route surface out of the already-large post
//! editor component. It is wasm-only by its declaration in `mod.rs`.

use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_params_map;

use common::ids::{PostId, RevisionId};
use common::pagination::PageSize;
use common::revision_history::RevisionHistoryDetail;

use super::{
    AuthenticatedHistoryState, CurrentPostHistory, HistoryCollectionDisplay, HistoryDisplayRow,
    HistoryListState, PostRevisionHistory, RevisionHistoryCursor, RevisionHistoryMetadata,
    RevisionHistoryPage, RevisionLifecycle, authenticated_history_state, current_history_rows,
    get_post_history, get_revision_history_detail, list_history, load_authenticated_history,
    revision_collection_displays, revision_history_rows,
};
use crate::auth;
use crate::topbar::Topbar;

#[derive(Clone, Copy)]
enum HistoryScope {
    Global,
    Post(PostId),
}

impl HistoryScope {
    async fn next_page(
        self,
        cursor: RevisionHistoryCursor,
    ) -> crate::error::WebResult<RevisionHistoryPage> {
        match self {
            Self::Global => super::list_history(Some(cursor), Some(PageSize::default())).await,
            Self::Post(post_id) => {
                super::get_post_history(post_id, Some(cursor), Some(PageSize::default()))
                    .await
                    .map(|history| history.revisions)
            }
        }
    }
}

fn lifecycle_label(lifecycle: &RevisionLifecycle) -> &'static str {
    match lifecycle {
        RevisionLifecycle::Draft => "Draft",
        RevisionLifecycle::Scheduled => "Scheduled",
        RevisionLifecycle::Published => "Published",
        RevisionLifecycle::Deleted => "Deleted",
    }
}

fn optional_title<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(|| "No title".to_owned(), |value| value.to_string())
}

fn history_auth_required() -> impl IntoView {
    view! {
        <div data-test="history-auth-required">
            <p>"You must be logged in to inspect Post Revision history."</p>
            <p>
                <a href="/login" class="j-btn is-primary">
                    "Sign in"
                </a>
            </p>
        </div>
    }
}

fn history_state_view(state: AuthenticatedHistoryState<AnyView>) -> AnyView {
    match state {
        AuthenticatedHistoryState::Ready(view) => view,
        AuthenticatedHistoryState::NotFound => "Page not found.".into_any(),
        AuthenticatedHistoryState::AuthRequired => history_auth_required().into_any(),
        AuthenticatedHistoryState::Failed(error) => {
            view! { <p class="error">{error.to_string()}</p> }.into_any()
        }
    }
}

/// Newest-first history across every Post owned by the authenticated user.
#[component]
pub fn HistoryPage() -> impl IntoView {
    let session = auth::use_session();
    let initial = Resource::new(
        || (),
        move |()| async move {
            let Some(_) = session.reconcile.await? else {
                return Ok(None);
            };
            super::list_history(None, Some(PageSize::default()))
                .await
                .map(Some)
        },
    );

    view! {
        <Topbar title="History" sub="Immutable Post Revisions" />
        <div class="j-scroll">
            <div class="j-page" data-test="history-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        let state = match initial.await {
                            Ok(Some(page)) => {
                                AuthenticatedHistoryState::Ready(
                                    history_list(page, HistoryScope::Global).into_any(),
                                )
                            }
                            Ok(None) => AuthenticatedHistoryState::AuthRequired,
                            Err(error) => AuthenticatedHistoryState::Failed(error),
                        };
                        history_state_view(state)
                    })}
                </Suspense>
            </div>
        </div>
    }
}

/// Current server-derived state followed by one Post's immutable revisions.
#[component]
pub fn PostHistoryPage() -> impl IntoView {
    let params = use_params_map();
    let session = auth::use_session();
    let post_id = move || {
        params
            .get()
            .get("post_id")
            .and_then(|value| value.parse::<PostId>().ok())
    };
    let initial = Resource::new(post_id, move |post_id| {
        let reconcile = session.reconcile;
        super::load_authenticated_history(
            post_id,
            move || async move { reconcile.await },
            |post_id| super::get_post_history(post_id, None, Some(PageSize::default())),
        )
    });

    view! {
        <Topbar title="Post History" sub="Current state and immutable revisions" />
        <div class="j-scroll">
            <div class="j-page" data-test="post-history-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        let state = super::authenticated_history_state(
                                post_id().is_some(),
                                initial.await,
                            )
                            .map_ready(|history| post_history_view(history).into_any());
                        history_state_view(state)
                    })}
                </Suspense>
            </div>
        </div>
    }
}

fn post_history_view(history: PostRevisionHistory) -> impl IntoView {
    let post_id = history.current.post_id;
    view! {
        {current_post_summary(history.current)}
        <section aria-labelledby="post-revisions-heading">
            <h2 id="post-revisions-heading">"Post Revisions"</h2>
            {history_list(history.revisions, HistoryScope::Post(post_id))}
        </section>
    }
}

fn current_post_summary(current: CurrentPostHistory) -> impl IntoView {
    view! {
        <section
            class="j-card j-history-summary"
            data-test="history-current"
            aria-labelledby="current-state-heading"
        >
            <div class="j-card-head">
                <div>
                    <h2 id="current-state-heading">"Current state"</h2>
                    <div class="j-sub">"Live server state, not a revision snapshot."</div>
                </div>
            </div>
            {history_fields(super::current_history_rows(current))}
        </section>
    }
}

fn history_fields(rows: Vec<HistoryDisplayRow>) -> impl IntoView {
    rows.into_iter()
        .map(|row| {
            view! {
                <div class="j-field-row">
                    <div class="j-field-label">{row.label}</div>
                    <div data-test=row.data_test>{row.value}</div>
                </div>
            }
        })
        .collect::<Vec<_>>()
}

fn history_list(initial: RevisionHistoryPage, scope: HistoryScope) -> impl IntoView {
    let state = HistoryListState::new(initial);

    let load_more = move |_| {
        let Some(next_cursor) = state.begin_load_more() else {
            return;
        };
        spawn_local(async move {
            state.finish_load_more(scope.next_page(next_cursor).await);
        });
    };

    view! {
        <div data-test="history-list">
            {move || {
                let revisions = state.rows.get();
                if revisions.is_empty() {
                    view! {
                        <div data-test="history-empty">
                            <p>"No Post Revisions yet."</p>
                            <p>
                                "Meaningful Post changes will appear here; unchanged saves do not create revisions."
                            </p>
                        </div>
                    }
                        .into_any()
                } else {
                    view! {
                        <div class="j-history-table-wrap">
                            <table class="j-table">
                                <thead>
                                    <tr>
                                        <th>"Snapshot"</th>
                                        <th>"Post"</th>
                                        <th>"Captured"</th>
                                        <th>"Snapshot state"</th>
                                        <th>"Post now"</th>
                                        <th>"Action"</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {revisions.into_iter().map(history_row).collect::<Vec<_>>()}
                                </tbody>
                            </table>
                        </div>
                    }
                        .into_any()
                }
            }} {move || state.load_error.get().map(|error| view! { <p class="error">{error}</p> })}
            {move || {
                state
                    .has_more
                    .get()
                    .then(|| {
                        view! {
                            <p class="j-history-load-more">
                                <button
                                    type="button"
                                    class="j-btn"
                                    data-test="history-load-more"
                                    disabled=move || state.loading_more.get()
                                    on:click=load_more
                                >
                                    {move || {
                                        if state.loading_more.get() {
                                            "Loading\u{2026}"
                                        } else {
                                            "Load more"
                                        }
                                    }}
                                </button>
                            </p>
                        }
                    })
            }}
        </div>
    }
}

fn history_row(revision: RevisionHistoryMetadata) -> impl IntoView {
    let revision_id = i64::from(revision.revision_id);
    let post_id = i64::from(revision.post_id);
    let title = optional_title(revision.title);
    let slug = revision.slug.to_string();
    let captured_at = revision.captured_at.to_string();
    let snapshot_lifecycle = lifecycle_label(&revision.snapshot_lifecycle);
    let current_state = if revision.current_deleted {
        "Deleted"
    } else {
        "Active"
    };
    let detail_href = format!("/posts/{post_id}/history/{revision_id}");
    let post_href = format!("/posts/{post_id}/history");

    view! {
        <tr data-test="history-row" data-current-deleted=revision.current_deleted.to_string()>
            <td>
                <strong>{title}</strong>
                <br />
                <span class="j-count">{slug}</span>
            </td>
            <td>
                <a href=post_href>{format!("Post {post_id}")}</a>
            </td>
            <td>{captured_at}</td>
            <td>{snapshot_lifecycle}</td>
            <td>{current_state}</td>
            <td>
                <a class="j-btn" data-test="history-detail-link" href=detail_href>
                    "Inspect"
                </a>
            </td>
        </tr>
    }
}

/// Complete immutable prior-state snapshot for one owned Post Revision.
#[component]
pub fn RevisionHistoryDetailPage() -> impl IntoView {
    let params = use_params_map();
    let session = auth::use_session();
    let route_ids = move || {
        let values = params.get();
        let post_id = values
            .get("post_id")
            .and_then(|value| value.parse::<PostId>().ok());
        let revision_id = values
            .get("revision_id")
            .and_then(|value| value.parse::<RevisionId>().ok());
        post_id.zip(revision_id)
    };
    let detail = Resource::new(route_ids, move |ids| {
        let reconcile = session.reconcile;
        super::load_authenticated_history(
            ids,
            move || async move { reconcile.await },
            |(post_id, revision_id)| super::get_revision_history_detail(post_id, revision_id),
        )
    });

    view! {
        <Topbar title="Post Revision" sub="Immutable prior-state snapshot" />
        <div class="j-scroll">
            <div class="j-page" data-test="history-detail-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        let state = super::authenticated_history_state(
                                route_ids().is_some(),
                                detail.await,
                            )
                            .map_ready(|detail| revision_detail_view(detail).into_any());
                        history_state_view(state)
                    })}
                </Suspense>
            </div>
        </div>
    }
}

fn revision_detail_view(detail: RevisionHistoryDetail) -> impl IntoView {
    let post_id_number = i64::from(detail.post_id);
    let post_history_href = format!("/posts/{post_id_number}/history");
    let metadata = super::revision_history_rows(&detail);
    let body = detail.body.to_string();
    let rendered_html = detail.rendered_html.to_string();
    let collections =
        super::revision_collection_displays(detail.tags, detail.audiences, &detail.media);

    view! {
        <p>
            <a href=post_history_href>"Back to Post History"</a>
        </p>
        <section class="j-card j-history-summary" aria-labelledby="revision-snapshot-heading">
            <div class="j-card-head">
                <h2 id="revision-snapshot-heading">"Snapshot metadata"</h2>
            </div>
            {history_fields(metadata)}
        </section>
        <section class="j-card" aria-labelledby="revision-source-heading">
            <div class="j-card-head">
                <h2 id="revision-source-heading">"Authored source"</h2>
            </div>
            <pre class="j-code" data-test="history-source">
                {body}
            </pre>
        </section>
        <section class="j-card" aria-labelledby="revision-rendered-heading">
            <div class="j-card-head">
                <h2 id="revision-rendered-heading">"Rendered representation"</h2>
            </div>
            <pre class="j-code" data-test="history-rendered">
                {rendered_html}
            </pre>
        </section>
        {revision_collections(collections)}
    }
}

fn revision_collections(collections: Vec<HistoryCollectionDisplay>) -> impl IntoView {
    collections
        .into_iter()
        .map(|collection| {
            view! {
                <section class="j-card" aria-labelledby=collection.heading_id>
                    <div class="j-card-head">
                        <h2 id=collection.heading_id>{collection.heading}</h2>
                    </div>
                    <pre class="j-code" data-test=collection.data_test>
                        {collection.value}
                    </pre>
                </section>
            }
        })
        .collect::<Vec<_>>()
}
