//! Test-only tooling that reaches jaunder's storage layer from OUTSIDE the
//! server process — a live-server Playwright e2e drives the `test-support`
//! binary over a process boundary to seed fixtures. It is the cross-process
//! sibling of the in-process `storage::test_support` module and is never linked
//! into the `jaunder` production binary (see ADR-0046, `test-support-seed-binary`).
//!
//! The seed core builds inputs from the shared `storage::seed_post_input` recipe
//! and writes them in one batched transaction (`PostStorage::create_posts`),
//! rather than the `storage::test_support::seed_posts` module helper: the e2e
//! suite shares one database across all tests, so seeds need per-user-unique,
//! content-shaped slugs/bodies that the module helper's fixed `seed-{i}` /
//! `# Post {i}` scheme cannot give.

use std::sync::Arc;

use common::display_name::DisplayName;
use common::ids::{FeedEventId, PostId, UserId};
use common::username::Username;
use host::feed::{FeedEventPhase, FeedPath};
use storage::{AppState, OperatorStatus, seed_post_input};

pub mod panic_gate;

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

/// Extracts the callback value only when its enclosing write received a commit
/// acknowledgement. Fixture writers cannot safely continue after an
/// acknowledgement loss because they need a definitive identifier or token.
fn confirmed_fixture_outcome<T>(
    outcome: common::MutationOutcome<T>,
    operation: impl std::fmt::Display,
) -> anyhow::Result<T> {
    match outcome {
        common::MutationOutcome::Confirmed(value) => Ok(value),
        common::MutationOutcome::CommitIndeterminate(_) => Err(anyhow::anyhow!(
            "{operation} commit acknowledgement was indeterminate"
        )),
    }
}

/// Seed `count` posts for `username` through the shared `seed_post_input`
/// recipe, written in one batched transaction — the same `create_post` write
/// path the server runs, so audience rows, rendered HTML, and both SQL dialects
/// come for free. `published` sets
/// `published_at = now` and a Public audience so the posts surface on the
/// timeline; otherwise they are drafts. Returns the created ids oldest-to-newest.
///
/// Slugs derive from `prefix` + index and the slug-uniqueness constraint is
/// per-user, so callers that share one database (the e2e suite) must pass a
/// distinct `prefix` for each user they seed — re-seeding the same user with the
/// same prefix would collide on the second invocation.
///
/// # Errors
///
/// Returns `Err` if `username` is invalid or unknown, a generated slug or body
/// fails to parse, or a post fails to persist.
pub async fn seed_posts_for_user(
    state: &Arc<AppState>,
    username: &str,
    count: usize,
    published: bool,
    prefix: &str,
) -> anyhow::Result<Vec<PostId>> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let user = state
        .users
        .get_user_by_username(&uname)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {username}"))?;

    let mut inputs = Vec::with_capacity(count);
    for i in 0..count {
        let slug = seed_slug(prefix, i).parse().map_err(|_| {
            anyhow::anyhow!("generated slug invalid for prefix {prefix:?} index {i}")
        })?;
        // Unlike the slug, whose validity depends on `prefix`, `seed_body` always emits a
        // literal `Body for …` line, so the non-blank invariant holds by construction.
        let Ok(body) = seed_body(prefix, i).parse() else {
            unreachable!("seed_body always yields a non-blank body");
        };
        inputs.push(seed_post_input(user.user_id, slug, body, published));
    }
    let posts = Arc::clone(&state.posts);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            Box::pin(async move { posts.create_posts(transaction, &inputs).await })
        })
        .await
        .map_err(|error| anyhow::anyhow!("batch seed of {count} posts failed: {error}"))?;
    confirmed_fixture_outcome(outcome, format_args!("batch seed of {count} posts"))
}

/// Seed terminal feed events through the same storage lifecycle used by the
/// worker. This is e2e-only fixture setup; it deliberately avoids queue SQL.
///
/// # Errors
///
/// Returns an error if a generated feed path or any storage transition fails.
pub async fn seed_dead_letters(
    state: &Arc<AppState>,
    phase: FeedEventPhase,
    count: usize,
) -> anyhow::Result<Vec<FeedEventId>> {
    let mut ids = Vec::with_capacity(count);
    for index in 0..count {
        let feed_path = format!("/~websub-fixture-{index}/feed.rss")
            .parse::<FeedPath>()
            .map_err(|_| anyhow::anyhow!("generated WebSub fixture feed path was invalid"))?;
        let feed_events = Arc::clone(&state.feed_events);
        let diagnostic = format!("fixture {phase:?} failure {index}");
        let id = confirmed_fixture_outcome(
            state
                .write_scope
                .run(move |transaction| {
                    Box::pin(async move {
                        let id = feed_events.enqueue(transaction, &feed_path).await?;
                        match phase {
                            FeedEventPhase::Regeneration => {
                                feed_events
                                    .dead_letter_regeneration(
                                        transaction,
                                        &[id],
                                        &diagnostic,
                                        common::time::UtcInstant::now(),
                                    )
                                    .await?;
                            }
                            FeedEventPhase::Publication => {
                                feed_events
                                    .dead_letter_publication(
                                        transaction,
                                        &[id],
                                        &diagnostic,
                                        common::time::UtcInstant::now(),
                                    )
                                    .await?;
                            }
                        }
                        Ok::<FeedEventId, storage::FeedEventError>(id)
                    })
                })
                .await
                .map_err(|error| {
                    anyhow::anyhow!("atomic WebSub dead-letter fixture failed: {error}")
                })?,
            "atomic WebSub dead-letter fixture",
        )?; // cov:ignore — llvm-cov attributes the already-covered outer `?` to its closing span
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
    display_name: Option<&DisplayName>,
    operator: bool,
) -> anyhow::Result<UserId> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let pw = password
        .parse::<host::password::Password>()
        .map_err(|e| anyhow::anyhow!("invalid password: {e}"))?;
    let prepared = storage::prepare_password(pw)
        .await
        .map_err(|error| anyhow::anyhow!("fixture password preparation failed: {error}"))?;
    let display_name = display_name.cloned();
    let operator = if operator {
        OperatorStatus::OPERATOR
    } else {
        OperatorStatus::STANDARD
    };
    let users = Arc::clone(&state.users);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            let users = Arc::clone(&users);
            Box::pin(async move {
                users
                    .create_user(
                        transaction,
                        &uname,
                        &prepared,
                        display_name.as_ref(),
                        operator,
                    )
                    .await
            })
        })
        .await
        .map_err(|error| anyhow::anyhow!("fixture user creation failed: {error}"))?;
    confirmed_fixture_outcome(outcome, "fixture user creation")
}

/// Reset the mail-capture file: delete `path` if it exists. A missing file is
/// success (`rm -f` semantics); any other error propagates. The one fixture
/// step that is not storage-linked.
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

/// The JSON seed record printed by the `seed-user` / `create-session`
/// subcommands: everything a browser context needs to boot authenticated
/// pre-paint — the session cookie and the advisory marker, both built by the
/// server's own primitives (`host::auth::session_cookie_header`,
/// `common::session_user`) so TypeScript never restates them (#791).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SeedRecord {
    pub username: String,
    pub user_id: i64,
    pub is_operator: bool,
    pub token: String,
    pub set_cookie: String,
    pub marker_key: String,
    pub marker: String,
}

/// The default session label for seeded sessions — distinct, so they are
/// obvious on `/sessions` and in debugging.
const DEFAULT_SEED_LABEL: &str = "E2E seed";

/// Build the record for one fresh session: mint the token through the real
/// `SessionStorage` path and derive both client-visible artifacts from the
/// server's own primitives.
async fn session_record(
    state: &Arc<AppState>,
    user_id: UserId,
    username: &Username,
    is_operator: bool,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord> {
    let label = label
        .unwrap_or(DEFAULT_SEED_LABEL)
        .parse::<common::session_label::SessionLabel>()
        .map_err(|e| anyhow::anyhow!("invalid session label: {e}"))?;
    let sessions = Arc::clone(&state.sessions);
    let outcome = state
        .write_scope
        .run(move |transaction| {
            let sessions = Arc::clone(&sessions);
            Box::pin(async move { sessions.create_session(transaction, user_id, &label).await })
        })
        .await
        .map_err(|error| anyhow::anyhow!("fixture session creation failed: {error}"))?;
    let token = confirmed_fixture_outcome(outcome, "fixture session creation")?;
    Ok(SeedRecord {
        username: username.to_string(),
        user_id: i64::from(user_id),
        is_operator,
        set_cookie: host::auth::session_cookie_header(&token, false),
        token: token.to_string(),
        marker_key: common::local_storage_key::LocalStorageKey::AuthMarker
            .as_ref()
            .to_owned(),
        marker: common::session_user::encode_marker(&common::session_user::SessionUser {
            username: username.clone(),
            is_operator,
        }),
    })
}

/// Create a fixture user (real `UserStorage::create_user` path — genuinely
/// argon2-hashed, so the account stays loginable through the UI) and a session
/// in one DB open. `label` defaults to `"E2E seed"`.
///
/// # Errors
///
/// Returns `Err` if the username or password is invalid, the label is invalid,
/// or the user cannot be created (e.g. a duplicate username).
pub async fn seed_user(
    state: &Arc<AppState>,
    username: &str,
    password: &str,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord> {
    let user_id = create_user(state, username, password, None, false).await?;
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    session_record(state, user_id, &uname, false, label).await
}

/// Create a session for an EXISTING user (e.g. the harness-seeded
/// `testoperator`); `is_operator` is read back from the user record so the
/// marker matches what a real login would write. `label` defaults to
/// `"E2E seed"`.
///
/// # Errors
///
/// Returns `Err` if the username is invalid or unknown, or the label is
/// invalid.
pub async fn create_session_for_user(
    state: &Arc<AppState>,
    username: &str,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord> {
    let uname = username
        .parse::<Username>()
        .map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let user = state
        .users
        .get_user_by_username(&uname)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {username}"))?;
    session_record(
        state,
        user.user_id,
        &uname,
        user.is_operator.is_operator(),
        label,
    )
    .await
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

    #[test]
    fn fixture_outcome_returns_a_confirmed_value() {
        assert_eq!(
            confirmed_fixture_outcome(common::MutationOutcome::Confirmed(42), "fixture operation")
                .expect("confirmed outcome"),
            42
        );
    }

    #[test]
    fn fixture_outcome_rejects_an_indeterminate_commit_with_the_operation_label() {
        let error = confirmed_fixture_outcome(
            common::MutationOutcome::CommitIndeterminate(()),
            "fixture operation",
        )
        .expect_err("indeterminate outcome");

        assert_eq!(
            error.to_string(),
            "fixture operation commit acknowledgement was indeterminate"
        );
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
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        let user = test_support::SeedUser::new().seed(&state).await;

        let ids = seed_posts_for_user(&state, &user.username, 3, true, "Timeline Post")
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
                &user.username,
                None,
                common::test_support::parse_row_limit("10"),
                &common::visibility::ViewerIdentity::Anonymous,
                common::time::UtcInstant::now(),
            )
            .await
            .expect("list ok");
        assert_eq!(page.len(), 3);
    }

    #[tokio::test]
    async fn rejects_a_prefix_that_cannot_form_a_valid_slug() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        let user = test_support::SeedUser::new().seed(&state).await;

        // A prefix with no alphanumerics collapses to an empty base, so the slug
        // would begin with '-' and fail `Slug` parsing — surfaced as an error
        // (not a panic) before any post is persisted.
        let err = seed_posts_for_user(&state, &user.username, 1, false, "***")
            .await
            .expect_err("invalid generated slug should error");
        assert!(err.to_string().contains("generated slug invalid"));
    }
}

#[cfg(test)]
mod dead_letter_tests {
    //! `SQLite`-only by design (same rationale as `seed_tests`): this fixture
    //! has no backend-specific policy and the e2e matrix exercises both
    //! supported storage dialects.
    use super::*;
    use common::pagination::PageSize;
    use storage::test_support;

    #[tokio::test]
    async fn seeds_terminal_events_in_each_requested_phase() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;

        for phase in [FeedEventPhase::Regeneration, FeedEventPhase::Publication] {
            let ids = seed_dead_letters(&state, phase, 2).await.expect("seed ok");
            assert_eq!(ids.len(), 2);

            let page = state
                .feed_events
                .dead_letters(phase, None, PageSize::default())
                .await
                .expect("dead-letter page");
            assert_eq!(page.events.len(), 2);
            assert!(page.next_cursor.is_none());
            for event in page.events {
                assert!(ids.contains(&event.id));
                assert_eq!(event.phase, phase);
                assert_eq!(event.attempts, 1);
                assert!(
                    event
                        .diagnostic
                        .as_deref()
                        .is_some_and(|diagnostic| diagnostic.contains("fixture"))
                );
            }
        }
    }

    #[tokio::test]
    async fn reports_atomic_seed_transaction_failures() {
        let test_support::TestEnv { state, base } = test_support::Backend::Sqlite.setup().await;
        base.close_pool().await;

        let error = seed_dead_letters(&state, FeedEventPhase::Regeneration, 1)
            .await
            .expect_err("closed storage must reject the atomic fixture");

        assert!(
            error
                .to_string()
                .contains("atomic WebSub dead-letter fixture failed")
        );
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
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;

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
        assert_eq!(u.is_operator, OperatorStatus::OPERATOR);

        // A freshly-init'd DB has a per-user uniqueness constraint, so a second
        // create with the same username surfaces as an error (no upsert).
        create_user(&state, "testoperator", "testpassword123", None, false)
            .await
            .expect_err("duplicate username should error");
    }
}

#[cfg(test)]
mod seed_session_tests {
    //! `SQLite`-only by design (same rationale as `seed_tests`): the seed
    //! functions have no per-backend branching — they dispatch through
    //! `UserStorage` / `SessionStorage`, implemented per backend — so the e2e
    //! matrix proves the dual-backend path; here we smoke the logic on `SQLite`.
    use super::*;
    use common::local_storage_key::LocalStorageKey;
    use common::session_user::decode_marker;
    use common::token::RawToken;
    use storage::test_support;

    /// The token a browser would send back, recovered from the record's
    /// `Set-Cookie` value exactly as a client would: the first `name=value`
    /// pair, up to the first `;`.
    fn cookie_token(record: &SeedRecord) -> RawToken {
        record
            .set_cookie
            .split(';')
            .next()
            .expect("cookie pair")
            .strip_prefix("session=")
            .expect("session cookie")
            .parse()
            .expect("token parses")
    }

    async fn authenticate_session(
        state: &Arc<AppState>,
        token: &RawToken,
    ) -> anyhow::Result<storage::SessionRecord> {
        let token = token.clone();
        let sessions = Arc::clone(&state.sessions);
        let outcome = state
            .write_scope
            .run(move |transaction| {
                let sessions = Arc::clone(&sessions);
                Box::pin(async move { sessions.authenticate(transaction, &token).await })
            })
            .await
            .map_err(|error| anyhow::anyhow!("fixture session authentication failed: {error}"))?;
        confirmed_fixture_outcome(outcome, "fixture session authentication")
    }

    #[tokio::test]
    async fn seed_user_returns_a_session_that_authenticates() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;

        let record = seed_user(&state, "alice", "password123", None)
            .await
            .expect("seed ok");

        // The cookie's token authenticates and resolves to the seeded user.
        let token = cookie_token(&record);
        let session = authenticate_session(&state, &token)
            .await
            .expect("token authenticates");
        assert_eq!(session.user_id, UserId::from(record.user_id));

        // The marker round-trips to the seeded identity, keyed by the shared
        // registry — never a restated literal.
        assert_eq!(record.marker_key, LocalStorageKey::AuthMarker.as_ref());
        let marker = decode_marker(&record.marker).expect("marker decodes");
        assert_eq!(marker.username, "alice");
        assert!(!marker.is_operator);

        // The default label makes seeded sessions obvious on /sessions.
        let sessions = state
            .sessions
            .list_sessions(session.user_id)
            .await
            .expect("list ok");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].label, "E2E seed");
    }

    #[tokio::test]
    async fn seed_user_honours_an_explicit_label() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;

        let record = seed_user(&state, "alice", "password123", Some("CI bot"))
            .await
            .expect("seed ok");
        let sessions = state
            .sessions
            .list_sessions(UserId::from(record.user_id))
            .await
            .expect("list ok");
        assert_eq!(sessions[0].label, "CI bot");
    }

    #[tokio::test]
    async fn create_session_for_user_reflects_the_operator_flag() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        create_user(&state, "testoperator", "testpassword123", None, true)
            .await
            .expect("create ok");

        let record = create_session_for_user(&state, "testoperator", None)
            .await
            .expect("session ok");
        let marker = decode_marker(&record.marker).expect("marker decodes");
        assert!(marker.is_operator, "operator user's marker must say so");
        let token = cookie_token(&record);
        authenticate_session(&state, &token)
            .await
            .expect("token authenticates");
    }

    #[tokio::test]
    async fn create_session_for_user_unknown_username_errors() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        create_session_for_user(&state, "ghost", None)
            .await
            .expect_err("unknown user should error");
    }

    #[tokio::test]
    async fn seed_user_duplicate_username_errors() {
        let test_support::TestEnv { state, base: _base } =
            test_support::Backend::Sqlite.setup().await;
        seed_user(&state, "alice", "password123", None)
            .await
            .expect("first seed ok");
        seed_user(&state, "alice", "password123", None)
            .await
            .expect_err("duplicate username should error");
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
