//! Host-compiled, host-tested decision logic for the posts vertical's listing pages
//! (#306, ADR-0083).
//!
//! The tag and user-timeline pages each carried the same shaped setup in their
//! wasm-only component bodies: decide whether a projector seed applies to the route
//! this render is for, and decide whether a route parameter is usable before
//! fetching. Neither is browser wiring — both are folds over already-resolved values
//! — so they belong here, where `nextest` can assert them and the coverage gate can
//! see them, leaving the component with the `Effect`/`Resource` wiring that genuinely
//! cannot run on the host.
//!
//! The post editor's two folds live here for the same reason: [`publish_redirect`]
//! (did this update publish, and where does the browser go?) and [`with_post_id`]
//! (short-circuit an absent/unparseable `post_id` to a client-side not-found instead
//! of paying a round-trip, #487).
//!
//! `PostCard`'s parent-callback plumbing ([`notify`], [`notify_with_fallback`]) is
//! here too. Firing an `Option<Callback>` is not browser wiring — ADR-0083 §1 grants
//! only `Effect::new` and `spawn_local` permanent wasm-only status — so it is
//! exercised under a reactive `Owner` exactly as [`crate::media::UploadCallbacks`]'s
//! twin is.

use std::future::Future;

use leptos::prelude::*;

use common::ids::PostId;
use common::root_relative_url::RootRelativeUrl;
use common::seed::{PageSeed, TimelinePage};
use common::tag::Tag;
use common::username::Username;

use crate::error::{WebError, WebResult};
use crate::posts::SavedPost;

/// Which listing page is rendering, carrying the route segments it has **already**
/// parsed (`None` = the segment was absent or would not parse).
///
/// A data enum rather than one seed-matching fn per page (ADR-0083 §3): the three
/// pages differ only in which parts of the URL identify them, so the difference
/// travels as a value and [`seeded_page`] stays one host-tested fold.
pub enum ListingRoute {
    /// `/~:username` — the user timeline.
    Profile(Option<Username>),
    /// `/tags/:tag` — the site-wide tag listing.
    SiteTag(Option<Tag>),
    /// `/~:username/tags/:tag` — the per-user tag listing.
    UserTag(Option<Username>, Option<Tag>),
}

/// The projector seed this render may adopt (#178/#179): `Some(page)` only when the
/// seed the server left in context is the seed for **this** route.
///
/// The guard is what makes a client-side navigation safe. The seed is whatever the
/// initial URL was painted with and it never changes, so a nav from `/~alice` to
/// `/~bob` would otherwise adopt alice's posts under bob's heading. A non-matching
/// seed, a seed for a different page kind, and no seed at all all mean the same
/// thing — nothing to adopt — and the reactive fetch fills the page in.
#[must_use]
pub fn seeded_page(seed: Option<PageSeed>, route: &ListingRoute) -> Option<TimelinePage> {
    match (seed?, route) {
        (PageSeed::Profile { username, page }, ListingRoute::Profile(wanted))
            if wanted.as_ref() == Some(&username) =>
        {
            Some(page)
        }
        (PageSeed::SiteTag { tag, page }, ListingRoute::SiteTag(wanted))
            if wanted.as_ref() == Some(&tag) =>
        {
            Some(page)
        }
        (
            PageSeed::UserTag {
                username,
                tag,
                page,
            },
            ListingRoute::UserTag(wanted_username, wanted_tag),
        ) if wanted_username.as_ref() == Some(&username) && wanted_tag.as_ref() == Some(&tag) => {
            Some(page)
        }
        _ => None,
    }
}

/// The username a user-scoped listing fetch needs.
///
/// # Errors
///
/// [`WebError::validation`] when the `~username` segment did not parse — the page
/// paints that error instead of fetching, since no such user can exist.
pub fn user_query(username: Option<Username>) -> WebResult<Username> {
    username.ok_or_else(|| WebError::validation("Invalid username"))
}

/// The tag a tag listing fetch needs.
///
/// # Errors
///
/// [`WebError::validation`] when the `:tag` segment did not parse.
pub fn tag_query(tag: Option<Tag>) -> WebResult<Tag> {
    tag.ok_or_else(|| WebError::validation("Invalid tag"))
}

/// Both route values the per-user tag listing needs, at once.
///
/// # Errors
///
/// The username's error outranks the tag's, so a page whose *both* segments are
/// broken names the first one — the order the two separate guards had.
pub fn user_tag_query(username: Option<Username>, tag: Option<Tag>) -> WebResult<(Username, Tag)> {
    Ok((user_query(username)?, tag_query(tag)?))
}

/// Where an update settles the browser, shaped for `on_settled_ok`'s read closure:
/// `Some(Ok(permalink))` only when the update **published** the post.
///
/// Editor → permalink is always a route change, so a fresh `PostPage` mount refetches
/// — no explicit invalidation needed (#592). A settled-but-still-draft update, a
/// failed one, and "not settled yet" all mean *nothing to navigate to*, and
/// `on_settled_ok` skips all three identically, so collapsing them here leaves the
/// component with no branch at all.
///
/// The permalink stays a [`RootRelativeUrl`] all the way to `use_navigate`, which takes
/// `&str` by deref — unwrapping it here would trade the type for an allocation.
#[must_use]
pub fn publish_redirect<E>(
    settled: Option<Result<SavedPost, E>>,
) -> Option<Result<RootRelativeUrl, E>> {
    let updated = settled?.ok()?;
    let published = updated.published_at.is_some();
    published.then_some(updated.permalink).map(Ok)
}

/// Fire an optional parent callback, when the caller supplied one.
///
/// Every lifecycle hook in the posts vertical spelled out the same `if let Some(cb)`
/// — caller plumbing, not component logic (#306), and nothing about it is
/// browser-bound, so it lives in this host-compiled module rather than in the
/// wasm-only `component.rs` where no test could reach it.
pub fn notify(callback: Option<Callback<()>>) {
    if let Some(callback) = callback {
        callback.run(());
    }
}

/// Fire `preferred`, falling back to `shared` when the caller supplied only the
/// shared one.
///
/// `PostCard`'s unpublish policy: a caller that wants to tell unpublish apart from the
/// other mutations passes `on_unpublish`; one that treats them alike passes only
/// `on_mutate` and still gets told. That is a real per-caller rule — which of two
/// callbacks wins — so it is asserted here rather than left as an `.or()` inside the
/// component.
pub fn notify_with_fallback(preferred: Option<Callback<()>>, shared: Option<Callback<()>>) {
    notify(preferred.or(shared));
}

/// Await `fetch` with the route's post id, or short-circuit to a client-side
/// not-found.
///
/// A missing or unparseable `post_id` is honest absence, not a real id: answering it
/// here rather than minting a sentinel id avoids a round-trip that could only ever
/// return not-found (#487). Both of the editor's resources fetch through this, so the
/// short-circuit is written — and asserted — once.
///
/// # Errors
///
/// [`WebError::not_found`] for an absent id; otherwise whatever `fetch` returns.
pub async fn with_post_id<T, F, Fut>(post_id: Option<PostId>, fetch: F) -> WebResult<T>
where
    F: FnOnce(PostId) -> Fut,
    Fut: Future<Output = WebResult<T>>,
{
    match post_id {
        Some(post_id) => fetch(post_id).await,
        None => Err(WebError::not_found("Post")),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::future::{ready, Ready};

    use super::*;
    use common::test_support::{parse_root_relative_url, parse_slug, parse_tag, parse_username};
    use common::time::UtcInstant;

    fn page(has_more: bool) -> TimelinePage {
        TimelinePage {
            posts: Vec::new(),
            next_cursor_created_at: None,
            next_cursor_post_id: None,
            has_more,
        }
    }

    fn alice() -> Username {
        parse_username("alice")
    }

    fn rust() -> Tag {
        parse_tag("rust")
    }

    // --- seed adoption ---

    #[test]
    fn a_matching_profile_seed_is_adopted() {
        let seed = PageSeed::Profile {
            username: alice(),
            page: page(true),
        };
        let adopted = seeded_page(Some(seed), &ListingRoute::Profile(Some(alice())))
            .expect("the seed names this route");
        // Assert the seeded PAGE came through, not merely that something did: a fold
        // that returned a default page would satisfy `is_some()`.
        assert!(adopted.has_more);
    }

    #[test]
    fn a_profile_seed_for_a_different_user_is_ignored() {
        // The #178/#179 client-nav hazard: the seed is the INITIAL URL's, so without
        // the guard `/~alice` → `/~bob` would paint alice's posts under bob's heading.
        let seed = PageSeed::Profile {
            username: alice(),
            page: page(true),
        };
        assert!(seeded_page(
            Some(seed),
            &ListingRoute::Profile(Some(parse_username("bob")))
        )
        .is_none());
    }

    #[test]
    fn a_profile_seed_is_ignored_when_the_route_segment_did_not_parse() {
        let seed = PageSeed::Profile {
            username: alice(),
            page: page(true),
        };
        assert!(seeded_page(Some(seed), &ListingRoute::Profile(None)).is_none());
    }

    #[test]
    fn a_matching_site_tag_seed_is_adopted() {
        let seed = PageSeed::SiteTag {
            tag: rust(),
            page: page(true),
        };
        let adopted =
            seeded_page(Some(seed), &ListingRoute::SiteTag(Some(rust()))).expect("tag matches");
        assert!(adopted.has_more);
    }

    #[test]
    fn a_site_tag_seed_for_a_different_tag_is_ignored() {
        let seed = PageSeed::SiteTag {
            tag: rust(),
            page: page(true),
        };
        assert!(seeded_page(
            Some(seed),
            &ListingRoute::SiteTag(Some(parse_tag("leptos")))
        )
        .is_none());
        let seed = PageSeed::SiteTag {
            tag: rust(),
            page: page(true),
        };
        assert!(seeded_page(Some(seed), &ListingRoute::SiteTag(None)).is_none());
    }

    #[test]
    fn a_user_tag_seed_needs_both_halves_to_match() {
        let matching = seeded_page(
            Some(PageSeed::UserTag {
                username: alice(),
                tag: rust(),
                page: page(true),
            }),
            &ListingRoute::UserTag(Some(alice()), Some(rust())),
        )
        .expect("both halves match");
        assert!(matching.has_more);

        // Half a match is no match — one `&&`, both directions asserted, so dropping
        // either conjunct fails.
        assert!(seeded_page(
            Some(PageSeed::UserTag {
                username: alice(),
                tag: rust(),
                page: page(true),
            }),
            &ListingRoute::UserTag(Some(parse_username("bob")), Some(rust())),
        )
        .is_none());
        assert!(seeded_page(
            Some(PageSeed::UserTag {
                username: alice(),
                tag: rust(),
                page: page(true),
            }),
            &ListingRoute::UserTag(Some(alice()), Some(parse_tag("leptos"))),
        )
        .is_none());
    }

    #[test]
    fn a_seed_of_the_wrong_variant_is_ignored() {
        // The projector painted a *different kind* of page — e.g. the site timeline or
        // a permalink — so this route has nothing to adopt even though a seed exists.
        assert!(seeded_page(
            Some(PageSeed::SiteTimeline(page(true))),
            &ListingRoute::Profile(Some(alice())),
        )
        .is_none());
        assert!(seeded_page(
            Some(PageSeed::SiteTag {
                tag: rust(),
                page: page(true),
            }),
            &ListingRoute::UserTag(Some(alice()), Some(rust())),
        )
        .is_none());
        assert!(seeded_page(
            Some(PageSeed::UserTag {
                username: alice(),
                tag: rust(),
                page: page(true),
            }),
            &ListingRoute::SiteTag(Some(rust())),
        )
        .is_none());
    }

    #[test]
    fn no_seed_at_all_adopts_nothing() {
        assert!(seeded_page(None, &ListingRoute::Profile(Some(alice()))).is_none());
        assert!(seeded_page(None, &ListingRoute::SiteTag(Some(rust()))).is_none());
        assert!(seeded_page(None, &ListingRoute::UserTag(Some(alice()), Some(rust()))).is_none());
    }

    // --- route-param guards ---

    #[test]
    fn user_query_passes_a_parsed_username_and_names_the_bad_segment() {
        assert_eq!(user_query(Some(alice())), Ok(alice()));
        assert_eq!(
            user_query(None),
            Err(WebError::validation("Invalid username"))
        );
    }

    #[test]
    fn tag_query_passes_a_parsed_tag_and_names_the_bad_segment() {
        assert_eq!(tag_query(Some(rust())), Ok(rust()));
        assert_eq!(tag_query(None), Err(WebError::validation("Invalid tag")));
    }

    #[test]
    fn user_tag_query_reports_the_username_first() {
        assert_eq!(
            user_tag_query(Some(alice()), Some(rust())),
            Ok((alice(), rust()))
        );
        assert_eq!(
            user_tag_query(None, Some(rust())),
            Err(WebError::validation("Invalid username"))
        );
        assert_eq!(
            user_tag_query(Some(alice()), None),
            Err(WebError::validation("Invalid tag"))
        );
        // Both broken: the username's error wins, which is the order the two separate
        // guards had.
        assert_eq!(
            user_tag_query(None, None),
            Err(WebError::validation("Invalid username"))
        );
    }

    // --- the editor's folds ---

    fn saved_post(published_at: Option<UtcInstant>) -> SavedPost {
        SavedPost {
            post_id: PostId::from(7),
            slug: parse_slug("hello"),
            published_at,
            permalink: parse_root_relative_url("/~alice/2026/01/02/hello"),
        }
    }

    #[test]
    fn a_published_update_redirects_to_its_typed_permalink() {
        assert_eq!(
            publish_redirect::<WebError>(Some(Ok(saved_post(Some(
                "2026-01-02T00:00:00Z".parse().expect("a real instant")
            )))))
            .expect("a published update navigates"),
            Ok(parse_root_relative_url("/~alice/2026/01/02/hello")),
        );
    }

    #[test]
    fn a_still_unpublished_update_stays_put() {
        // The editor must not navigate away when the author saved a draft — the whole
        // point of the inner `published_at.is_some()` branch this replaced.
        assert_eq!(
            publish_redirect::<WebError>(Some(Ok(saved_post(None)))),
            None
        );
    }

    #[test]
    fn an_unsettled_or_failed_update_navigates_nowhere() {
        assert_eq!(publish_redirect::<WebError>(None), None);
        assert_eq!(
            publish_redirect(Some(Err(WebError::validation("boom")))),
            None
        );
    }

    /// A fetch that records the id it was called with and hands it straight back.
    ///
    /// **One helper, not a stub inline per test.** The "must not fetch" case asserts
    /// precisely that this body never executes, so an inline stub there would be an
    /// uncovered region by construction — the coverage gate reports that, correctly,
    /// and a `cov:ignore` would be papering over it. Sharing the closure with the case
    /// where the fetch *does* run covers the body once, and strengthens the negative
    /// assertion into a real observation: the very same instrumented closure is
    /// demonstrably capable of recording, so an empty `seen` means it was not called
    /// rather than that it could not have been.
    fn recording_fetch(
        seen: &Cell<Option<PostId>>,
    ) -> impl FnOnce(PostId) -> Ready<WebResult<PostId>> + '_ {
        move |post_id| {
            seen.set(Some(post_id));
            ready(Ok(post_id))
        }
    }

    #[tokio::test]
    async fn with_post_id_fetches_when_the_route_named_one() {
        // The id reaches the fetch unchanged — a fold that dropped it would still
        // return `Ok` from a fetch that ignored its argument.
        let seen = Cell::new(None);
        let fetched = with_post_id(Some(PostId::from(7)), recording_fetch(&seen)).await;
        assert_eq!(
            seen.get(),
            Some(PostId::from(7)),
            "the fetch ran, with the route's id"
        );
        assert_eq!(fetched, Ok(PostId::from(7)));
    }

    #[tokio::test]
    async fn with_post_id_short_circuits_an_absent_id_without_fetching() {
        // Asserting the fetch never RAN is the point (#487): a version that called it
        // with a sentinel id would still return an error and pass a message-only check.
        let seen = Cell::new(None);
        let fetched = with_post_id(None, recording_fetch(&seen)).await;
        assert_eq!(fetched, Err(WebError::not_found("Post")));
        assert_eq!(seen.get(), None, "an absent id costs no round-trip");
    }

    #[tokio::test]
    async fn with_post_id_propagates_the_fetch_error() {
        let fetched = with_post_id(Some(PostId::from(7)), |_| async {
            Err::<String, _>(WebError::validation("boom"))
        })
        .await;
        assert_eq!(fetched, Err(WebError::validation("boom")));
    }

    // --- parent-callback plumbing ---

    /// Run `body` under a fresh reactive `Owner` (the `media::upload_state` /
    /// `forms::Field` convention), so `RwSignal`s and `Callback`s work host-side
    /// without a browser.
    fn with_owner(body: impl FnOnce()) {
        let owner = Owner::new();
        owner.set();
        body();
        drop(owner);
    }

    /// A real `Callback` that records having run into `fired`.
    ///
    /// **One helper, not a closure per test.** The cases that assert a callback did
    /// *not* fire build their sink through this same constructor, so "the signal is
    /// still false" means "this very callback — demonstrably capable of writing it,
    /// two tests up — was never run", not "nothing here could ever have written".
    fn recorder(fired: RwSignal<bool>) -> Callback<()> {
        Callback::new(move |()| fired.set(true))
    }

    #[test]
    fn notify_runs_a_supplied_callback() {
        with_owner(|| {
            let fired = RwSignal::new(false);
            notify(Some(recorder(fired)));
            assert!(fired.get(), "the callback must actually run");
        });
    }

    #[test]
    fn notify_without_a_callback_is_a_no_op() {
        with_owner(|| {
            let fired = RwSignal::new(false);
            // Build the callback a caller WOULD have passed, then pass none: the sink
            // is writable, so an unwritten sink is an observation, not a vacuum.
            let unsupplied = Some(recorder(fired));
            notify(None);
            assert!(!fired.get(), "no callback, nothing fired");
            notify(unsupplied);
            assert!(fired.get(), "and the sink was writable all along");
        });
    }

    #[test]
    fn the_preferred_callback_wins_when_both_are_supplied() {
        // `PostCard`'s unpublish arm: a caller that supplied `on_unpublish` must not
        // also get `on_mutate` — the two are distinct notifications (#592).
        with_owner(|| {
            let preferred = RwSignal::new(false);
            let shared = RwSignal::new(false);
            notify_with_fallback(Some(recorder(preferred)), Some(recorder(shared)));
            assert!(preferred.get(), "the preferred callback runs");
            assert!(!shared.get(), "and the fallback must not also run");
        });
    }

    #[test]
    fn the_shared_callback_runs_when_the_preferred_one_is_absent() {
        with_owner(|| {
            let shared = RwSignal::new(false);
            notify_with_fallback(None, Some(recorder(shared)));
            assert!(
                shared.get(),
                "a caller that supplied only the shared one is told"
            );
        });
    }

    #[test]
    fn a_caller_that_supplied_neither_callback_is_fine() {
        with_owner(|| {
            let never = RwSignal::new(false);
            let unsupplied = Some(recorder(never));
            notify_with_fallback(None, None);
            assert!(!never.get());
            notify_with_fallback(unsupplied, None);
            assert!(never.get(), "the sink was writable all along");
        });
    }
}
