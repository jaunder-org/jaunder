//! Test-only tooling that reaches jaunder's storage layer from OUTSIDE the
//! server process — a live-server Playwright e2e drives the `test-support`
//! binary over a process boundary to seed fixtures. It is the cross-process
//! sibling of the in-process `storage::test_support` module and is never linked
//! into the `jaunder` production binary (see ADR-0046, `test-support-seed-binary`).
//!
//! The seed core drives `storage::create_rendered_post` directly rather than
//! calling `storage::test_support::seed_posts`: the e2e suite shares one database
//! across all tests, so seeds need per-user-unique, content-shaped slugs/bodies
//! that the module helper's fixed `seed-{i}` / `# Post {i}` scheme cannot give.

use std::sync::Arc;

use common::username::Username;
use storage::{create_rendered_post, AppState, PostFormat};

/// The rendered-body source for seeded post `i` under `prefix`. Its Markdown H1
/// renders the text `"{prefix} {i}"`, which the heavy e2e timeline tests assert
/// on (first/last post title after pagination).
#[must_use]
pub fn seed_body(prefix: &str, i: usize) -> String {
    format!("# {prefix} {i}\n\nBody for {prefix} {i}")
}

/// A slug-valid, per-prefix-unique string for seeded post `i`: `prefix`
/// lowercased with every non-alphanumeric run collapsed to `-`, then the index
/// suffix. Because each heavy test registers a fresh user and the slug
/// uniqueness constraint is per-user, distinct prefixes keep every seed
/// invocation collision-free even against the shared e2e database.
#[must_use]
pub fn seed_slug(prefix: &str, i: usize) -> String {
    let base: String = prefix
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-');
    format!("{base}-{i}")
}

/// Seed `count` posts for `username` through the real `create_rendered_post`
/// path — the same code the server runs, so audience rows, rendered HTML, and
/// both SQL dialects come for free. `published` sets `published_at = now` and a
/// Public audience so the posts surface on the timeline; otherwise they are
/// drafts. Returns the created ids oldest-to-newest.
///
/// Slugs derive from `prefix` + index and the slug-uniqueness constraint is
/// per-user, so callers that share one database (the e2e suite) must pass a
/// distinct `prefix` for each user they seed — re-seeding the same user with the
/// same prefix would collide on the second invocation.
///
/// # Errors
///
/// Returns `Err` if `username` is invalid or unknown, a generated slug fails to
/// parse, or a post fails to persist.
pub async fn seed_posts_for_user(
    state: &Arc<AppState>,
    username: &str,
    count: usize,
    published: bool,
    prefix: &str,
) -> anyhow::Result<Vec<i64>> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let user = state
        .users
        .get_user_by_username(&uname)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {username}"))?;

    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        let published_at = published.then(chrono::Utc::now);
        let slug = seed_slug(prefix, i).parse().map_err(|_| {
            anyhow::anyhow!("generated slug invalid for prefix {prefix:?} index {i}")
        })?;
        let id = create_rendered_post(
            &*state.posts,
            user.user_id,
            None,
            slug,
            seed_body(prefix, i),
            PostFormat::Markdown,
            published_at,
            None,
            vec![common::visibility::AudienceTarget::Public],
        )
        .await
        .map_err(|e| anyhow::anyhow!("seed post {i} failed: {e:?}"))?;
        ids.push(id);
    }
    Ok(ids)
}

#[cfg(test)]
mod content_tests {
    use super::*;

    #[test]
    fn seed_body_renders_prefix_and_index() {
        assert_eq!(
            seed_body("Timeline Post", 50),
            "# Timeline Post 50\n\nBody for Timeline Post 50"
        );
    }

    #[test]
    fn seed_slug_is_slug_safe() {
        assert_eq!(seed_slug("Timeline Post", 0), "timeline-post-0");
        assert_eq!(seed_slug("Home Feed Mine", 12), "home-feed-mine-12");
    }
}

#[cfg(test)]
mod seed_tests {
    //! `SQLite`-only by design: `seed_posts_for_user` has no per-backend
    //! branching — it dispatches through `storage::create_rendered_post`, which
    //! the storage layer implements per backend — so these tests smoke the seed
    //! *logic* on `SQLite` for speed. The tool's dual-backend behaviour is proven
    //! end-to-end by the e2e matrix, which drives `test-support` against both
    //! `SQLite` and `Postgres` ({sqlite,postgres}×{chromium,firefox}).
    use super::*;
    use storage::test_support;

    #[tokio::test]
    async fn seeds_public_published_posts_visible_to_a_non_author() {
        let base = tempfile::TempDir::new().unwrap();
        let (state, _pool) = test_support::test_sqlite_state_with_pool(&base).await;
        let _uid = test_support::seed_user(&state).await; // creates "testuser"

        let ids = seed_posts_for_user(&state, "testuser", 3, true, "Timeline Post")
            .await
            .expect("seed ok");
        assert_eq!(ids.len(), 3);

        // The point of the tool: seeded posts are Public + published, so an
        // Anonymous (non-author) viewer sees all three. A bare `posts` insert
        // with no `post_audiences` row would be private and this would return 0
        // — this asserts the tool seeds a *timeline-visible* post, not just a row.
        let page = state
            .posts
            .list_published_by_user(
                &"testuser".parse().unwrap(),
                None,
                10,
                &common::visibility::ViewerIdentity::Anonymous,
                chrono::Utc::now(),
            )
            .await
            .expect("list ok");
        assert_eq!(page.len(), 3);
    }

    #[tokio::test]
    async fn rejects_a_prefix_that_cannot_form_a_valid_slug() {
        let base = tempfile::TempDir::new().unwrap();
        let (state, _pool) = test_support::test_sqlite_state_with_pool(&base).await;
        let _uid = test_support::seed_user(&state).await; // creates "testuser"

        // A prefix with no alphanumerics collapses to an empty base, so the slug
        // would begin with '-' and fail `Slug` parsing — surfaced as an error
        // (not a panic) before any post is persisted.
        let err = seed_posts_for_user(&state, "testuser", 1, false, "***")
            .await
            .expect_err("invalid generated slug should error");
        assert!(err.to_string().contains("generated slug invalid"));
    }
}
