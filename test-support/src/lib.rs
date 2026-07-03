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
use storage::{seed_rendered_post, AppState};

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
        let slug = seed_slug(prefix, i).parse().map_err(|_| {
            anyhow::anyhow!("generated slug invalid for prefix {prefix:?} index {i}")
        })?;
        let id = seed_rendered_post(
            &*state.posts,
            user.user_id,
            slug,
            seed_body(prefix, i),
            published,
        )
        .await
        .map_err(|e| anyhow::anyhow!("seed post {i} failed: {e:?}"))?;
        ids.push(id);
    }
    Ok(ids)
}

/// Create a fixture user through the real `UserStorage::create_user` path — the
/// same call `jaunder user-create` makes (`server::commands::cmd_user_create`),
/// minus that command's `CliBypass` registration metric: this is out-of-process
/// test seeding and must not emit observability noise the e2e suite might assert
/// on. Assumes a freshly-initialised DB (no upsert). Returns the new user id.
///
/// # Errors
///
/// Returns `Err` if the username or password is invalid, or the user cannot be
/// created (e.g. a duplicate username).
pub async fn create_user(
    state: &Arc<AppState>,
    username: &str,
    password: &str,
    display_name: Option<&str>,
    operator: bool,
) -> anyhow::Result<i64> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let pw = password
        .parse::<common::password::Password>()
        .map_err(|e| anyhow::anyhow!("invalid password: {e}"))?;
    let id = state
        .users
        .create_user(&uname, &pw, display_name, operator)
        .await?;
    Ok(id)
}

/// Set a `site_config` key through `SiteConfigStorage::set` (an upsert) —
/// replaces the raw-SQL `site_config` INSERTs the e2e sites use for
/// `site.registration_policy` and `feeds.websub_hub_url`. Generic key/value, so
/// one subcommand covers both and any future e2e site-config.
///
/// # Errors
///
/// Returns `Err` if the storage write fails.
pub async fn set_site_config(state: &Arc<AppState>, key: &str, value: &str) -> anyhow::Result<()> {
    state.site_config.set(key, value).await?;
    Ok(())
}

/// Reset the mail-capture file: delete `path` if it exists. A missing file is
/// success (`rm -f` semantics — matching the shell the script used); any other
/// error propagates. The one fixture step that is not storage-linked; folding it
/// here lets the shell script be retired in full.
///
/// # Errors
///
/// Returns `Err` if `path` exists but cannot be removed.
pub fn reset_mail(path: &std::path::Path) -> anyhow::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::anyhow!("reset-mail: {}: {e}", path.display())),
    }
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

#[cfg(test)]
mod create_user_tests {
    //! `SQLite`-only by design (same rationale as `seed_tests`): `create_user`
    //! has no per-backend branching — it dispatches through
    //! `UserStorage::create_user`, implemented per backend — so the e2e matrix
    //! proves the dual-backend path; here we smoke the logic on `SQLite`.
    use super::*;
    use storage::test_support;

    #[tokio::test]
    async fn creates_a_lookupable_operator_and_rejects_duplicates() {
        let base = tempfile::TempDir::new().unwrap();
        let (state, _pool) = test_support::test_sqlite_state_with_pool(&base).await;

        let id = create_user(&state, "testoperator", "testpassword123", None, true)
            .await
            .expect("create ok");

        let u = state
            .users
            .get_user_by_username(&"testoperator".parse().unwrap())
            .await
            .expect("lookup ok")
            .expect("user exists");
        assert_eq!(u.user_id, id);
        assert!(u.is_operator, "--operator should set is_operator");

        // A freshly-init'd DB has a per-user uniqueness constraint, so a second
        // create with the same username surfaces as an error (no upsert).
        let err = create_user(&state, "testoperator", "testpassword123", None, false)
            .await
            .expect_err("duplicate username should error");
        let _ = err;
    }
}

#[cfg(test)]
mod set_site_config_tests {
    //! `SQLite`-only smoke test (same rationale as `seed_tests`): `set_site_config`
    //! dispatches through `SiteConfigStorage::set`, implemented per backend, so
    //! the e2e matrix proves both backends; here we smoke the logic on `SQLite`.
    use super::*;
    use storage::test_support;

    #[tokio::test]
    async fn set_round_trips_and_upserts() {
        let base = tempfile::TempDir::new().unwrap();
        let (state, _pool) = test_support::test_sqlite_state_with_pool(&base).await;

        set_site_config(&state, "site.registration_policy", "open")
            .await
            .expect("set ok");
        assert_eq!(
            state
                .site_config
                .get("site.registration_policy")
                .await
                .unwrap(),
            Some("open".to_string())
        );

        // set() is an upsert: a second write on the same key overwrites.
        set_site_config(&state, "site.registration_policy", "closed")
            .await
            .expect("upsert ok");
        assert_eq!(
            state
                .site_config
                .get("site.registration_policy")
                .await
                .unwrap(),
            Some("closed".to_string())
        );
    }
}

#[cfg(test)]
mod reset_mail_tests {
    use super::*;

    #[test]
    fn removes_an_existing_file_and_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("mail.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        assert!(path.exists());

        reset_mail(&path).expect("remove ok");
        assert!(!path.exists(), "file should be gone");

        // rm -f semantics: a second reset on the now-missing file is still Ok.
        reset_mail(&path).expect("missing file is not an error");
    }

    #[test]
    fn propagates_errors_other_than_not_found() {
        // `remove_file` on a directory fails with a non-`NotFound` error, so the
        // catch-all arm surfaces it (rather than swallowing it like a missing file).
        let dir = tempfile::TempDir::new().unwrap();
        let subdir = dir.path().join("a-directory");
        std::fs::create_dir(&subdir).unwrap();

        let err = reset_mail(&subdir).expect_err("removing a directory should error");
        assert!(err.to_string().contains("reset-mail"));
    }
}
