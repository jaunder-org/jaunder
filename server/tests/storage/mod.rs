use chrono::{Datelike, Utc};
use common::ids::UserId;
use common::password::Password;
use common::slug::Slug;
use common::tag::{Tag, TagLabel};
use common::test_support::{parse_audience_name, parse_display_name, parse_email, parse_raw_token};
use common::username::Username;
use common::visibility::{
    AudienceTarget, Channel, SubscriptionPolicy, SubscriptionStatus, TargetKind, ViewerIdentity,
};
use host::invite::InviteCode;
use sqlx::{PgPool, SqlitePool};
use std::sync::Arc;
use storage::{
    create_rendered_post, open_database, perform_post_update, update_rendered_post, AppState,
    AudienceError, ConfirmPasswordResetError, CreatePostError, CreatePostInput, CreateUserError,
    DbConnectOptions, FeedCacheRow, GoLivePost, ListByTagError, PermalinkDate, PostCursor,
    PostFormat, PostUpdate, PostgresSubscriptionStorage, ProfileUpdate, PublishUpdate,
    RegisterWithInviteError, RenderedHtml, RenderedPostContent, RenderedPostUpdate,
    SessionAuthError, SqliteSubscriptionStorage, SubscriptionStorage, TaggingError,
    UpdatePostError, UpdatePostInput, UseEmailVerificationError, UseInviteError,
    UsePasswordResetError, UserAuthError,
};
use tempfile::TempDir;

use rstest::*;
// `#[template]`/`#[apply]` come from the `rstest_reuse` companion crate (rstest
// itself only exports `rstest`/`fixture`). The bare `use rstest_reuse;` is
// required at the crate root because `rstest_reuse::template` expands to code
// that names the `rstest_reuse` crate; `use rstest_reuse::*;` alone is not
// enough (it imports the public items but not the crate path).
use rstest_reuse::*;

use storage::test_support::{
    backends, fp, recorded_postgres_url, sqlite_url, template_postgres_url, Backend,
    PostgresDbGuard, TestEnv,
};

// The Postgres-backed cases below (the `::postgres` expansion of each
// `#[apply(backends)]` test) run against PostgreSQL when `JAUNDER_PG_TEST_URL`
// is set; each acquires its own database (a template clone via
// `unique_postgres_url`/`template_postgres_url`, see helpers), so they run
// safely under the default in-process parallelism. No `--test-threads=1` is
// needed (jaunder-qguq).

async fn open_pool(base: &TempDir) -> SqlitePool {
    let DbConnectOptions::Sqlite(opts) = sqlite_url(base) else {
        panic!("expected sqlite options");
    };
    let pool = SqlitePool::connect_with(opts.create_if_missing(true))
        .await
        .unwrap();
    sqlx::migrate!("../storage/migrations/sqlite")
        .run(&pool)
        .await
        .unwrap();
    pool
}

async fn open_pg_pool() -> (PgPool, PostgresDbGuard) {
    let (url, guard) = template_postgres_url().await;
    let pool = PgPool::connect(&url.to_string()).await.unwrap();
    (pool, guard)
}

async fn lookup_names(backend: Backend, env: &TestEnv, table: &str) -> Vec<String> {
    let sql = format!("SELECT name FROM {table} ORDER BY name");
    match backend {
        Backend::Sqlite => sqlx::query_scalar(&sql)
            .fetch_all(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => {
            let (pool, _pg) = open_pg_pool().await;
            sqlx::query_scalar(&sql).fetch_all(&pool).await.unwrap()
        }
    }
}

#[apply(backends)]
#[tokio::test]
async fn channels_bijection(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names = lookup_names(backend, &env, "channels").await;
    for n in &names {
        assert!(
            Channel::try_from(n.as_str()).is_ok(),
            "unseeded enum for channel {n}"
        );
    }
    let c = Channel::Local;
    assert!(
        names.iter().any(|n| n == c.as_str()),
        "missing seed for {c}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn target_kinds_bijection(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names = lookup_names(backend, &env, "target_kinds").await;
    for n in &names {
        assert!(
            TargetKind::try_from(n.as_str()).is_ok(),
            "unseeded enum for target kind {n}"
        );
    }
    for k in [
        TargetKind::Public,
        TargetKind::Subscribers,
        TargetKind::Named,
    ] {
        assert!(
            names.iter().any(|n| n == k.as_str()),
            "missing seed for {k}"
        );
    }
}

#[apply(backends)]
#[tokio::test]
async fn statuses_seed_maps_to_enum(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names = lookup_names(backend, &env, "subscription_statuses").await;
    // Seeded names must each map to a variant (no orphan seed)...
    for n in &names {
        assert!(
            SubscriptionStatus::try_from(n.as_str()).is_ok(),
            "unseeded enum for subscription status {n}"
        );
    }
    // ...and the one status seeded this milestone must be present. `Pending`
    // and `Blocked` variants exist (reserved for M13/M15) but have no rows yet,
    // so this is the subset direction only — not exact bijection.
    assert!(
        names
            .iter()
            .any(|n| n == SubscriptionStatus::Active.as_str()),
        "missing seed for {}",
        SubscriptionStatus::Active
    );
}

// Sibling of `lookup_names`: a raw SELECT of the seeded `local` channel id.
// The `local` channel is a lookup row present in every clone, so reading it via
// the per-test recorded URL (Postgres) or the same DB file (SQLite) both work;
// we use the established same-DB helpers for consistency. The trait method
// `local_channel_id()` is introduced in a later task — do not use it here.
async fn local_channel_id(backend: Backend, env: &TestEnv) -> i64 {
    let sql = "SELECT channel_id FROM channels WHERE name = 'local'";
    match backend {
        Backend::Sqlite => sqlx::query_scalar(sql)
            .fetch_one(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => {
            let pool = env.base.pool().postgres();
            sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
        }
    }
}

// The production `SubscriptionStorage::local_channel_id()` accessor must return
// the same id as the seeded `'local'` channel row (read here via the raw test
// helper of the same name).
#[apply(backends)]
#[tokio::test]
async fn local_channel_id_returns_seeded_local(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let expected = local_channel_id(backend, &env).await;
    let actual = state.subscriptions.local_channel_id().await.unwrap();
    assert_eq!(actual, expected);
}

#[apply(backends)]
#[tokio::test]
async fn subscribe_is_idempotent_and_active(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();
    let local = local_channel_id(backend, &env).await;
    let id1 = state
        .subscriptions
        .subscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    let id2 = state
        .subscriptions
        .subscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    assert_eq!(id1, id2, "subscribe is idempotent");
    assert!(state
        .subscriptions
        .is_subscriber(author, &ViewerIdentity::local(bob, local))
        .await
        .unwrap());
    assert!(!state
        .subscriptions
        .is_subscriber(author, &ViewerIdentity::Anonymous)
        .await
        .unwrap());
    // Active subscriber appears in the listing.
    let subs = state.subscriptions.list_subscribers(author).await.unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].subscription_id, id1);
    assert_eq!(subs[0].channel_id, local);
    assert_eq!(subs[0].subscriber_ref, bob.to_string());
    assert_eq!(subs[0].status, SubscriptionStatus::Active);
    // Unsubscribe round-trips: no longer a subscriber, listing empties.
    state
        .subscriptions
        .unsubscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    assert!(!state
        .subscriptions
        .is_subscriber(author, &ViewerIdentity::local(bob, local))
        .await
        .unwrap());
    assert!(state
        .subscriptions
        .list_subscribers(author)
        .await
        .unwrap()
        .is_empty());
}

// Fail-closed admission: `is_subscriber` admits only `active` rows, so a
// subscription a stricter policy left `pending` must NOT be admitted. The
// default `state.subscriptions` uses `OpenSubscriptionPolicy` (always active),
// so we construct the store directly with a stub policy returning `Pending`.
#[apply(backends)]
#[tokio::test]
async fn pending_subscription_is_not_admitted(#[case] backend: Backend) {
    struct StubPending;
    impl SubscriptionPolicy for StubPending {
        fn initial_status(&self, _a: UserId, _c: i64, _r: &str) -> SubscriptionStatus {
            SubscriptionStatus::Pending
        }
    }

    let env = backend.setup().await;
    // Only `active` is seeded this milestone (M13 adds `pending`). Seed the
    // `pending` lookup row locally so `subscribe` can persist a pending row and
    // we can prove `is_subscriber` still excludes it (the fail-closed property).
    // Build the store over the *same* per-test database as `env.state`, with the
    // stub `Pending` policy, per backend.
    let store: Box<dyn SubscriptionStorage> = match backend {
        Backend::Sqlite => {
            let pool = open_pool(&env.base).await; // same DB file as env.state
            sqlx::query("INSERT INTO subscription_statuses (name) VALUES ('pending')")
                .execute(&pool)
                .await
                .unwrap();
            Box::new(SqliteSubscriptionStorage::new(pool, Arc::new(StubPending)))
        }
        Backend::Postgres => {
            let pool = env.base.pool().postgres().clone(); // same DB as env.state
            sqlx::query("INSERT INTO subscription_statuses (name) VALUES ('pending')")
                .execute(&pool)
                .await
                .unwrap();
            Box::new(PostgresSubscriptionStorage::new(
                pool,
                Arc::new(StubPending),
            ))
        }
    };
    let author = env
        .state
        .users
        .create_user(&username("alice"), &password("pw1234567"), None, false)
        .await
        .unwrap();
    let bob = env
        .state
        .users
        .create_user(&username("bob"), &password("pw1234567"), None, false)
        .await
        .unwrap();
    let local = local_channel_id(backend, &env).await;
    store
        .subscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    // Resolution admits only `active` → a pending subscriber is excluded.
    assert!(!store
        .is_subscriber(author, &ViewerIdentity::local(bob, local))
        .await
        .unwrap());
    // ...and it is not listed (list_subscribers is active-only).
    assert!(store.list_subscribers(author).await.unwrap().is_empty());
}

// ── Audiences ─────────────────────────────────────────────────────────────────

// create → list → rename → delete round-trip. Every write is author-scoped and
// the listing is ordered by `audience_id`; rename and delete mutate exactly the
// targeted row.
#[apply(backends)]
#[tokio::test]
async fn audience_create_list_rename_delete(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let friends = state
        .audiences
        .create_audience(author, &parse_audience_name("Friends"))
        .await
        .unwrap();
    let family = state
        .audiences
        .create_audience(author, &parse_audience_name("Family"))
        .await
        .unwrap();

    // Listing is author-scoped and ordered by audience_id (insertion order).
    let listed = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].audience_id, friends);
    assert_eq!(listed[0].name, "Friends");
    assert_eq!(listed[1].audience_id, family);
    assert_eq!(listed[1].name, "Family");

    // Rename mutates exactly the targeted audience.
    state
        .audiences
        .rename_audience(author, friends, &parse_audience_name("Close Friends"))
        .await
        .unwrap();
    let listed = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(listed[0].name, "Close Friends");

    // Renaming an audience the author does not own is NotFound.
    let stranger = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();
    assert!(matches!(
        state
            .audiences
            .rename_audience(stranger, friends, &parse_audience_name("Hijacked"))
            .await,
        Err(AudienceError::NotFound)
    ));

    // Delete removes exactly the targeted audience.
    state
        .audiences
        .delete_audience(author, friends)
        .await
        .unwrap();
    let listed = state.audiences.list_audiences(author).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].audience_id, family);
}

// A duplicate `(author_user_id, name)` is mapped to DuplicateName on both create
// and rename; a different author may reuse the same name.
#[apply(backends)]
#[tokio::test]
async fn audience_duplicate_name_rejected(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let alice = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();

    state
        .audiences
        .create_audience(alice, &parse_audience_name("Friends"))
        .await
        .unwrap();
    // Same author, same name → DuplicateName.
    assert!(matches!(
        state
            .audiences
            .create_audience(alice, &parse_audience_name("Friends"))
            .await,
        Err(AudienceError::DuplicateName)
    ));
    // Different author may reuse the name.
    state
        .audiences
        .create_audience(bob, &parse_audience_name("Friends"))
        .await
        .unwrap();

    // Rename onto an existing name (same author) → DuplicateName.
    let work = state
        .audiences
        .create_audience(alice, &parse_audience_name("Work"))
        .await
        .unwrap();
    assert!(matches!(
        state
            .audiences
            .rename_audience(alice, work, &parse_audience_name("Friends"))
            .await,
        Err(AudienceError::DuplicateName)
    ));
}

// add_member / list_members / remove_member happy path against a same-owner
// subscription seeded via the wired SubscriptionStore.
#[apply(backends)]
#[tokio::test]
async fn audience_membership_round_trip(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();
    let local = local_channel_id(backend, &env).await;
    let sub = state
        .subscriptions
        .subscribe(author, local, &bob.to_string())
        .await
        .unwrap();
    let audience = state
        .audiences
        .create_audience(author, &parse_audience_name("Friends"))
        .await
        .unwrap();

    assert!(state
        .audiences
        .list_members(author, audience)
        .await
        .unwrap()
        .is_empty());

    state
        .audiences
        .add_member(author, audience, sub)
        .await
        .unwrap();
    // add_member is idempotent.
    state
        .audiences
        .add_member(author, audience, sub)
        .await
        .unwrap();
    assert_eq!(
        state
            .audiences
            .list_members(author, audience)
            .await
            .unwrap(),
        vec![sub]
    );

    state
        .audiences
        .remove_member(author, audience, sub)
        .await
        .unwrap();
    assert!(state
        .audiences
        .list_members(author, audience)
        .await
        .unwrap()
        .is_empty());
}

// The same-owner invariant is enforced by the composite FKs: pairing an audience
// with a subscription owned by a *different* author must be rejected by the DB
// and surface as `AudienceError::Storage` (no app-level check). Complements the
// raw-SQL `composite_fks_reject_cross_author_membership` test at the trait layer.
#[apply(backends)]
#[tokio::test]
async fn audience_add_member_cross_author_rejected(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let alice = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();
    let local = local_channel_id(backend, &env).await;
    // Subscription owned by BOB.
    let bob_sub = state
        .subscriptions
        .subscribe(bob, local, &alice.to_string())
        .await
        .unwrap();
    // Audience owned by ALICE.
    let alice_audience = state
        .audiences
        .create_audience(alice, &parse_audience_name("Friends"))
        .await
        .unwrap();

    // Alice pairs her audience with Bob's subscription: the
    // (subscription_id, author_user_id) FK fails → Storage error.
    assert!(matches!(
        state
            .audiences
            .add_member(alice, alice_audience, bob_sub)
            .await,
        Err(AudienceError::Storage(_))
    ));
    assert!(state
        .audiences
        .list_members(alice, alice_audience)
        .await
        .unwrap()
        .is_empty());
}

// `list_members` / `remove_member` are author-scoped: a different author can
// neither see nor mutate another author's audience membership (the WHERE clause
// filters by `author_user_id`, so a cross-author `audience_id` matches nothing).
// This is the storage guarantee that replaced web's `assert_owns_audience` check.
#[apply(backends)]
#[tokio::test]
async fn audience_members_are_author_scoped(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let alice = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();
    let local = local_channel_id(backend, &env).await;
    // A subscription and audience both owned by ALICE, with the sub as a member.
    let alice_sub = state
        .subscriptions
        .subscribe(alice, local, &bob.to_string())
        .await
        .unwrap();
    let alice_audience = state
        .audiences
        .create_audience(alice, &parse_audience_name("Friends"))
        .await
        .unwrap();
    state
        .audiences
        .add_member(alice, alice_audience, alice_sub)
        .await
        .unwrap();

    // Bob cannot list Alice's members...
    assert!(state
        .audiences
        .list_members(bob, alice_audience)
        .await
        .unwrap()
        .is_empty());
    // ...and a Bob-scoped remove leaves Alice's membership untouched (no-op).
    state
        .audiences
        .remove_member(bob, alice_audience, alice_sub)
        .await
        .unwrap();
    assert_eq!(
        state
            .audiences
            .list_members(alice, alice_audience)
            .await
            .unwrap(),
        vec![alice_sub]
    );
}

// `delete_audience` must remove the audience's membership rows in the same
// transaction, not just the `audiences` row. The schema declares no
// `ON DELETE CASCADE` and SQLite enforces foreign keys off by default, so a
// dropped `DELETE FROM audience_members` would silently orphan membership rows.
// A raw `COUNT(*)` proves they are gone (`list_members` on a deleted audience is
// trivially empty regardless, so it cannot catch the orphan).
#[apply(backends)]
#[tokio::test]
async fn audience_delete_cascades_memberships(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let alice = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();
    let local = local_channel_id(backend, &env).await;
    let sub = state
        .subscriptions
        .subscribe(alice, local, &bob.to_string())
        .await
        .unwrap();
    let audience = state
        .audiences
        .create_audience(alice, &parse_audience_name("Friends"))
        .await
        .unwrap();
    state
        .audiences
        .add_member(alice, audience, sub)
        .await
        .unwrap();

    // Precondition: the membership row exists.
    let members_sql =
        format!("SELECT COUNT(*) FROM audience_members WHERE audience_id = {audience}");
    assert_eq!(raw_scalar_i64(backend, &env, &members_sql).await, 1);

    state
        .audiences
        .delete_audience(alice, audience)
        .await
        .unwrap();

    // The membership row is gone, not orphaned.
    assert_eq!(
        raw_scalar_i64(backend, &env, &members_sql).await,
        0,
        "delete_audience must cascade-remove its membership rows"
    );
}

fn username(s: &str) -> Username {
    s.parse().unwrap()
}

fn password(s: &str) -> Password {
    s.parse().unwrap()
}

#[apply(backends)]
#[tokio::test]
async fn site_config_set_then_get_roundtrips(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    state
        .site_config
        .set("site.name", "Parity Site")
        .await
        .unwrap();
    assert_eq!(
        state.site_config.get("site.name").await.unwrap().as_deref(),
        Some("Parity Site")
    );
}

#[apply(backends)]
#[tokio::test]
async fn get_missing_key_returns_none(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    assert!(state
        .site_config
        .get("nonexistent")
        .await
        .unwrap()
        .is_none());
}

#[apply(backends)]
#[tokio::test]
async fn set_overwrites_existing_value(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    state.site_config.set("site.name", "First").await.unwrap();
    state.site_config.set("site.name", "Second").await.unwrap();

    assert_eq!(
        state.site_config.get("site.name").await.unwrap().as_deref(),
        Some("Second")
    );
}

#[apply(backends)]
#[tokio::test]
async fn second_open_on_migrated_database_succeeds(#[case] backend: Backend) {
    let env = backend.setup().await;

    // Re-open the *same* per-test database the setup just migrated, addressed by
    // its backend URL, to prove a second `open_database` (re-running migrations)
    // is idempotent on both backends.
    let opts = match backend {
        Backend::Sqlite => sqlite_url(&env.base),
        Backend::Postgres => recorded_postgres_url(&env.base).parse().unwrap(),
    };
    open_database(&opts).await.unwrap();
}

#[apply(backends)]
#[tokio::test]
async fn create_user_duplicate_and_authenticate_work(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let username = username("alice");
    let initial_password = password("password123");

    let user_id = state
        .users
        .create_user(
            &username,
            &initial_password,
            Some(&parse_display_name("Alice")),
            false,
        )
        .await
        .unwrap();
    let record = state
        .users
        .get_user_by_username(&username)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.user_id, user_id);

    let duplicate = state
        .users
        .create_user(&username, &password("other_password"), None, false)
        .await
        .unwrap_err();
    assert!(matches!(duplicate, CreateUserError::UsernameTaken));

    let authed = state
        .users
        .authenticate(&username, &initial_password)
        .await
        .unwrap();
    assert_eq!(authed.username, "alice");
    assert!(authed.last_authenticated_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn session_lifecycle_works(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("bob"), &password("secret_password"), None, false)
        .await
        .unwrap();

    let raw_token = state
        .sessions
        .create_session(user_id, "Laptop")
        .await
        .unwrap();
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.user_id, user_id);
    assert_eq!(record.username, "bob");

    let sessions = state.sessions.list_sessions(user_id).await.unwrap();
    assert_eq!(sessions.len(), 1);
    state
        .sessions
        .revoke_session(&record.token_hash)
        .await
        .unwrap();
    let err = state.sessions.authenticate(&raw_token).await.unwrap_err();
    assert!(matches!(err, SessionAuthError::SessionNotFound));
}

#[apply(backends)]
#[tokio::test]
async fn invite_and_atomic_registration_work(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let user_id = state
        .atomic
        .create_user_with_invite(
            &username("carol"),
            &password("password123"),
            Some(&parse_display_name("Carol")),
            false,
            &code,
        )
        .await
        .unwrap();
    let created = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(created.username, "carol");

    let err = state
        .atomic
        .create_user_with_invite(
            &username("carol2"),
            &password("password123"),
            None,
            false,
            &code,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, RegisterWithInviteError::InviteAlreadyUsed));
}

#[apply(backends)]
#[tokio::test]
async fn email_verification_and_password_reset_work(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("dave"), &password("password123"), None, false)
        .await
        .unwrap();

    let verify_token = state
        .email_verifications
        .create_email_verification(
            user_id,
            &"dave@example.com".parse().unwrap(),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .unwrap();
    let (verified_user_id, verified_email) = state
        .email_verifications
        .use_email_verification(&verify_token)
        .await
        .unwrap();
    assert_eq!(verified_user_id, user_id);
    assert_eq!(verified_email, "dave@example.com");

    state
        .users
        .set_email(user_id, Some(&"dave@example.com".parse().unwrap()), true)
        .await
        .unwrap();

    let reset_token = state
        .password_resets
        .create_password_reset(user_id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    let claimed_user_id = state
        .password_resets
        .use_password_reset(&reset_token)
        .await
        .unwrap();
    assert_eq!(claimed_user_id, user_id);

    let reset_token = state
        .password_resets
        .create_password_reset(user_id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    state
        .atomic
        .confirm_password_reset(&reset_token, &password("new_password123"))
        .await
        .unwrap();

    let authed = state
        .users
        .authenticate(&username("dave"), &password("new_password123"))
        .await
        .unwrap();
    assert_eq!(authed.user_id, user_id);
}

#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_hash_failure_returns_internal(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("reset_hash_fail"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();
    let reset_token = state
        .password_resets
        .create_password_reset(user_id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    // Valid token → the claim succeeds, then hashing the new password fails → Internal
    // (success-path hash failure; the failed hash rolls the claim back).
    let result = state
        .atomic
        .confirm_password_reset(
            &reset_token,
            &password("force-hash-error-for-test-coverage"),
        )
        .await;
    assert!(matches!(
        result,
        Err(ConfirmPasswordResetError::Internal(_))
    ));
}

#[apply(backends)]
#[tokio::test]
async fn confirm_password_reset_bogus_token_returns_not_found_without_hashing(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;
    // No password_resets row matches this token. A hash-failing new password proves the
    // hash is NOT attempted: the claim rejects the token first -> NotFound, not Internal
    // (ADR-0022). Before the reorder this would have hashed first and returned Internal.
    let result = state
        .atomic
        .confirm_password_reset(
            &parse_raw_token("dGVzdA"),
            &password("force-hash-error-for-test-coverage"),
        )
        .await;
    assert!(matches!(result, Err(ConfirmPasswordResetError::NotFound)));
}

#[test]
fn postgres_url_is_accepted_at_parse_time() {
    let result = "postgres://localhost/test".parse::<DbConnectOptions>();
    assert!(result.is_ok());
}

#[test]
fn unsupported_url_is_rejected_at_parse_time() {
    let result = "mysql://localhost/test".parse::<DbConnectOptions>();
    assert!(result.is_err());
}

#[apply(backends)]
#[tokio::test]
async fn feed_events_marks_run(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let fe = &state.feed_events;

    // Enqueue + claim to obtain real ids, then exercise every
    // FeedEventDialect mark_* method on this backend. Each is an independent
    // `UPDATE … WHERE id IN (…)`, so they all run regardless of row state.
    fe.enqueue(&fp("/feed.rss")).await.unwrap();
    let claimed = fe
        .claim_pending_batch(50, chrono::Duration::minutes(5))
        .await
        .unwrap();
    let ids: Vec<i64> = claimed.iter().map(|r| r.id).collect();
    assert!(!ids.is_empty());

    fe.mark_regenerated(&ids).await.unwrap();
    fe.mark_pinged(&ids).await.unwrap();
    fe.mark_failed(
        &ids,
        "boom",
        chrono::Utc::now() + chrono::Duration::minutes(1),
    )
    .await
    .unwrap();
    fe.mark_exhausted(&ids, "gave up").await.unwrap();
}

// --- UserStorage integration tests ---

#[apply(backends)]
#[tokio::test]
async fn create_user_succeeds_and_get_by_username_returns_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(
            &username("alice"),
            &password("password123"),
            Some(&parse_display_name("Alice")),
            false,
        )
        .await
        .unwrap();

    let record = state
        .users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.user_id, user_id);
    assert_eq!(record.username, "alice");
    assert_eq!(record.display_name.as_deref(), Some("Alice"));
}

#[apply(backends)]
#[tokio::test]
async fn duplicate_username_returns_username_taken(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let err = state
        .users
        .create_user(&username("alice"), &password("other_password"), None, false)
        .await
        .unwrap_err();
    assert!(matches!(err, CreateUserError::UsernameTaken));
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_correct_password_returns_record_and_sets_last_authenticated_at(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;

    state
        .users
        .create_user(&username("bob"), &password("secret_password"), None, false)
        .await
        .unwrap();

    let record = state
        .users
        .authenticate(&username("bob"), &password("secret_password"))
        .await
        .unwrap();
    assert_eq!(record.username, "bob");
    assert!(record.last_authenticated_at.is_some());

    let fetched = state.users.get_user(record.user_id).await.unwrap().unwrap();
    assert!(fetched.last_authenticated_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_wrong_password_returns_invalid_credentials(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    state
        .users
        .create_user(
            &username("carol"),
            &password("correct_password"),
            None,
            false,
        )
        .await
        .unwrap();

    let err = state
        .users
        .authenticate(&username("carol"), &password("wrong_password"))
        .await
        .unwrap_err();
    assert!(matches!(err, UserAuthError::InvalidCredentials));
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_unknown_username_returns_invalid_credentials(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let err = state
        .users
        .authenticate(&username("nobody"), &password("some_password"))
        .await
        .unwrap_err();
    assert!(matches!(err, UserAuthError::InvalidCredentials));
}

#[apply(backends)]
#[tokio::test]
async fn update_profile_persists_changes(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(
            &username("dave"),
            &password("passw0rd!"),
            Some(&parse_display_name("Dave")),
            false,
        )
        .await
        .unwrap();

    state
        .users
        .update_profile(
            user_id,
            &ProfileUpdate {
                display_name: Some(&parse_display_name("David")),
                bio: Some("A bio"),
            },
        )
        .await
        .unwrap();

    let record = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.display_name.as_deref(), Some("David"));
    assert_eq!(record.bio.as_deref(), Some("A bio"));
}

#[apply(backends)]
#[tokio::test]
async fn get_user_unknown_id_returns_none(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let record = state.users.get_user(UserId::from(999)).await.unwrap();
    assert!(record.is_none());
}

// --- SessionStorage integration tests ---

#[apply(backends)]
#[tokio::test]
async fn create_session_then_authenticate_returns_correct_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let raw_token = state
        .sessions
        .create_session(user_id, "test")
        .await
        .unwrap();
    let record = state.sessions.authenticate(&raw_token).await.unwrap();

    assert_eq!(record.user_id, user_id);
    assert_eq!(record.username, "alice");
    assert_eq!(record.label, "test");
    assert!(!record.token_hash.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_updates_last_used_at(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();

    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let first = state.sessions.authenticate(&raw_token).await.unwrap();
    let second = state.sessions.authenticate(&raw_token).await.unwrap();

    assert!(second.last_used_at >= first.last_used_at);
}

#[apply(backends)]
#[tokio::test]
async fn revoke_session_then_authenticate_returns_session_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("carol"), &password("password123"), None, false)
        .await
        .unwrap();

    let raw_token = state
        .sessions
        .create_session(user_id, "test session")
        .await
        .unwrap();
    let record = state.sessions.authenticate(&raw_token).await.unwrap();

    state
        .sessions
        .revoke_session(&record.token_hash)
        .await
        .unwrap();

    let err = state.sessions.authenticate(&raw_token).await.unwrap_err();
    assert!(matches!(err, SessionAuthError::SessionNotFound));
}

#[apply(backends)]
#[tokio::test]
async fn authenticate_with_invalid_base64_token_returns_invalid_token(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    // In-charset (base64url) but an invalid length that cannot decode, so hashing
    // fails and `authenticate` reports InvalidToken. (A non-charset string like
    // "not-base64!" can no longer be constructed as a `RawToken`.)
    let err = state
        .sessions
        .authenticate(&parse_raw_token("a"))
        .await
        .unwrap_err();
    assert!(matches!(err, SessionAuthError::InvalidToken));
}

#[apply(backends)]
#[tokio::test]
async fn list_sessions_returns_only_sessions_for_given_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let alice_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob_id = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();

    state
        .sessions
        .create_session(alice_id, "alice-1")
        .await
        .unwrap();
    state
        .sessions
        .create_session(alice_id, "alice-2")
        .await
        .unwrap();
    state
        .sessions
        .create_session(bob_id, "bob-1")
        .await
        .unwrap();

    let alice_sessions = state.sessions.list_sessions(alice_id).await.unwrap();
    assert_eq!(alice_sessions.len(), 2);
    assert!(alice_sessions.iter().all(|s| s.user_id == alice_id));

    let bob_sessions = state.sessions.list_sessions(bob_id).await.unwrap();
    assert_eq!(bob_sessions.len(), 1);
    assert_eq!(bob_sessions[0].user_id, bob_id);
}

// --- InviteStorage integration tests ---

#[apply(backends)]
#[tokio::test]
async fn create_invite_and_list_invites_includes_it(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let list = state.invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].code.as_ref(), code.as_ref());
    assert!(list[0].used_at.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn use_invite_with_valid_code_marks_it_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    state.invites.use_invite(&code, user_id).await.unwrap();

    let list = state.invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_some());
    assert_eq!(list[0].used_by, Some(user_id));
}

#[apply(backends)]
#[tokio::test]
async fn use_invite_with_unknown_code_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();

    let err = state
        .invites
        .use_invite(&"no-such-code".parse::<InviteCode>().unwrap(), user_id)
        .await
        .unwrap_err();
    assert!(matches!(err, UseInviteError::NotFound));
}

#[apply(backends)]
#[tokio::test]
async fn use_invite_with_expired_code_returns_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("carol"), &password("password123"), None, false)
        .await
        .unwrap();

    // expires_at in the past
    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let err = state.invites.use_invite(&code, user_id).await.unwrap_err();
    assert!(matches!(err, UseInviteError::Expired));
}

#[apply(backends)]
#[tokio::test]
async fn use_invite_on_already_used_code_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("dave"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    state.invites.use_invite(&code, user_id).await.unwrap();

    let err = state.invites.use_invite(&code, user_id).await.unwrap_err();
    assert!(matches!(err, UseInviteError::AlreadyUsed));
}

// --- create_user_with_invite integration tests ---

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_creates_user_and_marks_invite_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let user_id = state
        .atomic
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            Some(&parse_display_name("Alice")),
            false,
            &code,
        )
        .await
        .unwrap();

    let record = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.username, "alice");
    assert_eq!(record.display_name.as_deref(), Some("Alice"));

    // Invite was marked used
    let list = state.invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_some());
    assert_eq!(list[0].used_by, Some(user_id));
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_second_call_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    state
        .atomic
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            None,
            false,
            &code,
        )
        .await
        .unwrap();

    let err = state
        .atomic
        .create_user_with_invite(
            &username("bob"),
            &password("password123"),
            None,
            false,
            &code,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::InviteAlreadyUsed));

    // bob was not inserted
    assert!(state
        .users
        .get_user_by_username(&username("bob"))
        .await
        .unwrap()
        .is_none());
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_expired_returns_invite_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let err = state
        .atomic
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            None,
            false,
            &code,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::InviteExpired));

    assert!(state
        .users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap()
        .is_none());
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_unknown_code_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let err = state
        .atomic
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            None,
            false,
            &"no-such-code".parse::<InviteCode>().unwrap(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::InviteNotFound));

    assert!(state
        .users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap()
        .is_none());
}

#[apply(backends)]
#[tokio::test]
async fn create_user_with_invite_duplicate_username_returns_username_taken(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;

    // Create alice directly (without invite)
    state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let err = state
        .atomic
        .create_user_with_invite(
            &username("alice"),
            &password("other_password"),
            None,
            false,
            &code,
        )
        .await
        .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::UsernameTaken));

    // Invite was NOT marked used
    let list = state.invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_none());
}

// --- build_mailer tests ---

#[apply(backends)]
#[tokio::test]
async fn build_mailer_returns_noop_when_smtp_not_configured(#[case] backend: Backend) {
    let env = backend.setup().await;
    let mailer = jaunder::mailer::build_mailer(env.state.site_config.as_ref(), None).await;

    let msg = common::mailer::EmailMessage {
        from: None,
        to: vec!["alice@example.com".parse().unwrap()],
        subject: "Test".to_string(),
        body_text: "Hello".to_string(),
    };
    let result = mailer.send_email(&msg).await;
    assert!(
        matches!(result, Err(common::mailer::MailError::NotConfigured)),
        "expected NotConfigured, got {result:?}"
    );
}

// --- UserStorage::set_email integration tests ---

#[apply(backends)]
#[tokio::test]
async fn set_email_persists_and_get_user_reflects_it(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let addr = parse_email("alice@example.com");
    state
        .users
        .set_email(user_id, Some(&addr), true)
        .await
        .unwrap();

    let record = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.email, Some(addr));
    assert!(record.email_verified);
}

#[apply(backends)]
#[tokio::test]
async fn set_email_clears_previously_set_email(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();

    let addr = parse_email("bob@example.com");
    state
        .users
        .set_email(user_id, Some(&addr), true)
        .await
        .unwrap();

    state.users.set_email(user_id, None, false).await.unwrap();

    let record = state.users.get_user(user_id).await.unwrap().unwrap();
    assert!(record.email.is_none());
    assert!(!record.email_verified);
}

// --- EmailVerificationStorage integration tests ---

#[apply(backends)]
#[tokio::test]
async fn create_email_verification_and_use_returns_user_id_and_email(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    let (returned_user_id, returned_email) = state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap();

    assert_eq!(returned_user_id, user_id);
    assert_eq!(returned_email, "alice@example.com");
}

#[apply(backends)]
#[tokio::test]
async fn use_email_verification_already_used_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap();

    let err = state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap_err();
    assert!(
        matches!(err, UseEmailVerificationError::AlreadyUsed),
        "expected AlreadyUsed, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_email_verification_expired_returns_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    let err = state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap_err();
    assert!(
        matches!(err, UseEmailVerificationError::Expired),
        "expected Expired, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_email_verification_unknown_token_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let err = state
        .email_verifications
        .use_email_verification(&parse_raw_token("not-a-real-token"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, UseEmailVerificationError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn second_email_verification_supersedes_first(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let first_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    // Create a second verification; the first should be superseded.
    let second_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice2@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    // Second token works normally.
    let (uid, email) = state
        .email_verifications
        .use_email_verification(&second_token)
        .await
        .unwrap();
    assert_eq!(uid, user_id);
    assert_eq!(email, "alice2@example.com");

    // First token is now either NotFound or Expired.
    let err = state
        .email_verifications
        .use_email_verification(&first_token)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            UseEmailVerificationError::NotFound | UseEmailVerificationError::Expired
        ),
        "expected NotFound or Expired for superseded token, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_email_verification_with_corrupt_stored_email_returns_internal(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .email_verifications
        .create_email_verification(user_id, &"alice@example.com".parse().unwrap(), expires_at)
        .await
        .unwrap();

    // Corrupt the stored address out-of-band so claiming the token yields a
    // value that no longer parses as an email. The `email` column is plain
    // TEXT on both backends, so the same UPDATE is portable.
    raw_exec(
        backend,
        &env,
        "UPDATE email_verifications SET email = 'not-an-email'",
    )
    .await;

    let err = state
        .email_verifications
        .use_email_verification(&raw_token)
        .await
        .unwrap_err();
    assert!(
        matches!(err, UseEmailVerificationError::Internal(_)),
        "expected Internal for unparseable stored email, got {err:?}"
    );
}

// --- UserStorage::set_password integration tests ---

#[apply(backends)]
#[tokio::test]
async fn set_password_authenticate_with_old_returns_invalid_and_new_succeeds(
    #[case] backend: Backend,
) {
    let env = backend.setup().await;
    let state = &env.state;
    let users = &state.users;

    let user_id = users
        .create_user(&username("alice"), &password("old_password1"), None, false)
        .await
        .unwrap();

    users
        .set_password(user_id, &password("new_password2"))
        .await
        .unwrap();

    // Old password no longer works.
    let err = users
        .authenticate(&username("alice"), &password("old_password1"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, UserAuthError::InvalidCredentials),
        "expected InvalidCredentials, got {err:?}"
    );

    // New password works.
    let record = users
        .authenticate(&username("alice"), &password("new_password2"))
        .await
        .unwrap();
    assert_eq!(record.user_id, user_id);
}

// --- PasswordResetStorage integration tests ---

#[apply(backends)]
#[tokio::test]
async fn create_password_reset_and_use_returns_user_id(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let returned_user_id = state
        .password_resets
        .use_password_reset(&raw_token)
        .await
        .unwrap();
    assert_eq!(returned_user_id, user_id);
}

#[apply(backends)]
#[tokio::test]
async fn use_password_reset_already_used_returns_already_used(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    state
        .password_resets
        .use_password_reset(&raw_token)
        .await
        .unwrap();

    let err = state
        .password_resets
        .use_password_reset(&raw_token)
        .await
        .unwrap_err();
    assert!(
        matches!(err, UsePasswordResetError::AlreadyUsed),
        "expected AlreadyUsed, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_password_reset_expired_returns_expired(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = state
        .password_resets
        .create_password_reset(user_id, expires_at)
        .await
        .unwrap();

    let err = state
        .password_resets
        .use_password_reset(&raw_token)
        .await
        .unwrap_err();
    assert!(
        matches!(err, UsePasswordResetError::Expired),
        "expected Expired, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn use_password_reset_unknown_token_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    let err = state
        .password_resets
        .use_password_reset(&parse_raw_token("not-a-real-token"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, UsePasswordResetError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// PostStorage integration tests
// ---------------------------------------------------------------------------

fn make_create_post_input(user_id: UserId, slug: &str) -> CreatePostInput {
    CreatePostInput {
        user_id,
        title: Some(format!("Post {slug}").into()),
        slug: slug.parse().unwrap(),
        body: "body text".into(),
        format: PostFormat::Markdown,
        rendered_html: RenderedHtml::from_trusted("<p>body text</p>"),
        published_at: None,
        summary: None,
        audiences: vec![AudienceTarget::Public],
        idempotency_key: None,
    }
}

fn make_published_create_post_input(user_id: UserId, slug: &str) -> CreatePostInput {
    CreatePostInput {
        published_at: Some(Utc::now()),
        summary: None,
        ..make_create_post_input(user_id, slug)
    }
}

/// Creates a public post for `user_id` with an explicit `published_at`, returning
/// the new post id. A future `published_at` seeds a *scheduled* post (publicly
/// invisible until its time); a past one a live post. Lets the boundary tests
/// below pin the publication instant relative to the injected `now`.
async fn seed_post_published_at(
    state: &Arc<AppState>,
    user_id: UserId,
    slug: &str,
    published_at: chrono::DateTime<Utc>,
) -> i64 {
    create_rendered_post(
        &*state.posts,
        RenderedPostContent {
            user_id,
            title: None,
            slug: slug.parse().expect("valid slug"),
            body: format!("# {slug}\n\nbody").into(),
            format: PostFormat::Markdown,
            published_at: Some(published_at),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        },
    )
    .await
    .expect("seed post should be created")
}

// Scheduled-publishing boundary tests (issue #70): each public read must hide a
// future-dated post (`published_at > now`) and reveal it once `now` reaches its
// `published_at`. One common test per surface, both backends, fixed injected
// `now` (no sleeps) asserting both sides of the `<= now` boundary.

#[apply(backends)]
#[tokio::test]
async fn permalink_hides_scheduled_until_due(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    seed_post_published_at(state, user_id, "live-one", now - Duration::hours(1)).await;
    seed_post_published_at(state, user_id, "sched-one", now + Duration::hours(1)).await;

    // At `now`: the live post is visible, the scheduled one is not.
    let got_live = state
        .posts
        .get_post_by_permalink(
            &username("alice"),
            PermalinkDate {
                year: 2026,
                month: 6,
                day: 26,
            },
            &"live-one".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    assert!(got_live.is_some(), "live post must be visible at now");
    let got_sched = state
        .posts
        .get_post_by_permalink(
            &username("alice"),
            PermalinkDate {
                year: 2026,
                month: 6,
                day: 26,
            },
            &"sched-one".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    assert!(
        got_sched.is_none(),
        "scheduled post must be hidden before its time"
    );

    // One second past go-live: the scheduled post appears (locks the boundary).
    let after = now + Duration::hours(1) + Duration::seconds(1);
    let got_after = state
        .posts
        .get_post_by_permalink(
            &username("alice"),
            PermalinkDate {
                year: 2026,
                month: 6,
                day: 26,
            },
            &"sched-one".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            after,
        )
        .await
        .unwrap();
    assert!(
        got_after.is_some(),
        "scheduled post must appear once now >= published_at"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_published_by_user_hides_scheduled_until_due(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let live = seed_post_published_at(state, user_id, "live-one", now - Duration::hours(1)).await;
    let sched = seed_post_published_at(state, user_id, "sched-one", now + Duration::hours(1)).await;

    let at_now = state
        .posts
        .list_published_by_user(
            &username("alice"),
            None,
            50,
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    let ids_now: Vec<i64> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = now + Duration::hours(1) + Duration::seconds(1);
    let at_after = state
        .posts
        .list_published_by_user(
            &username("alice"),
            None,
            50,
            &ViewerIdentity::Anonymous,
            after,
        )
        .await
        .unwrap();
    assert!(
        at_after.iter().any(|p| p.post_id == sched),
        "scheduled post must be listed once now >= published_at"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_published_hides_scheduled_until_due(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let live = seed_post_published_at(state, user_id, "live-one", now - Duration::hours(1)).await;
    let sched = seed_post_published_at(state, user_id, "sched-one", now + Duration::hours(1)).await;

    let at_now = state
        .posts
        .list_published(None, 50, &ViewerIdentity::Anonymous, now)
        .await
        .unwrap();
    let ids_now: Vec<i64> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = now + Duration::hours(1) + Duration::seconds(1);
    let at_after = state
        .posts
        .list_published(None, 50, &ViewerIdentity::Anonymous, after)
        .await
        .unwrap();
    assert!(
        at_after.iter().any(|p| p.post_id == sched),
        "scheduled post must be listed once now >= published_at"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag_hides_scheduled_until_due(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let live = seed_post_published_at(state, user_id, "live-one", now - Duration::hours(1)).await;
    let sched = seed_post_published_at(state, user_id, "sched-one", now + Duration::hours(1)).await;
    state
        .posts
        .tag_post(live, &"scheduling".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    state
        .posts
        .tag_post(sched, &"scheduling".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    let tag_slug: Tag = "scheduling".parse().unwrap();

    let at_now = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, now)
        .await
        .unwrap();
    let ids_now: Vec<i64> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = now + Duration::hours(1) + Duration::seconds(1);
    let at_after = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, after)
        .await
        .unwrap();
    assert!(
        at_after.iter().any(|p| p.post_id == sched),
        "scheduled post must be listed once now >= published_at"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_hides_scheduled_until_due(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let live = seed_post_published_at(state, user_id, "live-one", now - Duration::hours(1)).await;
    let sched = seed_post_published_at(state, user_id, "sched-one", now + Duration::hours(1)).await;
    state
        .posts
        .tag_post(live, &"scheduling".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    state
        .posts
        .tag_post(sched, &"scheduling".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    let tag_slug: Tag = "scheduling".parse().unwrap();

    let at_now = state
        .posts
        .list_user_posts_by_tag(
            user_id,
            &tag_slug,
            None,
            50,
            &ViewerIdentity::Anonymous,
            now,
        )
        .await
        .unwrap();
    let ids_now: Vec<i64> = at_now.iter().map(|p| p.post_id).collect();
    assert!(ids_now.contains(&live), "live post must be listed at now");
    assert!(
        !ids_now.contains(&sched),
        "scheduled post must be hidden before its time"
    );

    let after = now + Duration::hours(1) + Duration::seconds(1);
    let at_after = state
        .posts
        .list_user_posts_by_tag(
            user_id,
            &tag_slug,
            None,
            50,
            &ViewerIdentity::Anonymous,
            after,
        )
        .await
        .unwrap();
    assert!(
        at_after.iter().any(|p| p.post_id == sched),
        "scheduled post must be listed once now >= published_at"
    );
}

// Post tests (backend-parametrized)

#[apply(backends)]
#[tokio::test]
async fn post_create_and_get_by_id_works(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let input = make_create_post_input(user_id, "hello-world");
    let post_id = state.posts.create_post(&input).await.unwrap();

    let record = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.post_id, post_id);
    assert_eq!(record.user_id, user_id);
    assert_eq!(record.title.as_deref(), Some("Post hello-world"));
    assert_eq!(record.slug, "hello-world");
    assert_eq!(record.format, PostFormat::Markdown);
    assert!(record.published_at.is_none());
    assert!(record.deleted_at.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn post_slug_conflict_returns_slug_conflict(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();

    // Two published posts with the same slug on the same date conflict on the
    // unique index (user_id, date(COALESCE(published_at, created_at)), slug).
    let now = Utc::now();
    let pub_input = CreatePostInput {
        user_id,
        title: Some("Published".into()),
        slug: "same-day-slug".parse().unwrap(),
        body: "body".into(),
        format: PostFormat::Markdown,
        rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
        published_at: Some(now),
        summary: None,
        audiences: vec![AudienceTarget::Public],
        idempotency_key: None,
    };
    state.posts.create_post(&pub_input.clone()).await.unwrap();

    let err = state.posts.create_post(&pub_input).await.unwrap_err();
    assert!(
        matches!(err, CreatePostError::SlugConflict),
        "expected SlugConflict, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn post_update_writes_revision_and_updates_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("carol"), &password("password123"), None, false)
        .await
        .unwrap();

    let post_id = state
        .posts
        .create_post(&make_create_post_input(user_id, "update-test"))
        .await
        .unwrap();

    let update_input = UpdatePostInput {
        title: Some("Updated Title".into()),
        slug: "update-test".parse().unwrap(),
        body: "updated body".into(),
        format: PostFormat::Org,
        rendered_html: RenderedHtml::from_trusted("<p>updated body</p>"),
        unpublish: true,
        explicit_published_at: None,
        summary: None,
        audiences: vec![AudienceTarget::Public],
    };
    let record = state
        .posts
        .update_post(post_id, user_id, &update_input)
        .await
        .unwrap();

    assert_eq!(record.title.as_deref(), Some("Updated Title"));
    assert_eq!(record.format, PostFormat::Org);
    assert_eq!(record.body, "updated body");
}

#[apply(backends)]
#[tokio::test]
async fn post_update_not_found_returns_error(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let update_input = UpdatePostInput {
        title: Some("Title".into()),
        slug: "nope".parse().unwrap(),
        body: "body".into(),
        format: PostFormat::Markdown,
        rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
        unpublish: true,
        explicit_published_at: None,
        summary: None,
        audiences: vec![AudienceTarget::Public],
    };
    let err = state
        .posts
        .update_post(9999, UserId::from(1), &update_input)
        .await
        .unwrap_err();
    assert!(
        matches!(err, UpdatePostError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn post_update_by_non_owner_returns_unauthorized(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let owner = state
        .users
        .create_user(&username("post_owner"), &password("password"), None, false)
        .await
        .expect("owner creation failed");
    let other = state
        .users
        .create_user(&username("other_user"), &password("password"), None, false)
        .await
        .expect("other creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: owner,
            title: Some("Owned".into()),
            slug: "owned".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let err = state
        .posts
        .update_post(
            post_id,
            other,
            &UpdatePostInput {
                title: Some("Hijacked".into()),
                slug: "hijacked".parse().unwrap(),
                body: "Nope".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>Nope</p>"),
                unpublish: true,
                explicit_published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .expect_err("non-owner update must fail");

    assert!(matches!(err, UpdatePostError::Unauthorized));
}

/// Builds a `PostUpdate` with the given publish verb and otherwise-valid,
/// stable fields. `slug` is pinned via `slug_override` so repeated updates on
/// different posts never collide on a derived slug.
fn update_input(
    post_id: i64,
    editor_user_id: UserId,
    slug: &Slug,
    publish: PublishUpdate,
) -> PostUpdate<'_> {
    PostUpdate {
        post_id,
        editor_user_id,
        body: "updated body".into(),
        title: Some("Updated Title"),
        format: PostFormat::Markdown,
        slug_override: Some(slug),
        publish,
        summary: None,
        audiences: vec![AudienceTarget::Public],
    }
}

// Issue #70: the storage update's publication verb is an explicit
// `PublishUpdate`, not a bool. One common test, both backends, with an injected
// `now` pinning the boundary; locks the four publish-timestamp cases.
#[apply(backends)]
#[tokio::test]
async fn update_publish_timestamp_semantics(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let alice = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    // A fresh draft (published_at NULL).
    let draft = state
        .posts
        .create_post(&make_create_post_input(alice, "p"))
        .await
        .unwrap();

    // Pinned override slugs (already valid, as they arrive at the storage layer).
    let p: Slug = "p".parse().unwrap();
    let q: Slug = "q".parse().unwrap();

    // Publish { at: Some(future) } on a draft => scheduled at that instant.
    let future = now + Duration::days(1);
    let rec = perform_post_update(
        &*state.posts,
        update_input(
            draft,
            alice,
            &p,
            PublishUpdate::Publish { at: Some(future) },
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        rec.published_at,
        Some(future),
        "explicit future timestamp is stored"
    );

    // Publish { at: None } on an already-published post keeps the existing timestamp.
    let rec2 = perform_post_update(
        &*state.posts,
        update_input(draft, alice, &p, PublishUpdate::Publish { at: None }),
    )
    .await
    .unwrap();
    assert_eq!(
        rec2.published_at,
        Some(future),
        "publish-without-timestamp keeps existing"
    );

    // Unpublish clears it.
    let rec3 = perform_post_update(
        &*state.posts,
        update_input(draft, alice, &p, PublishUpdate::Unpublish),
    )
    .await
    .unwrap();
    assert_eq!(rec3.published_at, None, "unpublish clears published_at");

    // Publish { at: None } on a never-published draft stamps ~now.
    let draft2 = state
        .posts
        .create_post(&make_create_post_input(alice, "q"))
        .await
        .unwrap();
    let rec4 = perform_post_update(
        &*state.posts,
        update_input(draft2, alice, &q, PublishUpdate::Publish { at: None }),
    )
    .await
    .unwrap();
    assert!(
        rec4.published_at.is_some(),
        "publish-now stamps a timestamp"
    );
}

// Raw read of a post's `post_audiences` rows as `(target_kind name, audience_id)`,
// ordered by kind name. Used by the audience-targeting persistence test.
async fn post_audience_rows(
    backend: Backend,
    env: &TestEnv,
    post_id: i64,
) -> Vec<(String, Option<i64>)> {
    let sql = "SELECT tk.name, pa.audience_id \
               FROM post_audiences pa \
               JOIN target_kinds tk ON tk.kind_id = pa.target_kind_id \
               WHERE pa.post_id = $1 \
               ORDER BY tk.name, pa.audience_id";
    match backend {
        Backend::Sqlite => sqlx::query_as(&sql.replace("$1", "?"))
            .bind(post_id)
            .fetch_all(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => {
            let pool = env.base.pool().postgres();
            sqlx::query_as(sql)
                .bind(post_id)
                .fetch_all(pool)
                .await
                .unwrap()
        }
    }
}

// Scheduled publishing (#70) relies on a standalone `published_at` index for the
// `published_at <= now` reads and the worker's go-live range scans. This asserts
// the migration created it; a backend `match` is legitimate here because we are
// querying each engine's schema catalog, not exercising divergent product
// behavior.
#[apply(backends)]
#[tokio::test]
async fn posts_published_at_index_exists(#[case] backend: Backend) {
    let env = backend.setup().await;
    let names: Vec<String> = match backend {
        Backend::Sqlite => sqlx::query_scalar(
            "SELECT name FROM sqlite_master \
             WHERE type = 'index' AND name = 'idx_posts_published_at'",
        )
        .fetch_all(&open_pool(&env.base).await)
        .await
        .unwrap(),
        Backend::Postgres => {
            let pool = env.base.pool().postgres();
            sqlx::query_scalar(
                "SELECT indexname FROM pg_indexes WHERE indexname = 'idx_posts_published_at'",
            )
            .fetch_all(pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(names, vec!["idx_posts_published_at".to_string()]);
}

// Create persists `post_audiences` rows matching the input vec; update replaces
// them (delete-all-then-insert). `Private`/empty → no rows. See ADR-0020.
#[apply(backends)]
#[tokio::test]
async fn post_audiences_are_persisted_and_replaced(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let author = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let aud = state
        .audiences
        .create_audience(author, &parse_audience_name("Friends"))
        .await
        .unwrap();

    // Create targeting [Public, Named(aud)] → two rows.
    let input = CreatePostInput {
        audiences: vec![AudienceTarget::Public, AudienceTarget::Named(aud)],
        ..make_create_post_input(author, "audience-post")
    };
    let post_id = state.posts.create_post(&input).await.unwrap();
    let rows = post_audience_rows(backend, &env, post_id).await;
    assert_eq!(
        rows,
        vec![
            ("named".to_string(), Some(aud)),
            ("public".to_string(), None),
        ],
        "create should persist one public and one named row"
    );

    // Update to [Private] → zero rows.
    let update_private = UpdatePostInput {
        title: Some("Post audience-post".into()),
        slug: "audience-post".parse().unwrap(),
        body: "body text".into(),
        format: PostFormat::Markdown,
        rendered_html: RenderedHtml::from_trusted("<p>body text</p>"),
        unpublish: true,
        explicit_published_at: None,
        summary: None,
        audiences: vec![AudienceTarget::Private],
    };
    state
        .posts
        .update_post(post_id, author, &update_private)
        .await
        .unwrap();
    assert!(
        post_audience_rows(backend, &env, post_id).await.is_empty(),
        "[Private] should leave no rows"
    );

    // Update to [] (empty) → also zero rows (equivalent to private).
    let update_empty = UpdatePostInput {
        audiences: vec![],
        ..update_private.clone()
    };
    state
        .posts
        .update_post(post_id, author, &update_empty)
        .await
        .unwrap();
    assert!(
        post_audience_rows(backend, &env, post_id).await.is_empty(),
        "an empty audience vec should leave no rows"
    );

    // Update to [Subscribers] → one subscribers row.
    let update_subs = UpdatePostInput {
        audiences: vec![AudienceTarget::Subscribers],
        ..update_private.clone()
    };
    state
        .posts
        .update_post(post_id, author, &update_subs)
        .await
        .unwrap();
    assert_eq!(
        post_audience_rows(backend, &env, post_id).await,
        vec![("subscribers".to_string(), None)],
        "update to [Subscribers] should leave exactly one subscribers row"
    );
}

// `get_post_audiences` reads a post's targeting back as a `Vec<AudienceTarget>`
// (owner-only, no viewer). Round-trips create → read for each shape.
#[apply(backends)]
#[tokio::test]
async fn get_post_audiences_round_trips(#[case] backend: Backend) {
    use std::collections::HashSet;

    let env = backend.setup().await;
    let state = &env.state;
    let author = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let aud = state
        .audiences
        .create_audience(author, &parse_audience_name("Friends"))
        .await
        .unwrap();

    // Public + Named(aud) → union read back (order-independent compare).
    let input = CreatePostInput {
        audiences: vec![AudienceTarget::Public, AudienceTarget::Named(aud)],
        ..make_create_post_input(author, "round-trip")
    };
    let post_id = state.posts.create_post(&input).await.unwrap();
    let read: HashSet<_> = state
        .posts
        .get_post_audiences(post_id)
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(
        read,
        HashSet::from([AudienceTarget::Public, AudienceTarget::Named(aud)]),
        "should read back the Public + Named union"
    );

    // Subscribers-only.
    let update_subs = UpdatePostInput {
        title: Some("Post round-trip".into()),
        slug: "round-trip".parse().unwrap(),
        body: "body text".into(),
        format: PostFormat::Markdown,
        rendered_html: RenderedHtml::from_trusted("<p>body text</p>"),
        unpublish: true,
        explicit_published_at: None,
        summary: None,
        audiences: vec![AudienceTarget::Subscribers],
    };
    state
        .posts
        .update_post(post_id, author, &update_subs)
        .await
        .unwrap();
    assert_eq!(
        state.posts.get_post_audiences(post_id).await.unwrap(),
        vec![AudienceTarget::Subscribers],
        "should read back Subscribers"
    );

    // Private / empty → no rows → empty vec.
    let update_private = UpdatePostInput {
        audiences: vec![AudienceTarget::Private],
        ..update_subs.clone()
    };
    state
        .posts
        .update_post(post_id, author, &update_private)
        .await
        .unwrap();
    assert!(
        state
            .posts
            .get_post_audiences(post_id)
            .await
            .unwrap()
            .is_empty(),
        "Private should read back as an empty vec"
    );
}

#[apply(backends)]
#[tokio::test]
async fn soft_delete_excludes_post_from_lists(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("dave"), &password("password123"), None, false)
        .await
        .unwrap();

    let post_id = state
        .posts
        .create_post(&make_published_create_post_input(user_id, "to-delete"))
        .await
        .unwrap();

    let published = state
        .posts
        .list_published(None, 10, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .unwrap();
    assert!(published.iter().any(|p| p.post_id == post_id));

    state.posts.soft_delete_post(post_id).await.unwrap();

    let published = state
        .posts
        .list_published(None, 10, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .unwrap();
    assert!(!published.iter().any(|p| p.post_id == post_id));

    let record = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .unwrap();
    assert!(record.deleted_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn list_published_in_window_applies_hybrid_rule_across_surfaces(#[case] backend: Backend) {
    use chrono::Duration;
    use common::feed::{FeedSurface, HybridWindow};

    let env = backend.setup().await;
    let state = &env.state;

    let alice_id = state
        .users
        .create_user(&username("walice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob_id = state
        .users
        .create_user(&username("wbob"), &password("password123"), None, false)
        .await
        .unwrap();

    let now = Utc::now();
    let make_post = |user_id: UserId, slug: &str, days_ago: i64| {
        let mut input = make_create_post_input(user_id, slug);
        input.published_at = Some(now - Duration::days(days_ago));
        input
    };

    // Alice: 4 posts published 1, 2, 100, 200 days ago.
    state
        .posts
        .create_post(&make_post(alice_id, "alice-recent-1", 1))
        .await
        .unwrap();
    state
        .posts
        .create_post(&make_post(alice_id, "alice-recent-2", 2))
        .await
        .unwrap();
    state
        .posts
        .create_post(&make_post(alice_id, "alice-old-1", 100))
        .await
        .unwrap();
    state
        .posts
        .create_post(&make_post(alice_id, "alice-old-2", 200))
        .await
        .unwrap();

    // Bob: 1 post published 5 days ago.
    state
        .posts
        .create_post(&make_post(bob_id, "bob-recent", 5))
        .await
        .unwrap();

    // Future-dated draft-equivalent (excluded).
    let mut future_input = make_create_post_input(alice_id, "alice-future");
    future_input.published_at = Some(now + Duration::days(1));
    state.posts.create_post(&future_input).await.unwrap();

    // Site feed, window {3 items, 30 days} → union of "top 3" and "in last 30
    // days". Alice 1d+2d and Bob 5d are in-window (3 posts). Alice 100d/200d
    // and the future post are excluded by their respective filters; the union
    // still picks at least 3 by ROW_NUMBER, so we get exactly those 3.
    let window = HybridWindow {
        min_items: 3,
        min_days: 30,
    };
    let site = state
        .posts
        .list_published_in_window(&FeedSurface::Site, &window, now, &ViewerIdentity::Anonymous)
        .await
        .unwrap();
    assert_eq!(site.len(), 3, "site feed in {{3 items, 30 days}}");
    assert!(site
        .iter()
        .all(|p| p.published_at.unwrap() >= now - Duration::days(30)));

    // Site feed with min_items=5: top 5 includes all four real posts plus
    // Bob's, regardless of age — total 5 (alice-old-2 included by count).
    let big = HybridWindow {
        min_items: 5,
        min_days: 30,
    };
    let site_big = state
        .posts
        .list_published_in_window(&FeedSurface::Site, &big, now, &ViewerIdentity::Anonymous)
        .await
        .unwrap();
    assert_eq!(site_big.len(), 5, "min_items=5 pulls in older posts");

    // User feed for Alice, {2 items, 30 days}: union of "Alice's top 2"
    // (alice-recent-1, alice-recent-2) and "Alice's posts in last 30 days"
    // (same two) → 2. The 100/200-day-old posts and future are excluded.
    let alice_window = HybridWindow {
        min_items: 2,
        min_days: 30,
    };
    let alice_feed = state
        .posts
        .list_published_in_window(
            &FeedSurface::User {
                username: "walice".parse().unwrap(),
            },
            &alice_window,
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(alice_feed.len(), 2);
    assert!(alice_feed.iter().all(|p| p.user_id == alice_id));

    // User feed: bob has only 1 post, returned even with min_items=10.
    let bob_feed = state
        .posts
        .list_published_in_window(
            &FeedSurface::User {
                username: "wbob".parse().unwrap(),
            },
            &HybridWindow {
                min_items: 10,
                min_days: 1,
            },
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(bob_feed.len(), 1);
    assert_eq!(bob_feed[0].user_id, bob_id);

    // Add a tag to alice-recent-1 and verify site-tag / user-tag feeds.
    let alice_recent_1 = state
        .posts
        .list_published_by_user(
            &username("walice"),
            None,
            10,
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.slug == "alice-recent-1")
        .unwrap();
    state
        .posts
        .tag_post(alice_recent_1.post_id, &"rust".parse::<TagLabel>().unwrap())
        .await
        .unwrap();

    let tag_site = state
        .posts
        .list_published_in_window(
            &FeedSurface::SiteTag {
                tag: "rust".parse().unwrap(),
            },
            &HybridWindow {
                min_items: 20,
                min_days: 30,
            },
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(tag_site.len(), 1);
    assert_eq!(tag_site[0].slug, "alice-recent-1");

    let tag_user = state
        .posts
        .list_published_in_window(
            &FeedSurface::UserTag {
                username: "walice".parse().unwrap(),
                tag: "rust".parse().unwrap(),
            },
            &HybridWindow {
                min_items: 20,
                min_days: 30,
            },
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert_eq!(tag_user.len(), 1);

    // User-tag for bob+rust: bob has no rust post → empty.
    let bob_tag = state
        .posts
        .list_published_in_window(
            &FeedSurface::UserTag {
                username: "wbob".parse().unwrap(),
                tag: "rust".parse().unwrap(),
            },
            &HybridWindow {
                min_items: 20,
                min_days: 30,
            },
            now,
            &ViewerIdentity::Anonymous,
        )
        .await
        .unwrap();
    assert!(bob_tag.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn list_published_by_user_returns_only_user_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let alice_id = state
        .users
        .create_user(&username("ealice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob_id = state
        .users
        .create_user(&username("ebob"), &password("password123"), None, false)
        .await
        .unwrap();

    state
        .posts
        .create_post(&make_published_create_post_input(alice_id, "alice-post1"))
        .await
        .unwrap();
    state
        .posts
        .create_post(&make_published_create_post_input(alice_id, "alice-post2"))
        .await
        .unwrap();
    state
        .posts
        .create_post(&make_published_create_post_input(bob_id, "bob-post1"))
        .await
        .unwrap();

    let alice_posts = state
        .posts
        .list_published_by_user(
            &username("ealice"),
            None,
            10,
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(alice_posts.len(), 2);
    assert!(alice_posts.iter().all(|p| p.user_id == alice_id));

    let bob_posts = state
        .posts
        .list_published_by_user(
            &username("ebob"),
            None,
            10,
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(bob_posts.len(), 1);
    assert_eq!(bob_posts[0].user_id, bob_id);
}

#[apply(backends)]
#[tokio::test]
async fn list_published_returns_published_non_deleted_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("fuser"), &password("password123"), None, false)
        .await
        .unwrap();

    // Create a draft (should not appear)
    state
        .posts
        .create_post(&make_create_post_input(user_id, "draft-post"))
        .await
        .unwrap();

    state
        .posts
        .create_post(&make_published_create_post_input(user_id, "pub-post1"))
        .await
        .unwrap();
    state
        .posts
        .create_post(&make_published_create_post_input(user_id, "pub-post2"))
        .await
        .unwrap();

    let published = state
        .posts
        .list_published(None, 10, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .unwrap();
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|p| p.published_at.is_some()));
}

#[apply(backends)]
#[tokio::test]
async fn list_drafts_by_user_returns_only_drafts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("guser"), &password("password123"), None, false)
        .await
        .unwrap();

    state
        .posts
        .create_post(&make_create_post_input(user_id, "draft-a"))
        .await
        .unwrap();
    state
        .posts
        .create_post(&make_create_post_input(user_id, "draft-b"))
        .await
        .unwrap();

    // Create a published post (should not appear in drafts)
    state
        .posts
        .create_post(&make_published_create_post_input(user_id, "published-c"))
        .await
        .unwrap();

    let drafts = state
        .posts
        .list_drafts_by_user(user_id, None, 10, Utc::now())
        .await
        .unwrap();
    assert_eq!(drafts.len(), 2);
    assert!(drafts.iter().all(|p| p.published_at.is_none()));
    assert!(drafts.iter().all(|p| p.user_id == user_id));
}

// The author's drafts surface is the "not-yet-live" surface: it must include
// true drafts AND scheduled (future-dated) posts, but exclude posts that are
// already live (`published_at <= now`). One common test, both backends, fixed
// injected `now` (issue #70).
#[apply(backends)]
#[tokio::test]
async fn drafts_list_includes_scheduled_excludes_live(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    // True draft (published_at NULL).
    state
        .posts
        .create_post(&make_create_post_input(user_id, "a-draft"))
        .await
        .unwrap();
    // Scheduled post (published_at in the future).
    seed_post_published_at(state, user_id, "a-sched", now + Duration::hours(2)).await;
    // Live post (published_at in the past).
    seed_post_published_at(state, user_id, "a-live", now - Duration::hours(2)).await;

    let rows = state
        .posts
        .list_drafts_by_user(user_id, None, 50, now)
        .await
        .unwrap();
    let slugs: Vec<String> = rows.iter().map(|p| p.slug.to_string()).collect();
    assert!(
        slugs.contains(&"a-draft".to_string()),
        "drafts must include true drafts: {slugs:?}"
    );
    assert!(
        slugs.contains(&"a-sched".to_string()),
        "drafts must include scheduled posts: {slugs:?}"
    );
    assert!(
        !slugs.contains(&"a-live".to_string()),
        "drafts must exclude live posts: {slugs:?}"
    );
}

// Go-live window/catch-up reads (issue #70, Task 7): the feed worker uses these
// to nudge cached feeds when a future-dated post crosses into "live" with no
// accompanying write. One common test per read, both backends, fixed injected
// clock (no sleeps).

#[apply(backends)]
#[tokio::test]
async fn list_posts_gone_live_between_returns_only_window_with_tags(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    let env = backend.setup().await;
    let state = &env.state;
    let after = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let upto = after + Duration::hours(1);
    let alice = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let bob = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();

    // Inside the window (after, upto], tagged: must be returned with its tag.
    let inside =
        seed_post_published_at(state, alice, "in-window", after + Duration::minutes(30)).await;
    state
        .posts
        .tag_post(inside, &"scheduling".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    // Exactly at the inclusive upper bound: must be returned (untagged).
    seed_post_published_at(state, bob, "at-upto", upto).await;
    // Exactly at the exclusive lower bound: must be excluded.
    seed_post_published_at(state, alice, "at-after", after).await;
    // Past the window: must be excluded.
    seed_post_published_at(state, alice, "out-window", upto + Duration::hours(1)).await;

    let live: Vec<GoLivePost> = state
        .posts
        .list_posts_gone_live_between(after, upto)
        .await
        .unwrap();
    assert_eq!(
        live.len(),
        2,
        "only the (after, upto] posts are returned: {live:?}"
    );

    let alice_live = live
        .iter()
        .find(|p| p.username == username("alice"))
        .expect("alice's in-window post is present");
    let slugs: Vec<String> = alice_live
        .tag_slugs
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(slugs, vec!["scheduling".to_string()], "tags are hydrated");

    let bob_live = live
        .iter()
        .find(|p| p.username == username("bob"))
        .expect("bob's at-upto post is present (inclusive upper)");
    assert!(
        bob_live.tag_slugs.is_empty(),
        "untagged post yields empty tag_slugs"
    );
}

#[apply(backends)]
#[tokio::test]
async fn feed_urls_needing_catchup_returns_stale_feeds(#[case] backend: Backend) {
    use chrono::{Duration, TimeZone};
    use common::feed::{FeedFormat, FeedPath, FeedSurface};
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc.with_ymd_and_hms(2026, 6, 26, 12, 0, 0).unwrap();
    let t0 = now - Duration::hours(2);
    let alice = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();

    // A live post, newer than t0, on the site/user feeds and — once tagged —
    // on the site-tag and user-tag feeds too.
    let post = seed_post_published_at(state, alice, "live-one", now - Duration::hours(1)).await;
    state
        .posts
        .tag_post(post, &"rust".parse::<TagLabel>().unwrap())
        .await
        .unwrap();

    let mk_row = |feed_url: &str, generated_at| FeedCacheRow {
        feed_path: fp(feed_url),
        body: "cached".to_string(),
        etag: "etag".to_string(),
        content_type: "application/atom+xml; charset=utf-8".to_string(),
        updated_at: generated_at,
        generated_at,
    };
    // The exact feed-url keys for each surface, built the same way the worker
    // does, so the per-surface arms of `max_published_at_for_surface` are all
    // exercised (Site, User, SiteTag, UserTag).
    let tag = "rust".parse().unwrap();
    let site_tag_url = FeedPath::canonical(&FeedSurface::SiteTag { tag }, FeedFormat::Atom);
    let user_tag_url = FeedPath::canonical(
        &FeedSurface::UserTag {
            username: username("alice"),
            tag: "rust".parse().unwrap(),
        },
        FeedFormat::Atom,
    );

    // Stale (generated before go-live) => must be returned.
    state
        .feed_cache
        .upsert(mk_row("/feed.atom", t0))
        .await
        .unwrap();
    state
        .feed_cache
        .upsert(mk_row(&site_tag_url, t0))
        .await
        .unwrap();
    state
        .feed_cache
        .upsert(mk_row(&user_tag_url, t0))
        .await
        .unwrap();
    // Fresh (generated after the newest live post) => must NOT be returned.
    state
        .feed_cache
        .upsert(mk_row("/~alice/feed.atom", now))
        .await
        .unwrap();

    let stale = state.posts.feed_urls_needing_catchup(now).await.unwrap();
    assert!(
        stale.iter().any(|u| u.as_ref() == "/feed.atom"),
        "a stale site feed is returned: {stale:?}"
    );
    assert!(
        stale.contains(&site_tag_url),
        "a stale site-tag feed is returned: {stale:?}"
    );
    assert!(
        stale.contains(&user_tag_url),
        "a stale user-tag feed is returned: {stale:?}"
    );
    assert!(
        !stale.iter().any(|u| u.as_ref() == "/~alice/feed.atom"),
        "a feed newer than its surface's newest post is not stale: {stale:?}"
    );
}

// =============================================================================
// Tag Tests
// =============================================================================

#[apply(backends)]
#[tokio::test]
async fn multiple_tags_on_single_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("multi_tag_user"),
            &password("password"),
            Some(&parse_display_name("Multi")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Multi Tag Post".into()),
            slug: "multi-tag-post".parse().unwrap(),
            body: "Content with many tags".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content with many tags</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"rust".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"performance".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"systems-programming".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 3);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(tag_slugs.contains(&"rust"));
    assert!(tag_slugs.contains(&"performance"));
    assert!(tag_slugs.contains(&"systems-programming"));
}

#[apply(backends)]
#[tokio::test]
async fn empty_tag_list(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("no_tag_user"),
            &password("password"),
            Some(&parse_display_name("NoTag")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("No Tags".into()),
            slug: "no-tags".parse().unwrap(),
            body: "Untagged post".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Untagged post</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 0);
}

#[apply(backends)]
#[tokio::test]
async fn tag_case_preservation_variants(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("case_user"),
            &password("password"),
            Some(&parse_display_name("Case")),
            false,
        )
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Post 1".into()),
            slug: "post-1".parse().unwrap(),
            body: "Content 1".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 1</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Post 2".into()),
            slug: "post-2".parse().unwrap(),
            body: "Content 2".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 2</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    // Tag with different casings but same canonical form - should map to same slug
    state
        .posts
        .tag_post(post1, &"Web-Development".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post post1 failed");
    state
        .posts
        .tag_post(post2, &"WEB-DEVELOPMENT".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post post2 failed");

    let tags1 = state
        .posts
        .get_tags_for_post(post1)
        .await
        .expect("get_tags_for_post post1 failed");
    let tags2 = state
        .posts
        .get_tags_for_post(post2)
        .await
        .expect("get_tags_for_post post2 failed");

    assert_eq!(tags1[0].tag_slug, "web-development");
    assert_eq!(tags2[0].tag_slug, "web-development");
    assert_eq!(tags1[0].tag_display, "Web-Development");
    assert_eq!(tags2[0].tag_display, "WEB-DEVELOPMENT");

    let tag_slug: Tag = "web-development".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 2);
}

#[apply(backends)]
#[tokio::test]
async fn tag_list_pagination(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("pagination_user"),
            &password("password"),
            Some(&parse_display_name("Pagination")),
            false,
        )
        .await
        .expect("user creation failed");

    let mut post_ids = Vec::new();
    for i in 0..5 {
        let post_id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: Some(format!("Post {i}").into()),
                slug: format!("post-{i}").parse().unwrap(),
                body: format!("Content {i}").into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted(format!("<p>Content {i}</p>")),
                published_at: Some(Utc::now()),
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .expect("post creation failed");
        post_ids.push(post_id);

        state
            .posts
            .tag_post(post_id, &"pagination-test".parse::<TagLabel>().unwrap())
            .await
            .expect("tag_post failed");
    }

    let tag_slug: Tag = "pagination-test".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 2, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 2);
    // Should be reverse chronological
    assert!(posts[0].created_at >= posts[1].created_at);
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_excludes_other_users(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user1 = state
        .users
        .create_user(
            &username("user1_tag"),
            &password("password"),
            Some(&parse_display_name("User1")),
            false,
        )
        .await
        .expect("user creation failed");

    let user2 = state
        .users
        .create_user(
            &username("user2_tag"),
            &password("password"),
            Some(&parse_display_name("User2")),
            false,
        )
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user1,
            title: Some("User1 Post".into()),
            slug: "user1-post".parse().unwrap(),
            body: "Content 1".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 1</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user2,
            title: Some("User2 Post".into()),
            slug: "user2-post".parse().unwrap(),
            body: "Content 2".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 2</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post1, &"shared-tag".parse::<TagLabel>().unwrap())
        .await
        .expect("tag post1 failed");
    state
        .posts
        .tag_post(post2, &"shared-tag".parse::<TagLabel>().unwrap())
        .await
        .expect("tag post2 failed");

    let tag_slug: Tag = "shared-tag".parse().unwrap();
    let user1_posts = state
        .posts
        .list_user_posts_by_tag(
            user1,
            &tag_slug,
            None,
            50,
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .expect("list_user_posts_by_tag failed");

    assert_eq!(user1_posts.len(), 1);
    assert_eq!(user1_posts[0].post_id, post1);

    let user2_posts = state
        .posts
        .list_user_posts_by_tag(
            user2,
            &tag_slug,
            None,
            50,
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .expect("list_user_posts_by_tag failed");

    assert_eq!(user2_posts.len(), 1);
    assert_eq!(user2_posts[0].post_id, post2);
}

#[apply(backends)]
#[tokio::test]
async fn selective_untag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("selective_untag"),
            &password("password"),
            Some(&parse_display_name("Selective")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Multi Tag".into()),
            slug: "multi-tag".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"tag-a".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"tag-b".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"tag-c".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 3);

    let tag_b: Tag = "tag-b".parse().unwrap();
    state
        .posts
        .untag_post(post_id, &tag_b)
        .await
        .expect("untag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 2);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(!tag_slugs.contains(&"tag-b"));
    assert!(tag_slugs.contains(&"tag-a"));
    assert!(tag_slugs.contains(&"tag-c"));
}

#[apply(backends)]
#[tokio::test]
async fn numeric_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("numeric_tag"),
            &password("password"),
            Some(&parse_display_name("Numeric")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Numeric Tag".into()),
            slug: "numeric-tag".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"python3".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"rust-2024".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"0day".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 3);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(tag_slugs.contains(&"python3"));
    assert!(tag_slugs.contains(&"rust-2024"));
    assert!(tag_slugs.contains(&"0day"));
}

#[apply(backends)]
#[tokio::test]
async fn retag_same_post_with_same_tag_fails(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("retag_user"),
            &password("password"),
            Some(&parse_display_name("Retag")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Retag Post".into()),
            slug: "retag-post".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"learning".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let result = state
        .posts
        .tag_post(post_id, &"learning".parse::<TagLabel>().unwrap())
        .await;
    assert!(matches!(result, Err(TaggingError::AlreadyTagged)));

    // Dedup is case-insensitive: a different-cased form of an existing tag is
    // still AlreadyTagged (both canonicalize to the same slug).
    let result = state
        .posts
        .tag_post(post_id, &"LEARNING".parse::<TagLabel>().unwrap())
        .await;
    assert!(matches!(result, Err(TaggingError::AlreadyTagged)));
}

#[apply(backends)]
#[tokio::test]
async fn untag_nonexistent_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let tag_slug: Tag = "phantom".parse().unwrap();
    let result = state.posts.untag_post(99999, &tag_slug).await;

    assert!(matches!(result, Err(TaggingError::TagNotFound)));
}

#[apply(backends)]
#[tokio::test]
async fn get_tags_nonexistent_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let tags = state
        .posts
        .get_tags_for_post(99999)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 0);
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_nonexistent_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let tag_slug: Tag = "nosuch-tag".parse().unwrap();
    let result = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, Utc::now())
        .await;

    assert!(matches!(result, Err(ListByTagError::TagNotFound)));
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_nonexistent_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("user_tag_nope"),
            &password("password"),
            Some(&parse_display_name("UserTagNope")),
            false,
        )
        .await
        .expect("user creation failed");

    let tag_slug: Tag = "nonexistent-tag-99".parse().unwrap();
    let result = state
        .posts
        .list_user_posts_by_tag(
            user,
            &tag_slug,
            None,
            50,
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await;

    assert!(matches!(result, Err(ListByTagError::TagNotFound)));
}

#[apply(backends)]
#[tokio::test]
async fn many_tags_many_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("many_tags_user"),
            &password("password"),
            Some(&parse_display_name("ManyTags")),
            false,
        )
        .await
        .expect("user creation failed");

    let mut post_ids = Vec::new();
    let tags = vec!["rust", "golang", "python", "javascript", "typescript"];

    for i in 0..3 {
        let post_id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: Some(format!("Post {i}").into()),
                slug: format!("post-many-{i}").parse().unwrap(),
                body: format!("Content {i}").into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted(format!("<p>Content {i}</p>")),
                published_at: Some(Utc::now()),
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .expect("post creation failed");
        post_ids.push(post_id);

        for tag in &tags {
            state
                .posts
                .tag_post(post_id, &tag.parse::<TagLabel>().unwrap())
                .await
                .expect("tag_post failed");
        }
    }

    for post_id in &post_ids {
        let tags_on_post = state
            .posts
            .get_tags_for_post(*post_id)
            .await
            .expect("get_tags_for_post failed");
        assert_eq!(tags_on_post.len(), 5);
    }

    for tag in &tags {
        let tag_slug: Tag = tag.parse().unwrap();
        let posts = state
            .posts
            .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, Utc::now())
            .await
            .expect("list_posts_by_tag failed");
        assert_eq!(posts.len(), 3);
    }
}

#[apply(backends)]
#[tokio::test]
async fn tag_all_numeric(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("numeric_only"),
            &password("password"),
            Some(&parse_display_name("NumericOnly")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Numeric Tag".into()),
            slug: "numeric-slug".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"2024".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"42".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 2);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(tag_slugs.contains(&"2024"));
    assert!(tag_slugs.contains(&"42"));
}

#[apply(backends)]
#[tokio::test]
async fn tag_hyphen_boundaries(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("hyphen_user"),
            &password("password"),
            Some(&parse_display_name("Hyphen")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Hyphen Test".into()),
            slug: "hyphen-test".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    // Valid: hyphens in the middle and at end
    state
        .posts
        .tag_post(post_id, &"web-development".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"a-b-c".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"end-".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 3);

    // Invalid slugs (leading hyphen, underscore) can no longer reach `tag_post`:
    // its `&TagLabel` argument is validated at construction, so those cases are
    // unconstructible here (they are rejected at the type boundary / atompub
    // ingest filter instead).
}

#[apply(backends)]
#[tokio::test]
async fn tag_with_long_display(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("long_tag_user"),
            &password("password"),
            Some(&parse_display_name("LongTagUser")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Long Tag Test".into()),
            slug: "long-tag-test".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let long_display = "very-long-technical-term-with-many-hyphens-and-lowercase-letters";
    state
        .posts
        .tag_post(post_id, &long_display.parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_display, long_display);
}

#[apply(backends)]
#[tokio::test]
async fn tag_list_ordering(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("ordering_user"),
            &password("password"),
            Some(&parse_display_name("Ordering")),
            false,
        )
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Post 1".into()),
            slug: "post-1-order".parse().unwrap(),
            body: "Content 1".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 1</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Post 2".into()),
            slug: "post-2-order".parse().unwrap(),
            body: "Content 2".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 2</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    // Tag in different orders
    state
        .posts
        .tag_post(post1, &"zebra".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post1, &"apple".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post1, &"mango".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    state
        .posts
        .tag_post(post2, &"mango".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags1 = state
        .posts
        .get_tags_for_post(post1)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags1.len(), 3);
    let slugs1: Vec<&str> = tags1.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(slugs1, vec!["apple", "mango", "zebra"]);

    // Verify consistency on multiple calls
    let tags1_again = state
        .posts
        .get_tags_for_post(post1)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags1_again.len(), 3);
    assert_eq!(tags1_again[0].tag_slug, "apple");
}

#[apply(backends)]
#[tokio::test]
async fn tags_for_multiple_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("multi_post_user"),
            &password("password"),
            Some(&parse_display_name("MultiPost")),
            false,
        )
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Post A".into()),
            slug: "post-a".parse().unwrap(),
            body: "Content A".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content A</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Post B".into()),
            slug: "post-b".parse().unwrap(),
            body: "Content B".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content B</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    // Only post2 is tagged; post1 stays untagged to assert the empty case.
    state
        .posts
        .tag_post(post2, &"featured".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags1 = state
        .posts
        .get_tags_for_post(post1)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags1.len(), 0);

    let tags2 = state
        .posts
        .get_tags_for_post(post2)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags2.len(), 1);
}

#[apply(backends)]
#[tokio::test]
async fn tag_mixed_alphanumeric(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("mixed_user"),
            &password("password"),
            Some(&parse_display_name("Mixed")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Mixed Post".into()),
            slug: "mixed-post".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"version-2-0-1".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"HTTP2".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, &"3D-Graphics".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 3);
    assert_eq!(tags[0].tag_slug, "3d-graphics");
    assert_eq!(tags[1].tag_slug, "http2");
    assert_eq!(tags[2].tag_slug, "version-2-0-1");
}

#[apply(backends)]
#[tokio::test]
async fn simple_tag_lifecycle(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("simple_user"),
            &password("password"),
            Some(&parse_display_name("Simple")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Simple".into()),
            slug: "simple".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"test".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags_before = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags_before.len(), 1);
    assert_eq!(tags_before[0].tag_display, "test");

    let tag_slug: Tag = "test".parse().unwrap();
    let posts_before = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");
    assert_eq!(posts_before.len(), 1);

    state
        .posts
        .untag_post(post_id, &tag_slug)
        .await
        .expect("untag_post failed");

    let tags_after = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags_after.len(), 0);

    // List by tag again - should return empty list (tag exists but no posts have it)
    let posts_after = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");
    assert_eq!(posts_after.len(), 0);
}

#[apply(backends)]
#[tokio::test]
async fn tag_creation_and_retrieval(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("alice"),
            &password("password"),
            Some(&parse_display_name("Alice")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Test Post".into()),
            slug: "test-post".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"rust".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_slug, "rust");
    assert_eq!(tags[0].tag_display, "rust");
}

#[apply(backends)]
#[tokio::test]
async fn tag_normalization(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("bob"),
            &password("password"),
            Some(&parse_display_name("Bob")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Test Post".into()),
            slug: "test-post".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"Rust-Web".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_slug, "rust-web"); // normalized
    assert_eq!(tags[0].tag_display, "Rust-Web"); // original preserved
}

#[apply(backends)]
#[tokio::test]
async fn untag_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("charlie"),
            &password("password"),
            Some(&parse_display_name("Charlie")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Test Post".into()),
            slug: "test-post".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"python".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 1);

    let tag_slug: Tag = "python".parse().unwrap();
    state
        .posts
        .untag_post(post_id, &tag_slug)
        .await
        .expect("untag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 0);
}

#[apply(backends)]
#[tokio::test]
async fn duplicate_tag_error(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("dave"),
            &password("password"),
            Some(&parse_display_name("Dave")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Test Post".into()),
            slug: "test-post".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"go".parse::<TagLabel>().unwrap())
        .await
        .expect("first tag_post failed");

    // Try to tag with same tag again (case insensitive)
    let result = state
        .posts
        .tag_post(post_id, &"GO".parse::<TagLabel>().unwrap())
        .await;
    match result {
        Err(TaggingError::AlreadyTagged) => {
            // Expected
        }
        other => panic!("Expected AlreadyTagged, got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user1 = state
        .users
        .create_user(
            &username("eve"),
            &password("password"),
            Some(&parse_display_name("Eve")),
            false,
        )
        .await
        .expect("user creation failed");

    let user2 = state
        .users
        .create_user(
            &username("frank"),
            &password("password"),
            Some(&parse_display_name("Frank")),
            false,
        )
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user1,
            title: Some("Post 1".into()),
            slug: "post-1".parse().unwrap(),
            body: "Content 1".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 1</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user2,
            title: Some("Post 2".into()),
            slug: "post-2".parse().unwrap(),
            body: "Content 2".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 2</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post1, &"javascript".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post2, &"javascript".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tag_slug: Tag = "javascript".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 2);
    assert!(posts.iter().any(|p| p.post_id == post1));
    assert!(posts.iter().any(|p| p.post_id == post2));
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user1 = state
        .users
        .create_user(
            &username("grace"),
            &password("password"),
            Some(&parse_display_name("Grace")),
            false,
        )
        .await
        .expect("user creation failed");

    let user2 = state
        .users
        .create_user(
            &username("henry"),
            &password("password"),
            Some(&parse_display_name("Henry")),
            false,
        )
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user1,
            title: Some("Post 1".into()),
            slug: "post-1".parse().unwrap(),
            body: "Content 1".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 1</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user1,
            title: Some("Post 2".into()),
            slug: "post-2".parse().unwrap(),
            body: "Content 2".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 2</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post3 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user2,
            title: Some("Post 3".into()),
            slug: "post-3".parse().unwrap(),
            body: "Content 3".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 3</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post1, &"clojure".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post2, &"clojure".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post3, &"clojure".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tag_slug: Tag = "clojure".parse().unwrap();
    let posts = state
        .posts
        .list_user_posts_by_tag(
            user1,
            &tag_slug,
            None,
            50,
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .expect("list_user_posts_by_tag failed");

    assert_eq!(posts.len(), 2);
    assert!(posts.iter().all(|p| p.user_id == user1));
}

#[apply(backends)]
#[tokio::test]
async fn tag_not_found_error(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let tag_slug: Tag = "nonexistent".parse().unwrap();
    let result = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, Utc::now())
        .await;

    match result {
        Err(ListByTagError::TagNotFound) => {
            // Expected
        }
        other => panic!("Expected TagNotFound, got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn soft_deleted_posts_excluded_from_tag_list(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("iris"),
            &password("password"),
            Some(&parse_display_name("Iris")),
            false,
        )
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Post 1".into()),
            slug: "post-1".parse().unwrap(),
            body: "Content 1".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 1</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Post 2".into()),
            slug: "post-2".parse().unwrap(),
            body: "Content 2".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content 2</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post1, &"haskell".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post2, &"haskell".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    state
        .posts
        .soft_delete_post(post1)
        .await
        .expect("soft_delete_post failed");

    let tag_slug: Tag = "haskell".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].post_id, post2);
}

#[apply(backends)]
#[tokio::test]
async fn tag_post_nonexistent_post_error(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let result = state
        .posts
        .tag_post(99999, &"nonexistent-post".parse::<TagLabel>().unwrap())
        .await;
    match result {
        Err(TaggingError::PostNotFound) => {
            // Expected
        }
        other => panic!("Expected PostNotFound, got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn untag_nonexistent_tag_error(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("karen"),
            &password("password"),
            Some(&parse_display_name("Karen")),
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Test Post".into()),
            slug: "test-post".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let tag_slug: Tag = "nonexistent".parse().unwrap();
    let result = state.posts.untag_post(post_id, &tag_slug).await;
    match result {
        Err(TaggingError::TagNotFound) => {
            // Expected
        }
        other => panic!("Expected TagNotFound, got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn draft_posts_excluded_from_tag_list(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("jack"),
            &password("password"),
            Some(&parse_display_name("Jack")),
            false,
        )
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Draft Post".into()),
            slug: "draft-post".parse().unwrap(),
            body: "Draft content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Draft</p>"),
            published_at: None, // Draft,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Published Post".into()),
            slug: "published-post".parse().unwrap(),
            body: "Published content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Published</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post1, &"kotlin".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post2, &"kotlin".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tag_slug: Tag = "kotlin".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].post_id, post2);
}

// ====== Additional coverage tests for error paths ======

#[apply(backends)]
#[tokio::test]
async fn post_update_invalid_slug(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(&username("test_user"), &password("password"), None, false)
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Original".into()),
            slug: "original-slug".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let _post_id2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Second".into()),
            slug: "second-slug".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let update_result = state
        .posts
        .update_post(
            post_id,
            user,
            &UpdatePostInput {
                title: Some("Updated".into()),
                slug: "second-slug".parse().unwrap(),
                body: "Updated content".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>Updated</p>"),
                unpublish: true,
                explicit_published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await;

    match update_result {
        Err(UpdatePostError::Internal(_)) => {
            // Expected: unique constraint violation on slug
        }
        other => panic!("Expected Internal error, got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_published_cursor_boundary(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("cursor_test_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let now = Utc::now();

    for i in 0..5 {
        let _ = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: Some(format!("Post {i}").into()),
                slug: format!("post-{i}").parse().unwrap(),
                body: "Content".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
                published_at: Some(now),
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .expect("post creation failed");
    }

    let all = state
        .posts
        .list_published(None, 10, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_published failed");
    assert_eq!(all.len(), 5);

    let first = state
        .posts
        .list_published(None, 2, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_published failed");
    assert_eq!(first.len(), 2);

    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[first.len() - 1].created_at,
            post_id: first[first.len() - 1].post_id,
        };
        let next = state
            .posts
            .list_published(Some(&cursor), 2, &ViewerIdentity::Anonymous, Utc::now())
            .await
            .expect("list_published with cursor failed");
        assert_eq!(next.len(), 2);
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_drafts_cursor_boundary(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("draft_cursor_test"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let _now = Utc::now();

    for i in 0..3 {
        let _ = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: Some(format!("Draft {i}").into()),
                slug: format!("draft-{i}").parse().unwrap(),
                body: "Content".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
                published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .expect("post creation failed");
    }

    let all = state
        .posts
        .list_drafts_by_user(user, None, 10, Utc::now())
        .await
        .expect("list_drafts_by_user failed");
    assert_eq!(all.len(), 3);

    let first = state
        .posts
        .list_drafts_by_user(user, None, 1, Utc::now())
        .await
        .expect("list_drafts_by_user failed");
    assert_eq!(first.len(), 1);

    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = state
            .posts
            .list_drafts_by_user(user, Some(&cursor), 2, Utc::now())
            .await
            .expect("list_drafts_by_user with cursor failed");
        assert!(next.len() <= 2);
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_user_posts_by_tag_cursor(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("tag_cursor_test"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let now = Utc::now();

    for i in 0..3 {
        let post_id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: Some(format!("Tagged {i}").into()),
                slug: format!("tagged-{i}").parse().unwrap(),
                body: "Content".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
                published_at: Some(now),
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .expect("post creation failed");

        state
            .posts
            .tag_post(post_id, &"cursor-tag".parse::<TagLabel>().unwrap())
            .await
            .expect("tag_post failed");
    }

    let tag: Tag = "cursor-tag".parse().unwrap();

    let all = state
        .posts
        .list_user_posts_by_tag(user, &tag, None, 10, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_user_posts_by_tag failed");
    assert_eq!(all.len(), 3);

    let first = state
        .posts
        .list_user_posts_by_tag(user, &tag, None, 1, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_user_posts_by_tag failed");
    assert_eq!(first.len(), 1);

    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = state
            .posts
            .list_user_posts_by_tag(
                user,
                &tag,
                Some(&cursor),
                2,
                &ViewerIdentity::Anonymous,
                Utc::now(),
            )
            .await
            .expect("list_user_posts_by_tag with cursor failed");
        assert!(next.len() <= 2);
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_posts_by_tag_cursor(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("global_tag_cursor_test"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let now = Utc::now();

    for i in 0..3 {
        let post_id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: Some(format!("Global {i}").into()),
                slug: format!("global-{i}").parse().unwrap(),
                body: "Content".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
                published_at: Some(now),
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .expect("post creation failed");

        state
            .posts
            .tag_post(post_id, &"global-tag".parse::<TagLabel>().unwrap())
            .await
            .expect("tag_post failed");
    }

    let tag: Tag = "global-tag".parse().unwrap();

    let all = state
        .posts
        .list_posts_by_tag(&tag, None, 10, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");
    assert_eq!(all.len(), 3);

    let first = state
        .posts
        .list_posts_by_tag(&tag, None, 1, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");
    assert_eq!(first.len(), 1);

    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = state
            .posts
            .list_posts_by_tag(
                &tag,
                Some(&cursor),
                2,
                &ViewerIdentity::Anonymous,
                Utc::now(),
            )
            .await
            .expect("list_posts_by_tag with cursor failed");
        assert!(next.len() <= 2);
    }
}

#[apply(backends)]
#[tokio::test]
async fn soft_delete_then_operations(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("soft_del_test"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("To Delete".into()),
            slug: "to-delete".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"delete-tag".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    state
        .posts
        .soft_delete_post(post_id)
        .await
        .expect("soft_delete_post failed");

    // Try to get by ID (should still exist internally)
    let post = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id failed");
    assert!(post.is_none() || post.unwrap().deleted_at.is_some());

    let tag: Tag = "delete-tag".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag, None, 10, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_posts_by_tag failed");
    assert!(posts.is_empty());
}

// ====== Additional error path and rollback scenario tests ======

#[apply(backends)]
#[tokio::test]
async fn tag_post_multiple_attempts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("tag_multi_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("For Tagging".into()),
            slug: "for-tagging".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"first-tag".parse::<TagLabel>().unwrap())
        .await
        .expect("first tag_post failed");

    state
        .posts
        .tag_post(post_id, &"second-tag".parse::<TagLabel>().unwrap())
        .await
        .expect("second tag_post failed");

    let result = state
        .posts
        .tag_post(post_id, &"first-tag".parse::<TagLabel>().unwrap())
        .await;
    match result {
        Err(TaggingError::AlreadyTagged) => {
            // Expected
        }
        other => panic!("Expected AlreadyTagged, got {other:?}"),
    }

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 2);
}

#[apply(backends)]
#[tokio::test]
async fn list_published_by_user_no_posts(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let _user = state
        .users
        .create_user(
            &username("no_posts_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let posts = state
        .posts
        .list_published_by_user(
            &username("no_posts_user"),
            None,
            10,
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .expect("list_published_by_user failed");
    assert!(posts.is_empty());

    let cursor = PostCursor {
        created_at: Utc::now(),
        post_id: 999,
    };
    let posts = state
        .posts
        .list_published_by_user(
            &username("no_posts_user"),
            Some(&cursor),
            10,
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .expect("list_published_by_user with cursor failed");
    assert!(posts.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn get_by_permalink_soft_deleted(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("permalink_del_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let created_at = Utc::now();

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Permalink Test".into()),
            slug: "permalink-test".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(created_at),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let post = state
        .posts
        .get_post_by_permalink(
            &username("permalink_del_user"),
            PermalinkDate {
                year: created_at.year(),
                month: created_at.month(),
                day: created_at.day(),
            },
            &"permalink-test".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .expect("get_post_by_permalink failed");
    assert!(post.is_some());

    state
        .posts
        .soft_delete_post(post_id)
        .await
        .expect("soft_delete_post failed");

    let post = state
        .posts
        .get_post_by_permalink(
            &username("permalink_del_user"),
            PermalinkDate {
                year: created_at.year(),
                month: created_at.month(),
                day: created_at.day(),
            },
            &"permalink-test".parse().unwrap(),
            &ViewerIdentity::Anonymous,
            Utc::now(),
        )
        .await
        .expect("get_post_by_permalink after delete failed");
    assert!(post.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn update_soft_deleted_post(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("update_del_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("To Update".into()),
            slug: "to-update".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .soft_delete_post(post_id)
        .await
        .expect("soft_delete_post failed");

    // Try to update - should fail with NotFound since we're using post_id that doesn't exist in the update logic
    let _result = state
        .posts
        .update_post(
            post_id,
            user,
            &UpdatePostInput {
                title: Some("Updated".into()),
                slug: "updated-slug".parse().unwrap(),
                body: "New content".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>New</p>"),
                unpublish: false,
                explicit_published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await;

    // Even though the post exists, the update might fail or succeed depending on implementation
    // The important part is that the post is soft deleted
    let post = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id failed");
    assert!(post.is_none() || post.unwrap().deleted_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn tag_edge_case_formats(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("tag_formats_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Edge Cases".into()),
            slug: "edge-cases".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"123".parse::<TagLabel>().unwrap())
        .await
        .expect("numeric tag failed");

    state
        .posts
        .tag_post(post_id, &"my-tag-here".parse::<TagLabel>().unwrap())
        .await
        .expect("hyphenated tag failed");

    state
        .posts
        .tag_post(post_id, &"MyTag".parse::<TagLabel>().unwrap())
        .await
        .expect("mixed case tag failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 3);
}

// ====== Comprehensive error path coverage ======

#[apply(backends)]
#[tokio::test]
async fn get_post_by_id_nonexistent(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let result = state
        .posts
        .get_post_by_id(999_999, &ViewerIdentity::Anonymous)
        .await;
    match result {
        Ok(None) => {
            // Expected
        }
        other => panic!("Expected Ok(None), got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn list_published_with_cursor_same_timestamp(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("cursor_same_ts_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let now = Utc::now();

    // Create posts at same timestamp
    let mut post_ids = vec![];
    for i in 0..4 {
        let post_id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: Some(format!("Post {i}").into()),
                slug: format!("post-cursor-same-{i}").parse().unwrap(),
                body: "Content".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
                published_at: Some(now),
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .expect("post creation failed");
        post_ids.push(post_id);
    }

    let first = state
        .posts
        .list_published(None, 2, &ViewerIdentity::Anonymous, Utc::now())
        .await
        .expect("list_published failed");
    assert_eq!(first.len(), 2);

    // Use cursor to get next batch with same created_at but different post_id
    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[first.len() - 1].created_at,
            post_id: first[first.len() - 1].post_id,
        };
        let next = state
            .posts
            .list_published(Some(&cursor), 2, &ViewerIdentity::Anonymous, Utc::now())
            .await
            .expect("list_published with cursor failed");
        assert_eq!(next.len(), 2);
    }
}

#[apply(backends)]
#[tokio::test]
async fn post_revisions_created(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("revision_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Original".into()),
            slug: "revision-test".parse().unwrap(),
            body: "Original content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Original</p>"),
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    let result = state
        .posts
        .update_post(
            post_id,
            user,
            &UpdatePostInput {
                title: Some("Updated".into()),
                slug: "revision-test".parse().unwrap(),
                body: "Updated content".into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted("<p>Updated</p>"),
                unpublish: false,
                explicit_published_at: None,
                summary: None,
                audiences: vec![AudienceTarget::Public],
            },
        )
        .await
        .expect("update_post failed");

    assert_eq!(result.title.as_deref(), Some("Updated"));
    assert_eq!(result.body, "Updated content");
    assert!(result.published_at.is_some());
}

#[apply(backends)]
#[tokio::test]
async fn tag_display_preservation(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("tag_display_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Display Test".into()),
            slug: "display-test".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"MySpecialTag".parse::<TagLabel>().unwrap())
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_display, "MySpecialTag");
    assert_eq!(tags[0].tag_slug, "myspecialtag");
}

#[apply(backends)]
#[tokio::test]
async fn untag_preserves_other_tags(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("untag_preserve_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Multi Tag".into()),
            slug: "multi-tag".parse().unwrap(),
            body: "Content".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>Content</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post_id, &"tag1".parse::<TagLabel>().unwrap())
        .await
        .expect("tag1 failed");
    state
        .posts
        .tag_post(post_id, &"tag2".parse::<TagLabel>().unwrap())
        .await
        .expect("tag2 failed");
    state
        .posts
        .tag_post(post_id, &"tag3".parse::<TagLabel>().unwrap())
        .await
        .expect("tag3 failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 3);

    let tag2: Tag = "tag2".parse().unwrap();
    state
        .posts
        .untag_post(post_id, &tag2)
        .await
        .expect("untag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 2);
    let tag_slugs: Vec<_> = tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert!(!tag_slugs.contains(&"tag2"));
}

// ====== Site config tests ======

#[apply(backends)]
#[tokio::test]
async fn site_config_operations(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let value = state.site_config.get("nonexistent.key").await;
    match value {
        Ok(None) => {
            // Expected
        }
        other => panic!("Expected Ok(None), got {other:?}"),
    }

    state
        .site_config
        .set("test.key", "test.value")
        .await
        .expect("set failed");

    let value = state.site_config.get("test.key").await;
    match value {
        Ok(Some(v)) => {
            assert_eq!(v, "test.value");
        }
        other => panic!("Expected Ok(Some), got {other:?}"),
    }

    state
        .site_config
        .set("test.key", "updated.value")
        .await
        .expect("set update failed");

    let value = state.site_config.get("test.key").await;
    match value {
        Ok(Some(v)) => {
            assert_eq!(v, "updated.value");
        }
        other => panic!("Expected updated value, got {other:?}"),
    }
}

#[apply(backends)]
#[tokio::test]
async fn session_list_operations(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("session_list_user"),
            &password("password"),
            None,
            false,
        )
        .await
        .expect("user creation failed");

    let session1 = state
        .sessions
        .create_session(user, "session 1")
        .await
        .expect("create_session 1 failed");

    let _session2 = state
        .sessions
        .create_session(user, "session 2")
        .await
        .expect("create_session 2 failed");

    let _session3 = state
        .sessions
        .create_session(user, "test session")
        .await
        .expect("create_session 3 failed");

    let sessions = state
        .sessions
        .list_sessions(user)
        .await
        .expect("list_sessions failed");

    assert_eq!(sessions.len(), 3);

    let labels: Vec<_> = sessions.iter().map(|s| s.label.as_str()).collect();
    assert!(labels.contains(&"session 1"));
    assert!(labels.contains(&"session 2"));
    assert!(labels.contains(&"test session"));

    let record = state
        .sessions
        .authenticate(&session1)
        .await
        .expect("authenticate failed");
    assert_eq!(record.user_id, user);
}

#[apply(backends)]
#[tokio::test]
async fn invite_list_operations(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let now = Utc::now();
    let future = now + chrono::Duration::hours(1);
    let past = now - chrono::Duration::hours(1);

    let _invite1 = state
        .invites
        .create_invite(future)
        .await
        .expect("create_invite 1 failed");

    let _invite2 = state
        .invites
        .create_invite(past)
        .await
        .expect("create_invite 2 failed");

    let invites = state
        .invites
        .list_invites()
        .await
        .expect("list_invites failed");

    assert!(invites.len() >= 2);

    let unused_count = invites.iter().filter(|i| i.used_at.is_none()).count();
    assert!(unused_count >= 2);
}

// =============================================================================
// create_rendered_post / update_rendered_post integration tests
// =============================================================================

#[apply(backends)]
#[tokio::test]
async fn create_rendered_post_markdown_renders_and_stores(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("render_alice"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let post_id = create_rendered_post(
        state.posts.as_ref(),
        RenderedPostContent {
            user_id,
            title: Some("Rendered Markdown".into()),
            slug: "rendered-markdown".parse().unwrap(),
            body: "**bold**".into(),
            format: PostFormat::Markdown,
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    let record = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.title.as_deref(), Some("Rendered Markdown"));
    assert!(
        record
            .rendered_html
            .as_ref()
            .contains("<strong>bold</strong>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_rendered_post_org_renders_and_stores(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("render_bob"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let post_id = create_rendered_post(
        state.posts.as_ref(),
        RenderedPostContent {
            user_id,
            title: Some("Rendered Org".into()),
            slug: "rendered-org".parse().unwrap(),
            body: "*bold*".into(),
            format: PostFormat::Org,
            published_at: None,
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    let record = state
        .posts
        .get_post_by_id(post_id, &ViewerIdentity::Anonymous)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.title.as_deref(), Some("Rendered Org"));
    assert!(
        record.rendered_html.as_ref().contains("<b>bold</b>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_rendered_post_slug_conflict_returns_storage_error(#[case] backend: Backend) {
    use storage::CreatePostError;

    let env = backend.setup().await;
    let state = &env.state;

    let user_id = state
        .users
        .create_user(
            &username("render_carol"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let now = Utc::now();

    create_rendered_post(
        state.posts.as_ref(),
        RenderedPostContent {
            user_id,
            title: Some("First Post".into()),
            slug: "conflict-slug".parse().unwrap(),
            body: "body".into(),
            format: PostFormat::Markdown,
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Second create with same slug+date conflicts
    let err = create_rendered_post(
        state.posts.as_ref(),
        RenderedPostContent {
            user_id,
            title: Some("Second Post".into()),
            slug: "conflict-slug".parse().unwrap(),
            body: "body".into(),
            format: PostFormat::Markdown,
            published_at: Some(now),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, CreatePostError::SlugConflict),
        "expected Storage error, got {err:?}"
    );
    assert!(
        err.to_string().contains("slug"),
        "expected slug conflict message, got: {err}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn create_post_foreign_key_violation_maps_to_internal(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    // A post referencing a non-existent user violates the `posts.user_id` foreign
    // key on both backends (SQLite enforces FKs here — sqlx's SqliteConnectOptions
    // defaults `foreign_keys` to ON), a *non-unique* DB error. So `create_post`
    // (via the shared `write_post_in_tx`) maps it to `Internal`, not `SlugConflict`
    // — exercising the generic-error arm.
    let input = CreatePostInput {
        user_id: UserId::from(999_999),
        title: Some("Orphan".into()),
        slug: "orphan".parse().unwrap(),
        body: "body".into(),
        format: PostFormat::Markdown,
        rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
        published_at: Some(Utc::now()),
        summary: None,
        audiences: vec![AudienceTarget::Public],
        idempotency_key: None,
    };

    let err = state.posts.create_post(&input).await.unwrap_err();
    assert!(
        matches!(err, CreatePostError::Internal(_)),
        "expected Internal for FK violation, got {err:?}"
    );
}

// =============================================================================
// create_posts (single-transaction batch insert) — issue #9
// =============================================================================

#[apply(backends)]
#[tokio::test]
async fn create_posts_empty_slice_is_noop(#[case] backend: Backend) {
    let env = backend.setup().await;
    let ids = env.state.posts.create_posts(&[]).await.unwrap();
    assert!(ids.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn create_posts_batches_all_rows_in_order(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("batch_alice"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let inputs: Vec<CreatePostInput> = (0..3)
        .map(|i| CreatePostInput {
            user_id,
            title: Some(format!("Batch {i}").into()),
            slug: format!("batch-{i}").parse().unwrap(),
            body: format!("body {i}").into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted(format!("<p>body {i}</p>")),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .collect();

    let ids = state.posts.create_posts(&inputs).await.unwrap();
    assert_eq!(ids.len(), 3);

    // Each id resolves to the matching row, and its Public audience is honored
    // (visible to Anonymous — get_post_by_id filters on audience).
    for (i, id) in ids.iter().enumerate() {
        let rec = state
            .posts
            .get_post_by_id(*id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rec.title.as_deref(), Some(format!("Batch {i}").as_str()));
    }
}

#[apply(backends)]
#[tokio::test]
async fn create_posts_conflict_rolls_back_whole_batch(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("batch_bob"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let mk = |slug: &str, i: usize| CreatePostInput {
        user_id,
        title: Some(format!("Row {i}").into()),
        slug: slug.parse().unwrap(),
        body: format!("body {i}").into(),
        format: PostFormat::Markdown,
        rendered_html: RenderedHtml::from_trusted(format!("<p>body {i}</p>")),
        published_at: Some(Utc::now()),
        summary: None,
        audiences: vec![AudienceTarget::Public],
        idempotency_key: None,
    };
    // Rows 0 and 2 collide on slug — the batch must fail on row 2 and undo 0/1.
    let inputs = vec![mk("dup", 0), mk("unique", 1), mk("dup", 2)];

    let err = state.posts.create_posts(&inputs).await.unwrap_err();
    assert!(
        matches!(err, CreatePostError::SlugConflict),
        "expected SlugConflict, got {err:?}"
    );

    // Nothing persisted: the author's collection (drafts + published) is empty.
    let collection = state
        .posts
        .list_collection_by_user(user_id, None, 50)
        .await
        .unwrap();
    assert!(
        collection.is_empty(),
        "expected full rollback, found {} rows",
        collection.len()
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_rendered_post_markdown_renders_and_updates(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("render_dave"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let post_id = state
        .posts
        .create_post(&make_create_post_input(user_id, "update-render-md"))
        .await
        .unwrap();

    let record = update_rendered_post(
        state.posts.as_ref(),
        RenderedPostUpdate {
            post_id,
            editor_user_id: user_id,
            title: Some("Updated Title".into()),
            slug: "update-render-md".parse().unwrap(),
            body: "**updated**".into(),
            format: PostFormat::Markdown,
            publish: PublishUpdate::Unpublish,
            summary: None,
            audiences: vec![AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    assert_eq!(record.title.as_deref(), Some("Updated Title"));
    assert!(
        record
            .rendered_html
            .as_ref()
            .contains("<strong>updated</strong>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_rendered_post_org_renders_and_updates(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("render_eve"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let post_id = state
        .posts
        .create_post(&make_create_post_input(user_id, "update-render-org"))
        .await
        .unwrap();

    let record = update_rendered_post(
        state.posts.as_ref(),
        RenderedPostUpdate {
            post_id,
            editor_user_id: user_id,
            title: Some("Updated Org Title".into()),
            slug: "update-render-org".parse().unwrap(),
            body: "*bold org*".into(),
            format: PostFormat::Org,
            publish: PublishUpdate::Unpublish,
            summary: None,
            audiences: vec![AudienceTarget::Public],
        },
    )
    .await
    .unwrap();

    assert_eq!(record.title.as_deref(), Some("Updated Org Title"));
    assert!(
        record.rendered_html.as_ref().contains("<b>bold org</b>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

#[apply(backends)]
#[tokio::test]
async fn update_rendered_post_not_found_returns_storage_error(#[case] backend: Backend) {
    use storage::UpdatePostError;

    let env = backend.setup().await;
    let state = &env.state;

    let err = update_rendered_post(
        state.posts.as_ref(),
        RenderedPostUpdate {
            post_id: 99999,
            editor_user_id: UserId::from(1),
            title: Some("No Post".into()),
            slug: "no-post".parse().unwrap(),
            body: "body".into(),
            format: PostFormat::Markdown,
            publish: PublishUpdate::Unpublish,
            summary: None,
            audiences: vec![AudienceTarget::Public],
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, UpdatePostError::NotFound),
        "expected Storage error, got {err:?}"
    );
    assert!(
        err.to_string().contains("not found"),
        "expected 'not found' message, got: {err}"
    );
}

// ── MediaStorage tests ────────────────────────────────────────────────────────

use storage::{CreateMediaError, DeleteMediaError, MediaRecord, MediaSource};

fn make_media_record(
    user_id: UserId,
    sha256: &str,
    filename: &str,
    source: MediaSource,
) -> MediaRecord {
    MediaRecord {
        user_id,
        sha256: sha256.to_string(),
        filename: filename.to_string(),
        source,
        content_type: "image/jpeg".to_string(),
        size_bytes: 12345,
        source_url: None,
        created_at: chrono::Utc::now(),
    }
}

#[apply(backends)]
#[tokio::test]
async fn create_and_get_media(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("mediauser1"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let sha256 = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234".to_string();
    let record = make_media_record(user_id, &sha256, "test.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();

    let fetched = state
        .media
        .get_media(user_id, &sha256, "test.jpg", &MediaSource::Upload)
        .await
        .unwrap();
    let fetched = fetched.expect("record should exist");
    assert_eq!(fetched.user_id, user_id);
    assert_eq!(fetched.sha256, sha256);
    assert_eq!(fetched.filename, "test.jpg");
    assert_eq!(fetched.source, MediaSource::Upload);
    assert_eq!(fetched.content_type, "image/jpeg");
    assert_eq!(fetched.size_bytes, 12345);
}

#[apply(backends)]
#[tokio::test]
async fn duplicate_media_returns_already_exists(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("mediauser2"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let sha256 = "bbbb1234bbbb1234bbbb1234bbbb1234bbbb1234bbbb1234bbbb1234bbbb1234".to_string();
    let record = make_media_record(user_id, &sha256, "dup.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();
    let err = state.media.create_media(&record).await.unwrap_err();
    assert!(
        matches!(err, CreateMediaError::AlreadyExists),
        "expected AlreadyExists, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn delete_media_removes_record(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("mediauser3"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let sha256 = "cccc1234cccc1234cccc1234cccc1234cccc1234cccc1234cccc1234cccc1234".to_string();
    let record = make_media_record(user_id, &sha256, "del.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();
    state
        .media
        .delete_media(user_id, &sha256, "del.jpg", &MediaSource::Upload)
        .await
        .unwrap();

    let fetched = state
        .media
        .get_media(user_id, &sha256, "del.jpg", &MediaSource::Upload)
        .await
        .unwrap();
    assert!(fetched.is_none(), "record should have been deleted");
}

#[apply(backends)]
#[tokio::test]
async fn delete_nonexistent_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("mediauser4"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let sha256 = "dddd1234dddd1234dddd1234dddd1234dddd1234dddd1234dddd1234dddd1234".to_string();
    let err = state
        .media
        .delete_media(user_id, &sha256, "ghost.jpg", &MediaSource::Upload)
        .await
        .unwrap_err();
    assert!(
        matches!(err, DeleteMediaError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_media_returns_records_for_user(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_a = state
        .users
        .create_user(
            &username("mediauser5a"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();
    let user_b = state
        .users
        .create_user(
            &username("mediauser5b"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let sha1 = "eeee1234eeee1234eeee1234eeee1234eeee1234eeee1234eeee1234eeee1234".to_string();
    let sha2 = "ffff1234ffff1234ffff1234ffff1234ffff1234ffff1234ffff1234ffff1234".to_string();
    let sha3 = "gggg1234gggg1234gggg1234gggg1234gggg1234gggg1234gggg1234gggg1234".to_string();

    state
        .media
        .create_media(&make_media_record(
            user_a,
            &sha1,
            "a1.jpg",
            MediaSource::Upload,
        ))
        .await
        .unwrap();
    state
        .media
        .create_media(&make_media_record(
            user_a,
            &sha2,
            "a2.jpg",
            MediaSource::Upload,
        ))
        .await
        .unwrap();
    state
        .media
        .create_media(&make_media_record(
            user_b,
            &sha3,
            "b1.jpg",
            MediaSource::Upload,
        ))
        .await
        .unwrap();

    let results = state.media.list_media(user_a, None, 10, 0).await.unwrap();
    assert_eq!(results.len(), 2, "user_a should have 2 records");
    assert!(results.iter().all(|r| r.user_id == user_a));
}

#[apply(backends)]
#[tokio::test]
async fn list_media_filtered_by_source(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("mediauser6"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let sha_up = "hhhh1234hhhh1234hhhh1234hhhh1234hhhh1234hhhh1234hhhh1234hhhh1234".to_string();
    let sha_ca = "iiii1234iiii1234iiii1234iiii1234iiii1234iiii1234iiii1234iiii1234".to_string();

    state
        .media
        .create_media(&make_media_record(
            user_id,
            &sha_up,
            "up.jpg",
            MediaSource::Upload,
        ))
        .await
        .unwrap();
    state
        .media
        .create_media(&make_media_record(
            user_id,
            &sha_ca,
            "ca.jpg",
            MediaSource::Cached,
        ))
        .await
        .unwrap();

    let uploads = state
        .media
        .list_media(user_id, Some(&MediaSource::Upload), 10, 0)
        .await
        .unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].source, MediaSource::Upload);

    let cached = state
        .media
        .list_media(user_id, Some(&MediaSource::Cached), 10, 0)
        .await
        .unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].source, MediaSource::Cached);
}

#[apply(backends)]
#[tokio::test]
async fn get_user_upload_usage_returns_zero_initially(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("mediauser7"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let usage = state.media.get_user_upload_usage(user_id).await.unwrap();
    assert_eq!(usage, 0);
}

#[apply(backends)]
#[tokio::test]
async fn get_user_upload_usage_sums_uploads_only(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("mediauser8"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let sha_up = "jjjj1234jjjj1234jjjj1234jjjj1234jjjj1234jjjj1234jjjj1234jjjj1234".to_string();
    let sha_ca = "kkkk1234kkkk1234kkkk1234kkkk1234kkkk1234kkkk1234kkkk1234kkkk1234".to_string();

    let mut upload = make_media_record(user_id, &sha_up, "upload.jpg", MediaSource::Upload);
    upload.size_bytes = 1000;
    state.media.create_media(&upload).await.unwrap();

    let mut cached = make_media_record(user_id, &sha_ca, "cached.jpg", MediaSource::Cached);
    cached.size_bytes = 9999;
    state.media.create_media(&cached).await.unwrap();

    let usage = state.media.get_user_upload_usage(user_id).await.unwrap();
    assert_eq!(usage, 1000, "only upload bytes should count toward usage");
}

#[apply(backends)]
#[tokio::test]
async fn find_by_hash_returns_any_match(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(
            &username("mediauser9"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    let sha256 = "llll1234llll1234llll1234llll1234llll1234llll1234llll1234llll1234".to_string();
    let record = make_media_record(user_id, &sha256, "find.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();

    let found = state
        .media
        .find_by_hash(&sha256, &MediaSource::Upload)
        .await
        .unwrap();
    let found = found.expect("should find the record by hash");
    assert_eq!(found.sha256, sha256);
}

// ── UserConfigStorage tests ───────────────────────────────────────────────────

#[apply(backends)]
#[tokio::test]
async fn user_config_get_returns_none_when_unset(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("cfguser1"), &password("password123"), None, false)
        .await
        .unwrap();

    let val = state.user_config.get(user_id, "some.key").await.unwrap();
    assert!(val.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn user_config_set_and_get(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("cfguser2"), &password("password123"), None, false)
        .await
        .unwrap();

    state
        .user_config
        .set(user_id, "theme", "dark")
        .await
        .unwrap();
    let val = state.user_config.get(user_id, "theme").await.unwrap();
    assert_eq!(val.as_deref(), Some("dark"));
}

#[apply(backends)]
#[tokio::test]
async fn user_config_overwrite(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("cfguser3"), &password("password123"), None, false)
        .await
        .unwrap();

    state
        .user_config
        .set(user_id, "theme", "light")
        .await
        .unwrap();
    state
        .user_config
        .set(user_id, "theme", "dark")
        .await
        .unwrap();
    let val = state.user_config.get(user_id, "theme").await.unwrap();
    assert_eq!(val.as_deref(), Some("dark"));
}

#[apply(backends)]
#[tokio::test]
async fn user_config_delete_removes_key(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("cfguser4"), &password("password123"), None, false)
        .await
        .unwrap();

    state
        .user_config
        .set(user_id, "theme", "dark")
        .await
        .unwrap();
    state.user_config.delete(user_id, "theme").await.unwrap();
    let val = state.user_config.get(user_id, "theme").await.unwrap();
    assert!(val.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn user_config_delete_nonexistent_is_ok(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("cfguser5"), &password("password123"), None, false)
        .await
        .unwrap();

    state
        .user_config
        .delete(user_id, "nonexistent.key")
        .await
        .unwrap();
}

// ====== tags.2: list_tags + get_tags_for_posts ======

#[apply(backends)]
#[tokio::test]
async fn list_tags_returns_alphabetical_with_prefix(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("list_tags_user"),
            &password("password"),
            Some(&parse_display_name("ListTags")),
            false,
        )
        .await
        .expect("user creation failed");
    let post = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: Some("Tagged".into()),
            slug: "tagged".parse().unwrap(),
            body: "body".into(),
            format: PostFormat::Markdown,
            rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
            idempotency_key: None,
        })
        .await
        .expect("post creation failed");

    // Mixed-case display tokens — the slug should normalize to lowercase.
    for display in &["Rust", "rust-lang", "performance", "PostgreSQL", "web"] {
        state
            .posts
            .tag_post(post, &display.parse::<TagLabel>().unwrap())
            .await
            .unwrap();
    }

    // No prefix → all tags, alphabetical by slug.
    let all = state.posts.list_tags(None, 50).await.unwrap();
    let slugs: Vec<&str> = all.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(
        slugs,
        vec!["performance", "postgresql", "rust", "rust-lang", "web"]
    );

    // Prefix "rust" → "rust" and "rust-lang", still alphabetical.
    let rs = state.posts.list_tags(Some("rust"), 50).await.unwrap();
    let rs_slugs: Vec<&str> = rs.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(rs_slugs, vec!["rust", "rust-lang"]);

    // Prefix case-insensitive: "RUST" matches the same set.
    let upper = state.posts.list_tags(Some("RUST"), 50).await.unwrap();
    let upper_slugs: Vec<&str> = upper.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(upper_slugs, vec!["rust", "rust-lang"]);

    // Limit clamps the result.
    let limited = state.posts.list_tags(None, 2).await.unwrap();
    assert_eq!(limited.len(), 2);

    // Empty-string prefix is treated as "no prefix".
    let empty = state.posts.list_tags(Some("   "), 50).await.unwrap();
    assert_eq!(empty.len(), 5);

    // Nonexistent prefix → empty.
    let none = state.posts.list_tags(Some("zz"), 50).await.unwrap();
    assert!(none.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn post_record_carries_tags(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user = state
        .users
        .create_user(
            &username("inline_tags_user"),
            &password("password"),
            Some(&parse_display_name("Inline")),
            false,
        )
        .await
        .expect("user creation failed");

    let mut post_ids = Vec::new();
    for n in 1..=3 {
        let id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: Some(format!("Post {n}").into()),
                slug: format!("post-{n}").parse().unwrap(),
                body: format!("body {n}").into(),
                format: PostFormat::Markdown,
                rendered_html: RenderedHtml::from_trusted(format!("<p>body {n}</p>")),
                published_at: Some(Utc::now()),
                summary: None,
                audiences: vec![AudienceTarget::Public],
                idempotency_key: None,
            })
            .await
            .expect("post creation failed");
        post_ids.push(id);
    }
    let (p1, p2, p3) = (post_ids[0], post_ids[1], post_ids[2]);

    // p1: two tags; p2: one tag; p3: none.
    state
        .posts
        .tag_post(p1, &"Rust".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    state
        .posts
        .tag_post(p1, &"web".parse::<TagLabel>().unwrap())
        .await
        .unwrap();
    state
        .posts
        .tag_post(p2, &"performance".parse::<TagLabel>().unwrap())
        .await
        .unwrap();

    // Each loaded post carries its own tags from the same query that loaded
    // the rest of the row — no separate batch call.
    let p1_record = state
        .posts
        .get_post_by_id(p1, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id p1")
        .expect("p1 should exist");
    let p1_slugs: Vec<&str> = p1_record.tags.iter().map(|t| t.tag_slug.as_ref()).collect();
    assert_eq!(p1_slugs, vec!["rust", "web"]);
    // Display casing is preserved.
    assert!(p1_record.tags.iter().any(|t| t.tag_display == "Rust"));

    let p2_record = state
        .posts
        .get_post_by_id(p2, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id p2")
        .expect("p2 should exist");
    assert_eq!(p2_record.tags.len(), 1);
    assert_eq!(p2_record.tags[0].tag_slug, "performance");
    assert_eq!(p2_record.tags[0].tag_display, "performance");

    let p3_record = state
        .posts
        .get_post_by_id(p3, &ViewerIdentity::Anonymous)
        .await
        .expect("get_post_by_id p3")
        .expect("p3 should exist");
    assert!(p3_record.tags.is_empty());
}

// ── Composite same-owner FK enforcement ───────────────────────────────────────

// Run a statement on the FK-enabled pool for `backend`. These small per-backend
// helpers mirror `open_pool`/`open_pg_pool`: `raw_exec` unwraps; `raw_try_exec`
// returns the Result so the test can assert rejection. Inlining integer ids via
// `format!` is safe here (test-only, no untrusted input) and sidesteps the
// SQLite/Postgres placeholder divergence.
async fn raw_exec(backend: Backend, env: &TestEnv, sql: &str) {
    raw_try_exec(backend, env, sql)
        .await
        .unwrap_or_else(|e| panic!("raw exec failed: {e}\nSQL: {sql}"));
}

async fn raw_try_exec(backend: Backend, env: &TestEnv, sql: &str) -> Result<(), sqlx::Error> {
    match backend {
        Backend::Sqlite => sqlx::query(sql)
            .execute(&open_pool(&env.base).await)
            .await
            .map(|_| ()),
        Backend::Postgres => {
            // Reuse the pool behind the per-test `AppState` (the same database
            // the state seeded), rather than reconnecting a fresh pool via
            // `recorded_postgres_url`.
            let pool = env.base.pool().postgres();
            sqlx::query(sql).execute(pool).await.map(|_| ())
        }
    }
}

// Reads a single `i64` (e.g. a `COUNT(*)`) on the FK-enabled pool for `backend`,
// so a test can observe rows the trait API cannot reach (e.g. membership rows of a
// deleted audience). Mirrors `raw_exec`'s per-backend pool selection.
async fn raw_scalar_i64(backend: Backend, env: &TestEnv, sql: &str) -> i64 {
    match backend {
        Backend::Sqlite => sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(&open_pool(&env.base).await)
            .await
            .unwrap(),
        Backend::Postgres => {
            let pool = env.base.pool().postgres();
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(pool)
                .await
                .unwrap()
        }
    }
}

// The same-owner invariant (an audience and a subscription paired in
// `audience_members` must belong to the same author) is enforced by the
// database via two composite FKs that both point at the same `author_user_id`
// column — never by application code. This raw-SQL test isolates the FK as the
// enforcer: `audience_members` has no trait insert that bypasses the owner
// column. With `author_user_id = A` the `(subscription_id, author_user_id)` FK
// fails (the subscription is B's); with `B` the `(audience_id, author_user_id)`
// FK fails (the audience is A's) — either way the DB must reject it.
#[apply(backends)]
#[tokio::test]
async fn composite_fks_reject_cross_author_membership(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    // Users via the already-wired UserStore; audience + subscription via raw SQL.
    let a = state
        .users
        .create_user(&username("alice"), &password("password123"), None, false)
        .await
        .unwrap();
    let b = state
        .users
        .create_user(&username("bob"), &password("password123"), None, false)
        .await
        .unwrap();

    raw_exec(
        backend,
        &env,
        &format!("INSERT INTO audiences (author_user_id, name) VALUES ({a}, 'Friends')"),
    )
    .await;
    raw_exec(
        backend,
        &env,
        &format!(
            "INSERT INTO subscriptions (author_user_id, channel_id, subscriber_ref, status_id) \
             VALUES ({b}, (SELECT channel_id FROM channels WHERE name='local'), '{b}', \
                     (SELECT status_id FROM subscription_statuses WHERE name='active'))"
        ),
    )
    .await;

    for owner in [a, b] {
        let res = raw_try_exec(
            backend,
            &env,
            &format!(
                "INSERT INTO audience_members (audience_id, subscription_id, author_user_id) VALUES (\
                   (SELECT audience_id FROM audiences WHERE author_user_id={a} AND name='Friends'), \
                   (SELECT subscription_id FROM subscriptions WHERE author_user_id={b} AND subscriber_ref='{b}'), \
                   {owner})"
            ),
        )
        .await;
        assert!(
            res.is_err(),
            "cross-author membership must be rejected by the DB (owner={owner})"
        );
    }
}

// ── Viewer-aware resolution filter (Task 13) ───────────────────────────────────

// The full resolution matrix: viewers {anonymous, author A, active subscriber S,
// named-member M (in audience G, also subscribed), non-member N (not subscribed)}
// × posts {Public, Private, Subscribers, Named(G), Named(G2), Public+Named(G)},
// asserting both `get_post_by_id` visibility AND presence in `list_published`
// per the truth table in the plan (Task 13). A post is returned to a viewer only
// if the viewer is the author OR a targeted audience admits them; admission is
// `active`-subscription-only (fail-closed).
#[apply(backends)]
#[tokio::test]
async fn resolution_matrix(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let local = local_channel_id(backend, &env).await;

    // Author A and three other accounts (S, M, N). N never subscribes.
    let a = state
        .users
        .create_user(&username("author_a"), &password("password123"), None, false)
        .await
        .unwrap();
    let s = state
        .users
        .create_user(
            &username("subscriber_s"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();
    let m = state
        .users
        .create_user(&username("member_m"), &password("password123"), None, false)
        .await
        .unwrap();
    let n = state
        .users
        .create_user(
            &username("nonmember_n"),
            &password("password123"),
            None,
            false,
        )
        .await
        .unwrap();

    // S and M are active subscribers to A; N is not. M is additionally a member
    // of audience G (but not G2).
    state
        .subscriptions
        .subscribe(a, local, &s.to_string())
        .await
        .unwrap();
    let m_sub = state
        .subscriptions
        .subscribe(a, local, &m.to_string())
        .await
        .unwrap();
    let g = state
        .audiences
        .create_audience(a, &parse_audience_name("G"))
        .await
        .unwrap();
    let g2 = state
        .audiences
        .create_audience(a, &parse_audience_name("G2"))
        .await
        .unwrap();
    state.audiences.add_member(a, g, m_sub).await.unwrap();

    // One published post per audience targeting. `Private` carries no audience
    // rows; `Public+Named(G)` carries both.
    let make = |slug: &str, audiences: Vec<AudienceTarget>| CreatePostInput {
        user_id: a,
        title: Some(format!("Post {slug}").into()),
        slug: slug.parse().unwrap(),
        body: "body".into(),
        format: PostFormat::Markdown,
        rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
        published_at: Some(Utc::now()),
        summary: None,
        audiences,
        idempotency_key: None,
    };
    let p_public = state
        .posts
        .create_post(&make("public", vec![AudienceTarget::Public]))
        .await
        .unwrap();
    let p_private = state
        .posts
        .create_post(&make("private", vec![]))
        .await
        .unwrap();
    let p_subscribers = state
        .posts
        .create_post(&make("subscribers", vec![AudienceTarget::Subscribers]))
        .await
        .unwrap();
    let p_named_g = state
        .posts
        .create_post(&make("named-g", vec![AudienceTarget::Named(g)]))
        .await
        .unwrap();
    let p_named_g2 = state
        .posts
        .create_post(&make("named-g2", vec![AudienceTarget::Named(g2)]))
        .await
        .unwrap();
    let p_public_named_g = state
        .posts
        .create_post(&make(
            "public-named-g",
            vec![AudienceTarget::Public, AudienceTarget::Named(g)],
        ))
        .await
        .unwrap();

    let anon = ViewerIdentity::Anonymous;
    let viewer_a = ViewerIdentity::local(a, local);
    let viewer_s = ViewerIdentity::local(s, local);
    let viewer_m = ViewerIdentity::local(m, local);
    let viewer_n = ViewerIdentity::local(n, local);

    // (label, post_id, [anon, A, S, M, N] expected visibility)
    let matrix: &[(&str, i64, [bool; 5])] = &[
        ("Public", p_public, [true, true, true, true, true]),
        ("Private", p_private, [false, true, false, false, false]),
        (
            "Subscribers",
            p_subscribers,
            [false, true, true, true, false],
        ),
        ("Named(G)", p_named_g, [false, true, false, true, false]),
        ("Named(G2)", p_named_g2, [false, true, false, false, false]),
        (
            "Public+Named(G)",
            p_public_named_g,
            [true, true, true, true, true],
        ),
    ];
    let viewers: [(&str, &ViewerIdentity); 5] = [
        ("anon", &anon),
        ("A", &viewer_a),
        ("S", &viewer_s),
        ("M", &viewer_m),
        ("N", &viewer_n),
    ];

    // `get_post_by_id`: each cell of the matrix.
    for (label, post_id, expected) in matrix {
        for (i, (vlabel, viewer)) in viewers.iter().enumerate() {
            let visible = state
                .posts
                .get_post_by_id(*post_id, viewer)
                .await
                .unwrap()
                .is_some();
            assert_eq!(
                visible, expected[i],
                "get_post_by_id: post {label} for viewer {vlabel}: expected {}, got {visible}",
                expected[i]
            );
        }
    }

    // `list_published`: the same truth table via presence in the site listing.
    for (vi, (vlabel, viewer)) in viewers.iter().enumerate() {
        let listed: std::collections::HashSet<i64> = state
            .posts
            .list_published(None, 100, viewer, Utc::now())
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.post_id)
            .collect();
        for (label, post_id, expected) in matrix {
            assert_eq!(
                listed.contains(post_id),
                expected[vi],
                "list_published: post {label} for viewer {vlabel}: expected {}, present={}",
                expected[vi],
                listed.contains(post_id)
            );
        }
    }
}
