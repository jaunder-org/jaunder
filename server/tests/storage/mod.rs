use chrono::Utc;
use common::config_key::SiteConfigKey;
use common::ids::UserId;
use common::test_support::{
    parse_bio, parse_byte_size, parse_content_hash, parse_content_type, parse_display_name,
    parse_email, parse_filename, parse_page_offset, parse_raw_token, parse_row_limit,
    parse_session_label, parse_url,
};
use host::invite::InviteCode;

use storage::{
    ConfirmPasswordResetError, CreateUserError, ProfileUpdate, RegisterWithInviteError,
    SessionAuthError, UseEmailVerificationError, UsePasswordResetError, UserAuthError,
    UserConfigKey,
};

use rstest::*;
// `#[template]`/`#[apply]` come from the `rstest_reuse` companion crate; the
// glob alone is not enough
// (docs/adr/0124-rstest-reuse-cross-module-templates.md).
use rstest_reuse::*;

use crate::helpers::create_session_for;
use storage::test_support::{Backend, SeedUser, backends, seed_users};

mod audiences;
mod database;
mod email_verification;
mod feed_events;
mod fixtures;
mod fk_constraints;
mod invites;
mod listing;
mod lookups;
mod password_reset;
mod posts;
mod resolution;
mod sessions;
mod site_config;
mod subscriptions;
mod tags;
mod users_auth;

use fixtures::{password, raw_exec, username};

// The Postgres-backed cases below (the `::postgres` expansion of each
// `#[apply(backends)]` test) run against PostgreSQL when `JAUNDER_PG_TEST_URL`
// is set; each acquires its own database (a template clone via
// `unique_postgres_url`/`template_postgres_url`, see helpers), so they run
// safely under the default in-process parallelism. No `--test-threads=1` is
// needed (jaunder-qguq).

#[apply(backends)]
#[tokio::test]
async fn site_config_set_then_get_roundtrips(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    state
        .site_config
        .set(SiteConfigKey::SiteTitle, "Parity Site")
        .await
        .unwrap();
    assert_eq!(
        state
            .site_config
            .get(SiteConfigKey::SiteTitle)
            .await
            .unwrap()
            .as_deref(),
        Some("Parity Site")
    );
}

#[apply(backends)]
#[tokio::test]
async fn get_missing_key_returns_none(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    assert!(
        state
            .site_config
            .get(SiteConfigKey::SiteTitle)
            .await
            .unwrap()
            .is_none()
    );
}

#[apply(backends)]
#[tokio::test]
async fn set_overwrites_existing_value(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;

    state
        .site_config
        .set(SiteConfigKey::SiteTitle, "First")
        .await
        .unwrap();
    state
        .site_config
        .set(SiteConfigKey::SiteTitle, "Second")
        .await
        .unwrap();

    assert_eq!(
        state
            .site_config
            .get(SiteConfigKey::SiteTitle)
            .await
            .unwrap()
            .as_deref(),
        Some("Second")
    );
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
    let user = SeedUser::new().seed(state).await;

    let raw_token = state
        .sessions
        .create_session(user.user_id, &parse_session_label("Laptop"))
        .await
        .unwrap();
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.user_id, user.user_id);
    assert_eq!(record.username, user.username);

    let sessions = state.sessions.list_sessions(user.user_id).await.unwrap();
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
    let user = SeedUser::new().seed(state).await;
    let user_id = user.user_id;

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
        .authenticate(&user.username, &password("new_password123"))
        .await
        .unwrap();
    assert_eq!(authed.user_id, user_id);
}

// --- UserStorage integration tests ---

// --- SessionStorage integration tests ---

// --- InviteStorage integration tests ---

// --- UserStorage::set_email integration tests ---

// --- EmailVerificationStorage integration tests ---

// --- UserStorage::set_password integration tests ---

// --- PasswordResetStorage integration tests ---

// ---------------------------------------------------------------------------
// PostStorage integration tests
// ---------------------------------------------------------------------------

// ── MediaStorage tests ────────────────────────────────────────────────────────

use common::media::{MediaRef, MediaSource};
use storage::{CreateMediaError, DeleteMediaError, MediaRecord, TryDeleteOutcome};

fn make_media_record(
    user_id: UserId,
    sha256: &str,
    filename: &str,
    source: MediaSource,
) -> MediaRecord {
    MediaRecord {
        user_id,
        sha256: parse_content_hash(sha256),
        filename: parse_filename(filename),
        source,
        content_type: parse_content_type("image/jpeg"),
        size_bytes: parse_byte_size("12345"),
        source_url: None,
        created_at: chrono::Utc::now(),
    }
}

#[apply(backends)]
#[tokio::test]
async fn create_and_get_media(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 =
        parse_content_hash("abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234");
    let record = make_media_record(user_id, &sha256, "test.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();

    let fetched = state
        .media
        .get_media(
            user_id,
            &sha256,
            &parse_filename("test.jpg"),
            &MediaSource::Upload,
        )
        .await
        .unwrap();
    let fetched = fetched.expect("record should exist");
    assert_eq!(fetched.user_id, user_id);
    assert_eq!(fetched.sha256, sha256);
    assert_eq!(fetched.filename, "test.jpg");
    assert_eq!(fetched.source, MediaSource::Upload);
    assert_eq!(fetched.content_type, "image/jpeg");
    assert_eq!(fetched.size_bytes, parse_byte_size("12345"));
}

#[apply(backends)]
#[tokio::test]
async fn media_source_url_round_trips_through_the_typed_column(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 =
        parse_content_hash("beef1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234");
    let mut record = make_media_record(user_id, &sha256, "cached.jpg", MediaSource::Cached);
    // Spelled non-canonically on the way in: `TaggedUrl`'s `FromStr` lowercases the host
    // and strips the default port, so the value stored is already normalized — asserting the
    // canonical form on read-back is what shows the column carries the *newtype*, not the
    // text as typed (#675).
    record.source_url = Some(parse_url("https://Example.COM:443/x.png"));

    state.media.create_media(&record).await.unwrap();

    let fetched = state
        .media
        .get_media(
            user_id,
            &sha256,
            &parse_filename("cached.jpg"),
            &MediaSource::Cached,
        )
        .await
        .unwrap()
        .expect("record should exist");
    assert_eq!(
        fetched.source_url.as_deref(),
        Some("https://example.com/x.png")
    );
}

#[apply(backends)]
#[tokio::test]
async fn media_row_with_an_invalid_source_url_fails_to_decode(#[case] backend: Backend) {
    // This is what makes `Option<MediaSourceUrl>` a contract rather than documentation: a
    // value that is not a valid absolute `http(s)` URL cannot be read back as one. Nothing
    // writes `source_url` yet (the remote-caching ingest does not exist), so a hand-edited
    // or future-buggy writer is exactly the threat, and it is inserted by raw SQL here
    // because the type makes it unconstructible in Rust.
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    env.base
        .pool()
        .execute(&format!(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes, \
             source_url) VALUES ({}, \
             'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc', 'c.png', \
             'cached', 'image/png', 10, 'not a url')",
            i64::from(user_id),
        ))
        .await
        .expect("raw insert should succeed — the database has no opinion on the text");

    let fetched = state
        .media
        .get_media(
            user_id,
            &parse_content_hash("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
            &parse_filename("c.png"),
            &MediaSource::Cached,
        )
        .await;
    assert!(
        fetched.is_err(),
        "a non-URL source_url must be a column-decode error, got {fetched:?}"
    );
}

#[apply(backends)]
#[tokio::test]
async fn list_media_skips_rows_that_fail_to_decode(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    // A valid record via the normal (validating) path.
    let good_sha = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
    let record = make_media_record(user_id, good_sha, "good.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();

    // A row whose `filename` column is a non-canonical value, inserted directly to
    // bypass the validating `create_media` (the `Filename` type makes an un-sanitized
    // name unconstructible in Rust). `media_record_from_row` fails to decode it.
    // created_at/source_url are omitted so both backends' column defaults apply.
    env.base
        .pool()
        .execute(&format!(
            "INSERT INTO media (user_id, sha256, filename, source, content_type, size_bytes) \
             VALUES ({}, 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', \
             '../escape', 'upload', 'image/png', 10)",
            i64::from(user_id),
        ))
        .await
        .unwrap();

    // list_media returns the decodable row and silently skips the corrupt one, rather
    // than failing the whole query (which would hide the user's valid media too).
    let listed = state
        .media
        .list_media(user_id, None, parse_row_limit("10"), parse_page_offset("0"))
        .await
        .unwrap();
    assert_eq!(
        listed.len(),
        1,
        "the corrupt row must be skipped and the valid row returned"
    );
    assert_eq!(listed[0].filename, "good.jpg");

    // A direct lookup of the corrupt row still surfaces the decode error (single-row
    // lookups stay strict — only the list path degrades gracefully).
    let direct = state
        .media
        .find_by_hash(
            &parse_content_hash("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            &MediaSource::Upload,
        )
        .await;
    assert!(
        direct.is_err(),
        "a direct lookup of the corrupt row must error"
    );
}

#[apply(backends)]
#[tokio::test]
async fn duplicate_media_returns_already_exists(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

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
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 =
        parse_content_hash("cccc1234cccc1234cccc1234cccc1234cccc1234cccc1234cccc1234cccc1234");
    let record = make_media_record(user_id, &sha256, "del.jpg", MediaSource::Upload);
    state.media.create_media(&record).await.unwrap();
    let outcome = state
        .media
        .try_delete_media(
            user_id,
            &MediaRef {
                source: MediaSource::Upload,
                sha256: sha256.clone(),
                filename: parse_filename("del.jpg"),
            },
            false,
        )
        .await
        .unwrap();
    assert_eq!(outcome, TryDeleteOutcome::Deleted);

    let fetched = state
        .media
        .get_media(
            user_id,
            &sha256,
            &parse_filename("del.jpg"),
            &MediaSource::Upload,
        )
        .await
        .unwrap();
    assert!(fetched.is_none(), "record should have been deleted");
}

#[apply(backends)]
#[tokio::test]
async fn delete_nonexistent_returns_not_found(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 =
        parse_content_hash("dddd1234dddd1234dddd1234dddd1234dddd1234dddd1234dddd1234dddd1234");
    let err = state
        .media
        .try_delete_media(
            user_id,
            &MediaRef {
                source: MediaSource::Upload,
                sha256: sha256.clone(),
                filename: parse_filename("ghost.jpg"),
            },
            false,
        )
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
    let [user_a, user_b] = seed_users(state).await;

    let sha1 = "eeee1234eeee1234eeee1234eeee1234eeee1234eeee1234eeee1234eeee1234".to_string();
    let sha2 = "ffff1234ffff1234ffff1234ffff1234ffff1234ffff1234ffff1234ffff1234".to_string();
    let sha3 = "9999123499991234999912349999123499991234999912349999123499991234".to_string();

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

    let results = state
        .media
        .list_media(user_a, None, parse_row_limit("10"), parse_page_offset("0"))
        .await
        .unwrap();
    assert_eq!(results.len(), 2, "user_a should have 2 records");
    assert!(results.iter().all(|r| r.user_id == user_a));
}

#[apply(backends)]
#[tokio::test]
async fn list_media_filtered_by_source(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha_up = "8888123488881234888812348888123488881234888812348888123488881234".to_string();
    let sha_ca = "7777123477771234777712347777123477771234777712347777123477771234".to_string();

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
        .list_media(
            user_id,
            Some(&MediaSource::Upload),
            parse_row_limit("10"),
            parse_page_offset("0"),
        )
        .await
        .unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].source, MediaSource::Upload);

    let cached = state
        .media
        .list_media(
            user_id,
            Some(&MediaSource::Cached),
            parse_row_limit("10"),
            parse_page_offset("0"),
        )
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
    let user_id = SeedUser::new().seed(state).await.user_id;

    let usage = state.media.get_user_upload_usage(user_id).await.unwrap();
    assert_eq!(usage, parse_byte_size("0"));
}

#[apply(backends)]
#[tokio::test]
async fn get_user_upload_usage_sums_uploads_only(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha_up = "6666123466661234666612346666123466661234666612346666123466661234".to_string();
    let sha_ca = "5555123455551234555512345555123455551234555512345555123455551234".to_string();

    let mut upload = make_media_record(user_id, &sha_up, "upload.jpg", MediaSource::Upload);
    upload.size_bytes = parse_byte_size("1000");
    state.media.create_media(&upload).await.unwrap();

    let mut cached = make_media_record(user_id, &sha_ca, "cached.jpg", MediaSource::Cached);
    cached.size_bytes = parse_byte_size("9999");
    state.media.create_media(&cached).await.unwrap();

    let usage = state.media.get_user_upload_usage(user_id).await.unwrap();
    assert_eq!(
        usage,
        parse_byte_size("1000"),
        "only upload bytes should count toward usage"
    );
}

#[apply(backends)]
#[tokio::test]
async fn find_by_hash_returns_any_match(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    let sha256 =
        parse_content_hash("4444123444441234444412344444123444441234444412344444123444441234");
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
    let user_id = SeedUser::new().seed(state).await.user_id;

    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert!(val.is_none());
}

/// D8: the typed key is the only way in, and a value survives it unchanged.
#[apply(backends)]
#[tokio::test]
async fn user_config_round_trips_through_typed_keys(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "markdown")
        .await
        .unwrap();
    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert_eq!(val.as_deref(), Some("markdown"));
}

#[apply(backends)]
#[tokio::test]
async fn user_config_set_and_get(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "org")
        .await
        .unwrap();
    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert_eq!(val.as_deref(), Some("org"));
}

#[apply(backends)]
#[tokio::test]
async fn user_config_overwrite(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "markdown")
        .await
        .unwrap();
    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "org")
        .await
        .unwrap();
    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert_eq!(val.as_deref(), Some("org"));
}

#[apply(backends)]
#[tokio::test]
async fn user_config_delete_removes_key(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .set(user_id, UserConfigKey::DefaultPostFormat, "org")
        .await
        .unwrap();
    state
        .user_config
        .delete(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    let val = state
        .user_config
        .get(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
    assert!(val.is_none());
}

#[apply(backends)]
#[tokio::test]
async fn user_config_delete_nonexistent_is_ok(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = SeedUser::new().seed(state).await.user_id;

    state
        .user_config
        .delete(user_id, UserConfigKey::DefaultPostFormat)
        .await
        .unwrap();
}
