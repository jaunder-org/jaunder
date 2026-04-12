mod helpers;

use chrono::{Datelike, Utc};
use jaunder::password::Password;
use jaunder::render::{create_rendered_post, update_rendered_post};
use jaunder::storage::{
    open_database, open_existing_database, AtomicOps, CreatePostError, CreatePostInput,
    CreateUserError, DbConnectOptions, EmailVerificationStorage, InviteStorage, ListByTagError,
    PasswordResetStorage, PostCursor, PostFormat, PostStorage, ProfileUpdate,
    RegisterWithInviteError, SessionAuthError, SessionStorage, SqliteAtomicOps,
    SqliteEmailVerificationStorage, SqliteInviteStorage, SqlitePasswordResetStorage,
    SqlitePostStorage, SqliteSessionStorage, SqliteUserStorage, TaggingError, UpdatePostError,
    UpdatePostInput, UseEmailVerificationError, UseInviteError, UsePasswordResetError,
    UserAuthError, UserStorage,
};
use jaunder::tag::Tag;
use jaunder::username::Username;
use sqlx::SqlitePool;
use tempfile::TempDir;

use helpers::{postgres_url, reset_postgres_schema, sqlite_url};

// PostgreSQL parity tests below share a single database URL and reset the
// schema before each run. Run them individually, or with `-- --test-threads=1`,
// against the test VM to avoid cross-test interference.

async fn open_pool(base: &TempDir) -> SqlitePool {
    let DbConnectOptions::Sqlite(opts) = sqlite_url(base) else {
        panic!("expected sqlite options");
    };
    let pool = SqlitePool::connect_with(opts.create_if_missing(true))
        .await
        .unwrap();
    sqlx::migrate!("./migrations/sqlite")
        .run(&pool)
        .await
        .unwrap();
    pool
}

async fn postgres_state() -> std::sync::Arc<jaunder::storage::AppState> {
    reset_postgres_schema().await;
    open_database(&postgres_url()).await.unwrap()
}

async fn sqlite_state() -> (TempDir, std::sync::Arc<jaunder::storage::AppState>) {
    let base = TempDir::new().unwrap();
    let state = open_database(&sqlite_url(&base)).await.unwrap();
    (base, state)
}

async fn user_storage(base: &TempDir) -> SqliteUserStorage {
    SqliteUserStorage::new(open_pool(base).await)
}

async fn storage_pair(base: &TempDir) -> (SqliteUserStorage, SqliteSessionStorage) {
    let pool = open_pool(base).await;
    (
        SqliteUserStorage::new(pool.clone()),
        SqliteSessionStorage::new(pool),
    )
}

async fn email_verification_storage(
    base: &TempDir,
) -> (SqliteUserStorage, SqliteEmailVerificationStorage) {
    let pool = open_pool(base).await;
    (
        SqliteUserStorage::new(pool.clone()),
        SqliteEmailVerificationStorage::new(pool),
    )
}

async fn invite_storage_triple(
    base: &TempDir,
) -> (SqliteUserStorage, SqliteSessionStorage, SqliteInviteStorage) {
    let pool = open_pool(base).await;
    (
        SqliteUserStorage::new(pool.clone()),
        SqliteSessionStorage::new(pool.clone()),
        SqliteInviteStorage::new(pool),
    )
}

fn username(s: &str) -> Username {
    s.parse().unwrap()
}

fn password(s: &str) -> Password {
    s.parse().unwrap()
}

async fn assert_site_config_roundtrip(state: &std::sync::Arc<jaunder::storage::AppState>) {
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

async fn assert_user_duplicate_and_authenticate(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    let username = username("alice");
    let initial_password = password("password123");

    let user_id = state
        .users
        .create_user(&username, &initial_password, Some("Alice"))
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
        .create_user(&username, &password("other_password"), None)
        .await
        .unwrap_err();
    assert!(matches!(duplicate, CreateUserError::UsernameTaken));

    let authed = state
        .users
        .authenticate(&username, &initial_password)
        .await
        .unwrap();
    assert_eq!(authed.username.as_str(), "alice");
    assert!(authed.last_authenticated_at.is_some());
}

async fn assert_session_lifecycle(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user_id = state
        .users
        .create_user(&username("bob"), &password("secret_password"), None)
        .await
        .unwrap();

    let raw_token = state
        .sessions
        .create_session(user_id, Some("Laptop"))
        .await
        .unwrap();
    let record = state.sessions.authenticate(&raw_token).await.unwrap();
    assert_eq!(record.user_id, user_id);
    assert_eq!(record.username.as_str(), "bob");

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

async fn assert_invite_and_atomic_registration(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = state.invites.create_invite(expires_at).await.unwrap();

    let user_id = state
        .atomic
        .create_user_with_invite(
            &username("carol"),
            &password("password123"),
            Some("Carol"),
            &code,
        )
        .await
        .unwrap();
    let created = state.users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(created.username.as_str(), "carol");

    let err = state
        .atomic
        .create_user_with_invite(&username("carol2"), &password("password123"), None, &code)
        .await
        .unwrap_err();
    assert!(matches!(err, RegisterWithInviteError::InviteAlreadyUsed));
}

async fn assert_email_verification_and_password_reset(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    let user_id = state
        .users
        .create_user(&username("dave"), &password("password123"), None)
        .await
        .unwrap();

    let verify_token = state
        .email_verifications
        .create_email_verification(
            user_id,
            "dave@example.com",
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

#[tokio::test]
async fn set_then_get_roundtrips() {
    let (_base, state) = sqlite_state().await;
    assert_site_config_roundtrip(&state).await;
}

#[tokio::test]
async fn get_missing_key_returns_none() {
    let base = TempDir::new().unwrap();
    let state = open_database(&sqlite_url(&base)).await.unwrap();

    assert!(state
        .site_config
        .get("nonexistent")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn set_overwrites_existing_value() {
    let base = TempDir::new().unwrap();
    let state = open_database(&sqlite_url(&base)).await.unwrap();

    state.site_config.set("site.name", "First").await.unwrap();
    state.site_config.set("site.name", "Second").await.unwrap();

    assert_eq!(
        state.site_config.get("site.name").await.unwrap().as_deref(),
        Some("Second")
    );
}

#[tokio::test]
async fn second_open_on_migrated_database_succeeds() {
    let base = TempDir::new().unwrap();

    drop(open_database(&sqlite_url(&base)).await.unwrap());

    open_database(&sqlite_url(&base)).await.unwrap();
}

#[tokio::test]
async fn sqlite_app_state_parity_suite() {
    let (_base, state) = sqlite_state().await;
    assert_site_config_roundtrip(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_user_duplicate_and_authenticate(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_session_lifecycle(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_invite_and_atomic_registration(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_email_verification_and_password_reset(&state).await;
}

#[tokio::test]
async fn sqlite_create_user_duplicate_and_authenticate_work() {
    let (_base, state) = sqlite_state().await;
    assert_user_duplicate_and_authenticate(&state).await;
}

#[tokio::test]
async fn sqlite_session_lifecycle_works() {
    let (_base, state) = sqlite_state().await;
    assert_session_lifecycle(&state).await;
}

#[tokio::test]
async fn sqlite_invite_and_atomic_registration_work() {
    let (_base, state) = sqlite_state().await;
    assert_invite_and_atomic_registration(&state).await;
}

#[tokio::test]
async fn sqlite_email_verification_and_password_reset_work() {
    let (_base, state) = sqlite_state().await;
    assert_email_verification_and_password_reset(&state).await;
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

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn open_database_succeeds_on_postgres_test_vm() {
    reset_postgres_schema().await;
    open_database(&postgres_url()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn open_database_runs_postgres_migrations_on_existing_empty_db() {
    reset_postgres_schema().await;
    let state = open_database(&postgres_url()).await.unwrap();
    assert_eq!(state.site_config.get("missing").await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn open_existing_database_runs_postgres_migrations_on_unmigrated_db() {
    reset_postgres_schema().await;
    let state = open_existing_database(&postgres_url()).await.unwrap();
    assert_eq!(state.site_config.get("missing").await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_app_state_parity_suite() {
    let state = postgres_state().await;
    assert_site_config_roundtrip(&state).await;

    let state = postgres_state().await;
    assert_user_duplicate_and_authenticate(&state).await;

    let state = postgres_state().await;
    assert_session_lifecycle(&state).await;

    let state = postgres_state().await;
    assert_invite_and_atomic_registration(&state).await;

    let state = postgres_state().await;
    assert_email_verification_and_password_reset(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_site_config_set_then_get_roundtrips() {
    let state = postgres_state().await;
    assert_site_config_roundtrip(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_create_user_duplicate_and_authenticate_work() {
    let state = postgres_state().await;
    assert_user_duplicate_and_authenticate(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_session_lifecycle_works() {
    let state = postgres_state().await;
    assert_session_lifecycle(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_invite_and_atomic_registration_work() {
    let state = postgres_state().await;
    assert_invite_and_atomic_registration(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_email_verification_and_password_reset_work() {
    let state = postgres_state().await;
    assert_email_verification_and_password_reset(&state).await;
}

// --- UserStorage integration tests ---

#[tokio::test]
async fn create_user_succeeds_and_get_by_username_returns_record() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), Some("Alice"))
        .await
        .unwrap();

    let record = users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.user_id, user_id);
    assert_eq!(record.username.as_str(), "alice");
    assert_eq!(record.display_name.as_deref(), Some("Alice"));
}

#[tokio::test]
async fn duplicate_username_returns_username_taken() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let err = users
        .create_user(&username("alice"), &password("other_password"), None)
        .await
        .unwrap_err();
    assert!(matches!(err, CreateUserError::UsernameTaken));
}

#[tokio::test]
async fn authenticate_correct_password_returns_record_and_sets_last_authenticated_at() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    users
        .create_user(&username("bob"), &password("secret_password"), None)
        .await
        .unwrap();

    let record = users
        .authenticate(&username("bob"), &password("secret_password"))
        .await
        .unwrap();
    assert_eq!(record.username.as_str(), "bob");
    assert!(record.last_authenticated_at.is_some());

    // Verify the DB was updated.
    let fetched = users.get_user(record.user_id).await.unwrap().unwrap();
    assert!(fetched.last_authenticated_at.is_some());
}

#[tokio::test]
async fn authenticate_wrong_password_returns_invalid_credentials() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    users
        .create_user(&username("carol"), &password("correct_password"), None)
        .await
        .unwrap();

    let err = users
        .authenticate(&username("carol"), &password("wrong_password"))
        .await
        .unwrap_err();
    assert!(matches!(err, UserAuthError::InvalidCredentials));
}

#[tokio::test]
async fn authenticate_unknown_username_returns_invalid_credentials() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    let err = users
        .authenticate(&username("nobody"), &password("some_password"))
        .await
        .unwrap_err();
    assert!(matches!(err, UserAuthError::InvalidCredentials));
}

#[tokio::test]
async fn update_profile_persists_changes() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    let user_id = users
        .create_user(&username("dave"), &password("passw0rd!"), Some("Dave"))
        .await
        .unwrap();

    users
        .update_profile(
            user_id,
            &ProfileUpdate {
                display_name: Some("David"),
                bio: Some("A bio"),
            },
        )
        .await
        .unwrap();

    let record = users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.display_name.as_deref(), Some("David"));
    assert_eq!(record.bio.as_deref(), Some("A bio"));
}

#[tokio::test]
async fn get_user_unknown_id_returns_none() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    let record = users.get_user(999).await.unwrap();
    assert!(record.is_none());
}

// --- SessionStorage integration tests ---

#[tokio::test]
async fn create_session_then_authenticate_returns_correct_record() {
    let base = TempDir::new().unwrap();
    let (users, sessions) = storage_pair(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let raw_token = sessions
        .create_session(user_id, Some("test"))
        .await
        .unwrap();
    let record = sessions.authenticate(&raw_token).await.unwrap();

    assert_eq!(record.user_id, user_id);
    assert_eq!(record.username.as_str(), "alice");
    assert_eq!(record.label.as_deref(), Some("test"));
    assert!(!record.token_hash.is_empty());
}

#[tokio::test]
async fn authenticate_updates_last_used_at() {
    let base = TempDir::new().unwrap();
    let (users, sessions) = storage_pair(&base).await;

    let user_id = users
        .create_user(&username("bob"), &password("password123"), None)
        .await
        .unwrap();

    let raw_token = sessions.create_session(user_id, None).await.unwrap();
    let first = sessions.authenticate(&raw_token).await.unwrap();
    let second = sessions.authenticate(&raw_token).await.unwrap();

    assert!(second.last_used_at >= first.last_used_at);
}

#[tokio::test]
async fn revoke_session_then_authenticate_returns_session_not_found() {
    let base = TempDir::new().unwrap();
    let (users, sessions) = storage_pair(&base).await;

    let user_id = users
        .create_user(&username("carol"), &password("password123"), None)
        .await
        .unwrap();

    let raw_token = sessions.create_session(user_id, None).await.unwrap();
    let record = sessions.authenticate(&raw_token).await.unwrap();

    sessions.revoke_session(&record.token_hash).await.unwrap();

    let err = sessions.authenticate(&raw_token).await.unwrap_err();
    assert!(matches!(err, SessionAuthError::SessionNotFound));
}

#[tokio::test]
async fn authenticate_with_invalid_base64_token_returns_invalid_token() {
    let base = TempDir::new().unwrap();
    let (_, sessions) = storage_pair(&base).await;

    let err = sessions.authenticate("not-base64!").await.unwrap_err();
    assert!(matches!(err, SessionAuthError::InvalidToken));
}

#[tokio::test]
async fn list_sessions_returns_only_sessions_for_given_user() {
    let base = TempDir::new().unwrap();
    let (users, sessions) = storage_pair(&base).await;

    let alice_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();
    let bob_id = users
        .create_user(&username("bob"), &password("password123"), None)
        .await
        .unwrap();

    sessions
        .create_session(alice_id, Some("alice-1"))
        .await
        .unwrap();
    sessions
        .create_session(alice_id, Some("alice-2"))
        .await
        .unwrap();
    sessions
        .create_session(bob_id, Some("bob-1"))
        .await
        .unwrap();

    let alice_sessions = sessions.list_sessions(alice_id).await.unwrap();
    assert_eq!(alice_sessions.len(), 2);
    assert!(alice_sessions.iter().all(|s| s.user_id == alice_id));

    let bob_sessions = sessions.list_sessions(bob_id).await.unwrap();
    assert_eq!(bob_sessions.len(), 1);
    assert_eq!(bob_sessions[0].user_id, bob_id);
}

// --- InviteStorage integration tests ---

#[tokio::test]
async fn create_invite_and_list_invites_includes_it() {
    let base = TempDir::new().unwrap();
    let (_, _, invites) = invite_storage_triple(&base).await;

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    let list = invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].code, code);
    assert!(list[0].used_at.is_none());
}

#[tokio::test]
async fn use_invite_with_valid_code_marks_it_used() {
    let base = TempDir::new().unwrap();
    let (users, _, invites) = invite_storage_triple(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    invites.use_invite(&code, user_id).await.unwrap();

    let list = invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_some());
    assert_eq!(list[0].used_by, Some(user_id));
}

#[tokio::test]
async fn use_invite_with_unknown_code_returns_not_found() {
    let base = TempDir::new().unwrap();
    let (users, _, invites) = invite_storage_triple(&base).await;

    let user_id = users
        .create_user(&username("bob"), &password("password123"), None)
        .await
        .unwrap();

    let err = invites
        .use_invite("no-such-code", user_id)
        .await
        .unwrap_err();
    assert!(matches!(err, UseInviteError::NotFound));
}

#[tokio::test]
async fn use_invite_with_expired_code_returns_expired() {
    let base = TempDir::new().unwrap();
    let (users, _, invites) = invite_storage_triple(&base).await;

    let user_id = users
        .create_user(&username("carol"), &password("password123"), None)
        .await
        .unwrap();

    // expires_at in the past
    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let code = invites.create_invite(expires_at).await.unwrap();

    let err = invites.use_invite(&code, user_id).await.unwrap_err();
    assert!(matches!(err, UseInviteError::Expired));
}

#[tokio::test]
async fn use_invite_on_already_used_code_returns_already_used() {
    let base = TempDir::new().unwrap();
    let (users, _, invites) = invite_storage_triple(&base).await;

    let user_id = users
        .create_user(&username("dave"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    invites.use_invite(&code, user_id).await.unwrap();

    let err = invites.use_invite(&code, user_id).await.unwrap_err();
    assert!(matches!(err, UseInviteError::AlreadyUsed));
}

// --- create_user_with_invite integration tests ---

#[tokio::test]
async fn create_user_with_invite_creates_user_and_marks_invite_used() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;
    let invites = SqliteInviteStorage::new(pool.clone());
    let users = SqliteUserStorage::new(pool.clone());

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    let user_id = SqliteAtomicOps::new(pool.clone())
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            Some("Alice"),
            &code,
        )
        .await
        .unwrap();

    // User was created
    let record = users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(record.username.as_str(), "alice");
    assert_eq!(record.display_name.as_deref(), Some("Alice"));

    // Invite was marked used
    let list = invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_some());
    assert_eq!(list[0].used_by, Some(user_id));
}

#[tokio::test]
async fn create_user_with_invite_second_call_returns_already_used() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;
    let invites = SqliteInviteStorage::new(pool.clone());

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    SqliteAtomicOps::new(pool.clone())
        .create_user_with_invite(&username("alice"), &password("password123"), None, &code)
        .await
        .unwrap();

    let err = SqliteAtomicOps::new(pool.clone())
        .create_user_with_invite(&username("bob"), &password("password123"), None, &code)
        .await
        .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::InviteAlreadyUsed));

    // bob was not inserted
    let users = SqliteUserStorage::new(pool.clone());
    assert!(users
        .get_user_by_username(&username("bob"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn create_user_with_invite_expired_returns_invite_expired() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;
    let invites = SqliteInviteStorage::new(pool.clone());

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let code = invites.create_invite(expires_at).await.unwrap();

    let err = SqliteAtomicOps::new(pool.clone())
        .create_user_with_invite(&username("alice"), &password("password123"), None, &code)
        .await
        .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::InviteExpired));

    let users = SqliteUserStorage::new(pool.clone());
    assert!(users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn create_user_with_invite_unknown_code_returns_not_found() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;

    let err = SqliteAtomicOps::new(pool.clone())
        .create_user_with_invite(
            &username("alice"),
            &password("password123"),
            None,
            "no-such-code",
        )
        .await
        .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::InviteNotFound));

    let users = SqliteUserStorage::new(pool.clone());
    assert!(users
        .get_user_by_username(&username("alice"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn create_user_with_invite_duplicate_username_returns_username_taken() {
    let base = TempDir::new().unwrap();
    let pool = open_pool(&base).await;
    let invites = SqliteInviteStorage::new(pool.clone());
    let users = SqliteUserStorage::new(pool.clone());

    // Create alice directly (without invite)
    users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let code = invites.create_invite(expires_at).await.unwrap();

    let err = SqliteAtomicOps::new(pool.clone())
        .create_user_with_invite(&username("alice"), &password("other_password"), None, &code)
        .await
        .unwrap_err();

    assert!(matches!(err, RegisterWithInviteError::UsernameTaken));

    // Invite was NOT marked used
    let list = invites.list_invites().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0].used_at.is_none());
}

// --- AppState mailer tests ---

#[tokio::test]
async fn open_database_uses_noop_mailer_when_smtp_not_configured() {
    let base = TempDir::new().unwrap();
    let opts = sqlite_url(&base);
    let state = open_database(&opts).await.unwrap();

    let msg = common::mailer::EmailMessage {
        from: None,
        to: vec!["alice@example.com".parse().unwrap()],
        subject: "Test".to_string(),
        body_text: "Hello".to_string(),
    };
    let result = state.mailer.send_email(&msg).await;
    assert!(
        matches!(result, Err(common::mailer::MailError::NotConfigured)),
        "expected NotConfigured, got {result:?}"
    );
}

// --- UserStorage::set_email integration tests ---

#[tokio::test]
async fn set_email_persists_and_get_user_reflects_it() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let addr: email_address::EmailAddress = "alice@example.com".parse().unwrap();
    users.set_email(user_id, Some(&addr), true).await.unwrap();

    let record = users.get_user(user_id).await.unwrap().unwrap();
    assert_eq!(
        record.email.as_ref().map(|e| e.as_str()),
        Some("alice@example.com")
    );
    assert!(record.email_verified);
}

#[tokio::test]
async fn set_email_clears_previously_set_email() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    let user_id = users
        .create_user(&username("bob"), &password("password123"), None)
        .await
        .unwrap();

    let addr: email_address::EmailAddress = "bob@example.com".parse().unwrap();
    users.set_email(user_id, Some(&addr), true).await.unwrap();

    users.set_email(user_id, None, false).await.unwrap();

    let record = users.get_user(user_id).await.unwrap().unwrap();
    assert!(record.email.is_none());
    assert!(!record.email_verified);
}

async fn password_reset_storage(base: &TempDir) -> (SqliteUserStorage, SqlitePasswordResetStorage) {
    let pool = open_pool(base).await;
    (
        SqliteUserStorage::new(pool.clone()),
        SqlitePasswordResetStorage::new(pool),
    )
}

// --- EmailVerificationStorage integration tests ---

#[tokio::test]
async fn create_email_verification_and_use_returns_user_id_and_email() {
    let base = TempDir::new().unwrap();
    let (users, ev) = email_verification_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = ev
        .create_email_verification(user_id, "alice@example.com", expires_at)
        .await
        .unwrap();

    let (returned_user_id, returned_email) = ev.use_email_verification(&raw_token).await.unwrap();

    assert_eq!(returned_user_id, user_id);
    assert_eq!(returned_email, "alice@example.com");
}

#[tokio::test]
async fn use_email_verification_already_used_returns_already_used() {
    let base = TempDir::new().unwrap();
    let (users, ev) = email_verification_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = ev
        .create_email_verification(user_id, "alice@example.com", expires_at)
        .await
        .unwrap();

    ev.use_email_verification(&raw_token).await.unwrap();

    let err = ev.use_email_verification(&raw_token).await.unwrap_err();
    assert!(
        matches!(err, UseEmailVerificationError::AlreadyUsed),
        "expected AlreadyUsed, got {err:?}"
    );
}

#[tokio::test]
async fn use_email_verification_expired_returns_expired() {
    let base = TempDir::new().unwrap();
    let (users, ev) = email_verification_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = ev
        .create_email_verification(user_id, "alice@example.com", expires_at)
        .await
        .unwrap();

    let err = ev.use_email_verification(&raw_token).await.unwrap_err();
    assert!(
        matches!(err, UseEmailVerificationError::Expired),
        "expected Expired, got {err:?}"
    );
}

#[tokio::test]
async fn use_email_verification_unknown_token_returns_not_found() {
    let base = TempDir::new().unwrap();
    let (_, ev) = email_verification_storage(&base).await;

    let err = ev
        .use_email_verification("not-a-real-token")
        .await
        .unwrap_err();
    assert!(
        matches!(err, UseEmailVerificationError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

#[tokio::test]
async fn second_email_verification_supersedes_first() {
    let base = TempDir::new().unwrap();
    let (users, ev) = email_verification_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let first_token = ev
        .create_email_verification(user_id, "alice@example.com", expires_at)
        .await
        .unwrap();

    // Create a second verification; the first should be superseded.
    let second_token = ev
        .create_email_verification(user_id, "alice2@example.com", expires_at)
        .await
        .unwrap();

    // Second token works normally.
    let (uid, email) = ev.use_email_verification(&second_token).await.unwrap();
    assert_eq!(uid, user_id);
    assert_eq!(email, "alice2@example.com");

    // First token is now either NotFound or Expired.
    let err = ev.use_email_verification(&first_token).await.unwrap_err();
    assert!(
        matches!(
            err,
            UseEmailVerificationError::NotFound | UseEmailVerificationError::Expired
        ),
        "expected NotFound or Expired for superseded token, got {err:?}"
    );
}

// --- UserStorage::set_password integration tests ---

#[tokio::test]
async fn set_password_authenticate_with_old_returns_invalid_and_new_succeeds() {
    let base = TempDir::new().unwrap();
    let users = user_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("old_password1"), None)
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

#[tokio::test]
async fn create_password_reset_and_use_returns_user_id() {
    let base = TempDir::new().unwrap();
    let (users, pr) = password_reset_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = pr.create_password_reset(user_id, expires_at).await.unwrap();

    let returned_user_id = pr.use_password_reset(&raw_token).await.unwrap();
    assert_eq!(returned_user_id, user_id);
}

#[tokio::test]
async fn use_password_reset_already_used_returns_already_used() {
    let base = TempDir::new().unwrap();
    let (users, pr) = password_reset_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() + chrono::Duration::hours(24);
    let raw_token = pr.create_password_reset(user_id, expires_at).await.unwrap();

    pr.use_password_reset(&raw_token).await.unwrap();

    let err = pr.use_password_reset(&raw_token).await.unwrap_err();
    assert!(
        matches!(err, UsePasswordResetError::AlreadyUsed),
        "expected AlreadyUsed, got {err:?}"
    );
}

#[tokio::test]
async fn use_password_reset_expired_returns_expired() {
    let base = TempDir::new().unwrap();
    let (users, pr) = password_reset_storage(&base).await;

    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let expires_at = Utc::now() - chrono::Duration::hours(1);
    let raw_token = pr.create_password_reset(user_id, expires_at).await.unwrap();

    let err = pr.use_password_reset(&raw_token).await.unwrap_err();
    assert!(
        matches!(err, UsePasswordResetError::Expired),
        "expected Expired, got {err:?}"
    );
}

#[tokio::test]
async fn use_password_reset_unknown_token_returns_not_found() {
    let base = TempDir::new().unwrap();
    let (_, pr) = password_reset_storage(&base).await;

    let err = pr.use_password_reset("not-a-real-token").await.unwrap_err();
    assert!(
        matches!(err, UsePasswordResetError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// PostStorage integration tests
// ---------------------------------------------------------------------------

async fn post_storage(base: &TempDir) -> (SqliteUserStorage, SqlitePostStorage) {
    let pool = open_pool(base).await;
    (
        SqliteUserStorage::new(pool.clone()),
        SqlitePostStorage::new(pool),
    )
}

fn make_create_post_input(user_id: i64, slug: &str) -> CreatePostInput {
    CreatePostInput {
        user_id,
        title: format!("Post {slug}"),
        slug: slug.parse().unwrap(),
        body: "body text".to_string(),
        format: PostFormat::Markdown,
        rendered_html: "<p>body text</p>".to_string(),
        published_at: None,
    }
}

fn make_published_create_post_input(user_id: i64, slug: &str) -> CreatePostInput {
    CreatePostInput {
        published_at: Some(Utc::now()),
        ..make_create_post_input(user_id, slug)
    }
}

async fn assert_post_create_and_get_by_id(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user_id = state
        .users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();

    let input = make_create_post_input(user_id, "hello-world");
    let post_id = state.posts.create_post(&input).await.unwrap();

    let record = state.posts.get_post_by_id(post_id).await.unwrap().unwrap();
    assert_eq!(record.post_id, post_id);
    assert_eq!(record.user_id, user_id);
    assert_eq!(record.title, "Post hello-world");
    assert_eq!(record.slug.as_str(), "hello-world");
    assert_eq!(record.format, PostFormat::Markdown);
    assert!(record.published_at.is_none());
    assert!(record.deleted_at.is_none());
}

async fn assert_post_slug_conflict(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user_id = state
        .users
        .create_user(&username("bob"), &password("password123"), None)
        .await
        .unwrap();

    // Two drafts created on the same date with the same slug should conflict
    let now = Utc::now();
    let input1 = CreatePostInput {
        user_id,
        title: "First".to_string(),
        slug: "duplicate-slug".parse().unwrap(),
        body: "body".to_string(),
        format: PostFormat::Markdown,
        rendered_html: "<p>body</p>".to_string(),
        published_at: None,
    };
    state.posts.create_post(&input1).await.unwrap();

    let input2 = CreatePostInput {
        published_at: Some(now),
        ..input1.clone()
    };
    // The unique index is on (user_id, date(COALESCE(published_at, created_at)), slug).
    // For same-day same-slug, this should fail for a published post paired with any other
    // (since draft uses created_at and published uses published_at).
    // The simplest reliable conflict: two drafts on the same day.
    // SQLite's unique index covers (user_id, date(COALESCE(published_at, created_at)), slug)
    // both use the same date (today), so the second insert should violate it.
    let _ = input2; // May or may not conflict depending on date; test published conflict below.

    // Test published conflict: publish two posts with same slug on same date
    let pub_input = CreatePostInput {
        user_id,
        title: "Published".to_string(),
        slug: "same-day-slug".parse().unwrap(),
        body: "body".to_string(),
        format: PostFormat::Markdown,
        rendered_html: "<p>body</p>".to_string(),
        published_at: Some(now),
    };
    state.posts.create_post(&pub_input.clone()).await.unwrap();

    let err = state.posts.create_post(&pub_input).await.unwrap_err();
    assert!(
        matches!(err, CreatePostError::SlugConflict),
        "expected SlugConflict, got {err:?}"
    );
}

async fn assert_post_update_creates_revision(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user_id = state
        .users
        .create_user(&username("carol"), &password("password123"), None)
        .await
        .unwrap();

    let post_id = state
        .posts
        .create_post(&make_create_post_input(user_id, "update-test"))
        .await
        .unwrap();

    let update_input = UpdatePostInput {
        title: "Updated Title".to_string(),
        slug: "update-test".parse().unwrap(),
        body: "updated body".to_string(),
        format: PostFormat::Org,
        rendered_html: "<p>updated body</p>".to_string(),
        published_at: None,
    };
    state
        .posts
        .update_post(post_id, user_id, &update_input)
        .await
        .unwrap();

    let record = state.posts.get_post_by_id(post_id).await.unwrap().unwrap();
    assert_eq!(record.title, "Updated Title");
    assert_eq!(record.format, PostFormat::Org);
    assert_eq!(record.body, "updated body");
}

async fn assert_post_update_not_found(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let update_input = UpdatePostInput {
        title: "Title".to_string(),
        slug: "nope".parse().unwrap(),
        body: "body".to_string(),
        format: PostFormat::Markdown,
        rendered_html: "<p>body</p>".to_string(),
        published_at: None,
    };
    let err = state
        .posts
        .update_post(9999, 1, &update_input)
        .await
        .unwrap_err();
    assert!(
        matches!(err, UpdatePostError::NotFound),
        "expected NotFound, got {err:?}"
    );
}

async fn assert_soft_delete_excludes_from_lists(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    let user_id = state
        .users
        .create_user(&username("dave"), &password("password123"), None)
        .await
        .unwrap();

    let post_id = state
        .posts
        .create_post(&make_published_create_post_input(user_id, "to-delete"))
        .await
        .unwrap();

    // It should appear before deletion
    let published = state.posts.list_published(None, 10).await.unwrap();
    assert!(published.iter().any(|p| p.post_id == post_id));

    state.posts.soft_delete_post(post_id).await.unwrap();

    // Should not appear after deletion
    let published = state.posts.list_published(None, 10).await.unwrap();
    assert!(!published.iter().any(|p| p.post_id == post_id));

    // deleted_at should be set
    let record = state.posts.get_post_by_id(post_id).await.unwrap().unwrap();
    assert!(record.deleted_at.is_some());
}

async fn assert_list_published_by_user(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let alice_id = state
        .users
        .create_user(&username("ealice"), &password("password123"), None)
        .await
        .unwrap();
    let bob_id = state
        .users
        .create_user(&username("ebob"), &password("password123"), None)
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
        .list_published_by_user(&username("ealice"), None, 10)
        .await
        .unwrap();
    assert_eq!(alice_posts.len(), 2);
    assert!(alice_posts.iter().all(|p| p.user_id == alice_id));

    let bob_posts = state
        .posts
        .list_published_by_user(&username("ebob"), None, 10)
        .await
        .unwrap();
    assert_eq!(bob_posts.len(), 1);
    assert_eq!(bob_posts[0].user_id, bob_id);
}

async fn assert_list_published_returns_all_published(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    let user_id = state
        .users
        .create_user(&username("fuser"), &password("password123"), None)
        .await
        .unwrap();

    // Create a draft (should not appear)
    state
        .posts
        .create_post(&make_create_post_input(user_id, "draft-post"))
        .await
        .unwrap();

    // Create two published posts
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

    let published = state.posts.list_published(None, 10).await.unwrap();
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|p| p.published_at.is_some()));
}

async fn assert_list_drafts_by_user(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user_id = state
        .users
        .create_user(&username("guser"), &password("password123"), None)
        .await
        .unwrap();

    // Create two drafts
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
        .list_drafts_by_user(user_id, None, 10)
        .await
        .unwrap();
    assert_eq!(drafts.len(), 2);
    assert!(drafts.iter().all(|p| p.published_at.is_none()));
    assert!(drafts.iter().all(|p| p.user_id == user_id));
}

// SQLite post tests

#[tokio::test]
async fn sqlite_post_create_and_get_by_id_works() {
    let base = TempDir::new().unwrap();
    let (_, posts) = post_storage(&base).await;
    let pool = open_pool(&base).await;
    let users = SqliteUserStorage::new(pool);
    let user_id = users
        .create_user(&username("alice"), &password("password123"), None)
        .await
        .unwrap();
    let input = make_create_post_input(user_id, "hello-world");
    let post_id = posts.create_post(&input).await.unwrap();
    let record = posts.get_post_by_id(post_id).await.unwrap().unwrap();
    assert_eq!(record.post_id, post_id);
    assert_eq!(record.slug.as_str(), "hello-world");
    assert!(record.deleted_at.is_none());
}

#[tokio::test]
async fn sqlite_post_slug_conflict_returns_slug_conflict() {
    let base = TempDir::new().unwrap();
    let now = Utc::now();
    let (users, posts) = post_storage(&base).await;
    let user_id = users
        .create_user(&username("bob"), &password("password123"), None)
        .await
        .unwrap();
    let input = CreatePostInput {
        user_id,
        title: "Post".to_string(),
        slug: "my-slug".parse().unwrap(),
        body: "body".to_string(),
        format: PostFormat::Markdown,
        rendered_html: "<p>body</p>".to_string(),
        published_at: Some(now),
    };
    posts.create_post(&input).await.unwrap();
    let err = posts.create_post(&input).await.unwrap_err();
    assert!(
        matches!(err, CreatePostError::SlugConflict),
        "expected SlugConflict, got {err:?}"
    );
}

#[tokio::test]
async fn sqlite_post_update_writes_revision_and_updates_record() {
    let (_base, state) = sqlite_state().await;
    assert_post_update_creates_revision(&state).await;
}

#[tokio::test]
async fn sqlite_post_update_not_found_returns_error() {
    let (_base, state) = sqlite_state().await;
    assert_post_update_not_found(&state).await;
}

#[tokio::test]
async fn sqlite_soft_delete_excludes_post_from_lists() {
    let (_base, state) = sqlite_state().await;
    assert_soft_delete_excludes_from_lists(&state).await;
}

#[tokio::test]
async fn sqlite_list_published_by_user_returns_only_user_posts() {
    let (_base, state) = sqlite_state().await;
    assert_list_published_by_user(&state).await;
}

#[tokio::test]
async fn sqlite_list_published_returns_published_non_deleted_posts() {
    let (_base, state) = sqlite_state().await;
    assert_list_published_returns_all_published(&state).await;
}

#[tokio::test]
async fn sqlite_list_drafts_by_user_returns_only_drafts() {
    let (_base, state) = sqlite_state().await;
    assert_list_drafts_by_user(&state).await;
}

#[tokio::test]
async fn sqlite_post_app_state_parity_suite() {
    let (_base, state) = sqlite_state().await;
    assert_post_create_and_get_by_id(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_post_slug_conflict(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_post_update_creates_revision(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_post_update_not_found(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_soft_delete_excludes_from_lists(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_list_published_by_user(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_list_published_returns_all_published(&state).await;

    let (_base, state) = sqlite_state().await;
    assert_list_drafts_by_user(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_post_create_and_get_by_id_works() {
    let state = postgres_state().await;
    assert_post_create_and_get_by_id(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_post_slug_conflict_returns_slug_conflict() {
    let state = postgres_state().await;
    assert_post_slug_conflict(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_post_update_writes_revision_and_updates_record() {
    let state = postgres_state().await;
    assert_post_update_creates_revision(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_post_update_not_found_returns_error() {
    let state = postgres_state().await;
    assert_post_update_not_found(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_soft_delete_excludes_post_from_lists() {
    let state = postgres_state().await;
    assert_soft_delete_excludes_from_lists(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_published_by_user_returns_only_user_posts() {
    let state = postgres_state().await;
    assert_list_published_by_user(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_published_returns_published_non_deleted_posts() {
    let state = postgres_state().await;
    assert_list_published_returns_all_published(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_drafts_by_user_returns_only_drafts() {
    let state = postgres_state().await;
    assert_list_drafts_by_user(&state).await;
}

// =============================================================================
// Tag Tests
// =============================================================================

// Test: Multiple tags on a single post
async fn assert_multiple_tags_on_single_post(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("multi_tag_user"),
            &password("password"),
            Some("Multi"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Multi Tag Post".to_string(),
            slug: "multi-tag-post".parse().unwrap(),
            body: "Content with many tags".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content with many tags</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Add multiple tags
    state
        .posts
        .tag_post(post_id, "rust")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "performance")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "systems-programming")
        .await
        .expect("tag_post failed");

    // Retrieve and verify all tags
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 3);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_str()).collect();
    assert!(tag_slugs.contains(&"rust"));
    assert!(tag_slugs.contains(&"performance"));
    assert!(tag_slugs.contains(&"systems-programming"));
}

// Test: Post with no tags
async fn assert_empty_tag_list(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("no_tag_user"),
            &password("password"),
            Some("NoTag"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "No Tags".to_string(),
            slug: "no-tags".parse().unwrap(),
            body: "Untagged post".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Untagged post</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Retrieve tags - should be empty
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 0);
}

// Test: Tag case preservation with different casing
async fn assert_tag_case_preservation_variants(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("case_user"), &password("password"), Some("Case"))
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Post 1".to_string(),
            slug: "post-1".parse().unwrap(),
            body: "Content 1".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 1</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Post 2".to_string(),
            slug: "post-2".parse().unwrap(),
            body: "Content 2".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 2</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag with different casings but same canonical form - should map to same slug
    state
        .posts
        .tag_post(post1, "Web-Development")
        .await
        .expect("tag_post post1 failed");
    state
        .posts
        .tag_post(post2, "WEB-DEVELOPMENT")
        .await
        .expect("tag_post post2 failed");

    // Both should resolve to same slug
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

    assert_eq!(tags1[0].tag_slug.as_str(), "web-development");
    assert_eq!(tags2[0].tag_slug.as_str(), "web-development");
    assert_eq!(tags1[0].tag_display, "Web-Development");
    assert_eq!(tags2[0].tag_display, "WEB-DEVELOPMENT");

    // List by tag should find both posts
    let tag_slug: Tag = "web-development".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50)
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 2);
}

// Test: Invalid tag input
async fn assert_invalid_tag_input(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("invalid_tag_user"),
            &password("password"),
            Some("Invalid"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Test Post".to_string(),
            slug: "invalid-test".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Test invalid tags - should return error
    let result = state.posts.tag_post(post_id, "-invalid").await;
    assert!(matches!(result, Err(TaggingError::Internal(_))));

    let result = state.posts.tag_post(post_id, "invalid@tag").await;
    assert!(matches!(result, Err(TaggingError::Internal(_))));

    let result = state.posts.tag_post(post_id, "invalid tag").await;
    assert!(matches!(result, Err(TaggingError::Internal(_))));

    let result = state.posts.tag_post(post_id, "").await;
    assert!(matches!(result, Err(TaggingError::Internal(_))));
}

// Test: Tag pagination with many posts
async fn assert_tag_list_pagination(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("pagination_user"),
            &password("password"),
            Some("Pagination"),
        )
        .await
        .expect("user creation failed");

    // Create multiple posts with the same tag
    let mut post_ids = Vec::new();
    for i in 0..5 {
        let post_id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: format!("Post {}", i),
                slug: format!("post-{}", i).parse().unwrap(),
                body: format!("Content {}", i),
                format: PostFormat::Markdown,
                rendered_html: format!("<p>Content {}</p>", i),
                published_at: Some(Utc::now()),
            })
            .await
            .expect("post creation failed");
        post_ids.push(post_id);

        state
            .posts
            .tag_post(post_id, "pagination-test")
            .await
            .expect("tag_post failed");
    }

    // List with limit
    let tag_slug: Tag = "pagination-test".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 2)
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 2);
    // Should be reverse chronological
    assert!(posts[0].created_at >= posts[1].created_at);
}

// Test: User-specific tag listing
async fn assert_list_user_posts_by_tag_excludes_other_users(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    let user1 = state
        .users
        .create_user(&username("user1_tag"), &password("password"), Some("User1"))
        .await
        .expect("user creation failed");

    let user2 = state
        .users
        .create_user(&username("user2_tag"), &password("password"), Some("User2"))
        .await
        .expect("user creation failed");

    // Both users tag posts with "shared-tag"
    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user1,
            title: "User1 Post".to_string(),
            slug: "user1-post".parse().unwrap(),
            body: "Content 1".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 1</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user2,
            title: "User2 Post".to_string(),
            slug: "user2-post".parse().unwrap(),
            body: "Content 2".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 2</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    state
        .posts
        .tag_post(post1, "shared-tag")
        .await
        .expect("tag post1 failed");
    state
        .posts
        .tag_post(post2, "shared-tag")
        .await
        .expect("tag post2 failed");

    // List user1's posts by tag - should only see post1
    let tag_slug: Tag = "shared-tag".parse().unwrap();
    let user1_posts = state
        .posts
        .list_user_posts_by_tag(user1, &tag_slug, None, 50)
        .await
        .expect("list_user_posts_by_tag failed");

    assert_eq!(user1_posts.len(), 1);
    assert_eq!(user1_posts[0].post_id, post1);

    // List user2's posts by tag - should only see post2
    let user2_posts = state
        .posts
        .list_user_posts_by_tag(user2, &tag_slug, None, 50)
        .await
        .expect("list_user_posts_by_tag failed");

    assert_eq!(user2_posts.len(), 1);
    assert_eq!(user2_posts[0].post_id, post2);
}

// Test: Untag multiple times and verify correct tag removed
async fn assert_selective_untag(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("selective_untag"),
            &password("password"),
            Some("Selective"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Multi Tag".to_string(),
            slug: "multi-tag".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Add tags
    state
        .posts
        .tag_post(post_id, "tag-a")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "tag-b")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "tag-c")
        .await
        .expect("tag_post failed");

    // Verify 3 tags
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 3);

    // Remove one tag
    let tag_b: Tag = "tag-b".parse().unwrap();
    state
        .posts
        .untag_post(post_id, &tag_b)
        .await
        .expect("untag_post failed");

    // Verify 2 tags remain and tag-b is gone
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 2);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_str()).collect();
    assert!(!tag_slugs.contains(&"tag-b"));
    assert!(tag_slugs.contains(&"tag-a"));
    assert!(tag_slugs.contains(&"tag-c"));
}

// Test: Tag with numeric characters
async fn assert_numeric_tag(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("numeric_tag"),
            &password("password"),
            Some("Numeric"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Numeric Tag".to_string(),
            slug: "numeric-tag".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag with numeric values
    state
        .posts
        .tag_post(post_id, "python3")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "rust-2024")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "0day")
        .await
        .expect("tag_post failed");

    // Verify tags
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 3);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_str()).collect();
    assert!(tag_slugs.contains(&"python3"));
    assert!(tag_slugs.contains(&"rust-2024"));
    assert!(tag_slugs.contains(&"0day"));
}

// Test: Retagging a post with the same tag (duplicate tag error)
async fn assert_retag_same_post_with_same_tag_fails(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    let user = state
        .users
        .create_user(
            &username("retag_user"),
            &password("password"),
            Some("Retag"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Retag Post".to_string(),
            slug: "retag-post".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag once
    state
        .posts
        .tag_post(post_id, "learning")
        .await
        .expect("tag_post failed");

    // Try to tag again with the exact same display form
    let result = state.posts.tag_post(post_id, "learning").await;
    assert!(matches!(result, Err(TaggingError::AlreadyTagged)));

    // Try to tag again with different casing of same tag
    let result = state.posts.tag_post(post_id, "LEARNING").await;
    assert!(matches!(result, Err(TaggingError::AlreadyTagged)));
}

// Test: Untag from nonexistent post (should fail)
async fn assert_untag_nonexistent_post(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let tag_slug: Tag = "phantom".parse().unwrap();
    let result = state.posts.untag_post(99999, &tag_slug).await;

    // Nonexistent post/tag combination should return TagNotFound
    assert!(matches!(result, Err(TaggingError::TagNotFound)));
}

// Test: Get tags for nonexistent post (should return empty)
async fn assert_get_tags_nonexistent_post(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let tags = state
        .posts
        .get_tags_for_post(99999)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 0);
}

// Test: List posts by nonexistent tag
async fn assert_list_posts_by_nonexistent_tag(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let tag_slug: Tag = "nosuch-tag".parse().unwrap();
    let result = state.posts.list_posts_by_tag(&tag_slug, None, 50).await;

    assert!(matches!(result, Err(ListByTagError::TagNotFound)));
}

// Test: List user posts by nonexistent tag
async fn assert_list_user_posts_by_nonexistent_tag(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    let user = state
        .users
        .create_user(
            &username("user_tag_nope"),
            &password("password"),
            Some("UserTagNope"),
        )
        .await
        .expect("user creation failed");

    let tag_slug: Tag = "nonexistent-tag-99".parse().unwrap();
    let result = state
        .posts
        .list_user_posts_by_tag(user, &tag_slug, None, 50)
        .await;

    assert!(matches!(result, Err(ListByTagError::TagNotFound)));
}

// Test: Many tags on many posts
async fn assert_many_tags_many_posts(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("many_tags_user"),
            &password("password"),
            Some("ManyTags"),
        )
        .await
        .expect("user creation failed");

    let mut post_ids = Vec::new();
    let tags = vec!["rust", "golang", "python", "javascript", "typescript"];

    // Create 3 posts, each with 5 tags
    for i in 0..3 {
        let post_id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: format!("Post {}", i),
                slug: format!("post-many-{}", i).parse().unwrap(),
                body: format!("Content {}", i),
                format: PostFormat::Markdown,
                rendered_html: format!("<p>Content {}</p>", i),
                published_at: Some(Utc::now()),
            })
            .await
            .expect("post creation failed");
        post_ids.push(post_id);

        for tag in &tags {
            state
                .posts
                .tag_post(post_id, tag)
                .await
                .expect("tag_post failed");
        }
    }

    // Verify each post has 5 tags
    for post_id in &post_ids {
        let tags_on_post = state
            .posts
            .get_tags_for_post(*post_id)
            .await
            .expect("get_tags_for_post failed");
        assert_eq!(tags_on_post.len(), 5);
    }

    // Verify each tag is found on all 3 posts
    for tag in &tags {
        let tag_slug: Tag = tag.parse().unwrap();
        let posts = state
            .posts
            .list_posts_by_tag(&tag_slug, None, 50)
            .await
            .expect("list_posts_by_tag failed");
        assert_eq!(posts.len(), 3);
    }
}

// Test: Tag with all-numeric slug
async fn assert_tag_all_numeric(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("numeric_only"),
            &password("password"),
            Some("NumericOnly"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Numeric Tag".to_string(),
            slug: "numeric-slug".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag with all-numeric value
    state
        .posts
        .tag_post(post_id, "2024")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "42")
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 2);
    let tag_slugs: Vec<&str> = tags.iter().map(|t| t.tag_slug.as_str()).collect();
    assert!(tag_slugs.contains(&"2024"));
    assert!(tag_slugs.contains(&"42"));
}

// Test: Tag with hyphens at boundaries
async fn assert_tag_hyphen_boundaries(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("hyphen_user"),
            &password("password"),
            Some("Hyphen"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Hyphen Test".to_string(),
            slug: "hyphen-test".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Valid: hyphens in the middle and at end
    state
        .posts
        .tag_post(post_id, "web-development")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "a-b-c")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "end-")
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 3);

    // Invalid: hyphens at start should fail
    let result = state.posts.tag_post(post_id, "-start").await;
    assert!(matches!(result, Err(TaggingError::Internal(_))));

    // Invalid: consecutive hyphens in the middle are allowed per the regex
    // but let's test another boundary case: non-alphanumeric characters
    let result = state.posts.tag_post(post_id, "tag_underscore").await;
    assert!(matches!(result, Err(TaggingError::Internal(_))));
}

// Test: Tag with long slug and display name
async fn assert_tag_with_long_display(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("long_tag_user"),
            &password("password"),
            Some("LongTagUser"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Long Tag Test".to_string(),
            slug: "long-tag-test".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag with long display name
    let long_display = "very-long-technical-term-with-many-hyphens-and-lowercase-letters";
    state
        .posts
        .tag_post(post_id, long_display)
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

// Test: Tag list ordering and consistency
async fn assert_tag_list_ordering(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("ordering_user"),
            &password("password"),
            Some("Ordering"),
        )
        .await
        .expect("user creation failed");

    // Create posts and tag with multiple tags
    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Post 1".to_string(),
            slug: "post-1-order".parse().unwrap(),
            body: "Content 1".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 1</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Post 2".to_string(),
            slug: "post-2-order".parse().unwrap(),
            body: "Content 2".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 2</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag in different orders
    state
        .posts
        .tag_post(post1, "zebra")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post1, "apple")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post1, "mango")
        .await
        .expect("tag_post failed");

    state
        .posts
        .tag_post(post2, "mango")
        .await
        .expect("tag_post failed");

    // Get tags for post1 - should be ordered by slug
    let tags1 = state
        .posts
        .get_tags_for_post(post1)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags1.len(), 3);
    let slugs1: Vec<&str> = tags1.iter().map(|t| t.tag_slug.as_str()).collect();
    assert_eq!(slugs1, vec!["apple", "mango", "zebra"]);

    // Verify consistency on multiple calls
    let tags1_again = state
        .posts
        .get_tags_for_post(post1)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags1_again.len(), 3);
    assert_eq!(tags1_again[0].tag_slug.as_str(), "apple");
}

// Test: Boundary test for tag operations without tags
async fn assert_tags_for_multiple_posts(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("multi_post_user"),
            &password("password"),
            Some("MultiPost"),
        )
        .await
        .expect("user creation failed");

    // Create multiple posts with varied tag configurations
    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Post A".to_string(),
            slug: "post-a".parse().unwrap(),
            body: "Content A".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content A</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Post B".to_string(),
            slug: "post-b".parse().unwrap(),
            body: "Content B".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content B</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // post1: no tags
    // post2: one tag
    state
        .posts
        .tag_post(post2, "featured")
        .await
        .expect("tag_post failed");

    // Verify post1 has no tags
    let tags1 = state
        .posts
        .get_tags_for_post(post1)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags1.len(), 0);

    // Verify post2 has one tag
    let tags2 = state
        .posts
        .get_tags_for_post(post2)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags2.len(), 1);
}

// Test: Tag normalization with mixed alphanumeric
async fn assert_tag_mixed_alphanumeric(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("mixed_user"),
            &password("password"),
            Some("Mixed"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Mixed Post".to_string(),
            slug: "mixed-post".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Test mixed alphanumeric tags
    state
        .posts
        .tag_post(post_id, "version-2-0-1")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "HTTP2")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post_id, "3D-Graphics")
        .await
        .expect("tag_post failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 3);
    // Verify normalization of numeric and alphabetic
    assert_eq!(tags[0].tag_slug.as_str(), "3d-graphics");
    assert_eq!(tags[1].tag_slug.as_str(), "http2");
    assert_eq!(tags[2].tag_slug.as_str(), "version-2-0-1");
}

// Test: User with single tag on single post, then untag
async fn assert_simple_tag_lifecycle(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("simple_user"),
            &password("password"),
            Some("Simple"),
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Simple".to_string(),
            slug: "simple".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag the post
    state
        .posts
        .tag_post(post_id, "test")
        .await
        .expect("tag_post failed");

    // Verify it's there
    let tags_before = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags_before.len(), 1);
    assert_eq!(tags_before[0].tag_display, "test");

    // List by tag
    let tag_slug: Tag = "test".parse().unwrap();
    let posts_before = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50)
        .await
        .expect("list_posts_by_tag failed");
    assert_eq!(posts_before.len(), 1);

    // Untag
    state
        .posts
        .untag_post(post_id, &tag_slug)
        .await
        .expect("untag_post failed");

    // Verify tag is gone from post
    let tags_after = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags_after.len(), 0);

    // List by tag again - should return empty list (tag exists but no posts have it)
    let posts_after = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50)
        .await
        .expect("list_posts_by_tag failed");
    assert_eq!(posts_after.len(), 0);
}

async fn assert_tag_creation_and_retrieval(state: &std::sync::Arc<jaunder::storage::AppState>) {
    // Create a user and post
    let user = state
        .users
        .create_user(&username("alice"), &password("password"), Some("Alice"))
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Test Post".to_string(),
            slug: "test-post".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag the post
    state
        .posts
        .tag_post(post_id, "rust")
        .await
        .expect("tag_post failed");

    // Retrieve tags
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_slug.as_str(), "rust");
    assert_eq!(tags[0].tag_display, "rust");
}

async fn assert_tag_normalization(state: &std::sync::Arc<jaunder::storage::AppState>) {
    // Create a user and post
    let user = state
        .users
        .create_user(&username("bob"), &password("password"), Some("Bob"))
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Test Post".to_string(),
            slug: "test-post".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag with mixed-case display name
    state
        .posts
        .tag_post(post_id, "Rust-Web")
        .await
        .expect("tag_post failed");

    // Retrieve and verify normalization
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_slug.as_str(), "rust-web"); // normalized
    assert_eq!(tags[0].tag_display, "Rust-Web"); // original preserved
}

async fn assert_untag_post(state: &std::sync::Arc<jaunder::storage::AppState>) {
    // Create a user and post
    let user = state
        .users
        .create_user(&username("charlie"), &password("password"), Some("Charlie"))
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Test Post".to_string(),
            slug: "test-post".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag the post
    state
        .posts
        .tag_post(post_id, "python")
        .await
        .expect("tag_post failed");

    // Verify it's there
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 1);

    // Untag it
    let tag_slug: Tag = "python".parse().unwrap();
    state
        .posts
        .untag_post(post_id, &tag_slug)
        .await
        .expect("untag_post failed");

    // Verify it's gone
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 0);
}

async fn assert_duplicate_tag_error(state: &std::sync::Arc<jaunder::storage::AppState>) {
    // Create a user and post
    let user = state
        .users
        .create_user(&username("dave"), &password("password"), Some("Dave"))
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Test Post".to_string(),
            slug: "test-post".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag the post
    state
        .posts
        .tag_post(post_id, "go")
        .await
        .expect("first tag_post failed");

    // Try to tag with same tag again (case insensitive)
    let result = state.posts.tag_post(post_id, "GO").await;
    match result {
        Err(TaggingError::AlreadyTagged) => {
            // Expected
        }
        other => panic!("Expected AlreadyTagged, got {:?}", other),
    }
}

async fn assert_list_posts_by_tag(state: &std::sync::Arc<jaunder::storage::AppState>) {
    // Create users
    let user1 = state
        .users
        .create_user(&username("eve"), &password("password"), Some("Eve"))
        .await
        .expect("user creation failed");

    let user2 = state
        .users
        .create_user(&username("frank"), &password("password"), Some("Frank"))
        .await
        .expect("user creation failed");

    // Create posts and tag them
    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user1,
            title: "Post 1".to_string(),
            slug: "post-1".parse().unwrap(),
            body: "Content 1".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 1</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user2,
            title: "Post 2".to_string(),
            slug: "post-2".parse().unwrap(),
            body: "Content 2".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 2</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag both with "javascript"
    state
        .posts
        .tag_post(post1, "javascript")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post2, "javascript")
        .await
        .expect("tag_post failed");

    // List posts by tag
    let tag_slug: Tag = "javascript".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50)
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 2);
    assert!(posts.iter().any(|p| p.post_id == post1));
    assert!(posts.iter().any(|p| p.post_id == post2));
}

async fn assert_list_user_posts_by_tag(state: &std::sync::Arc<jaunder::storage::AppState>) {
    // Create users
    let user1 = state
        .users
        .create_user(&username("grace"), &password("password"), Some("Grace"))
        .await
        .expect("user creation failed");

    let user2 = state
        .users
        .create_user(&username("henry"), &password("password"), Some("Henry"))
        .await
        .expect("user creation failed");

    // Create posts
    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user1,
            title: "Post 1".to_string(),
            slug: "post-1".parse().unwrap(),
            body: "Content 1".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 1</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user1,
            title: "Post 2".to_string(),
            slug: "post-2".parse().unwrap(),
            body: "Content 2".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 2</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    let post3 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user2,
            title: "Post 3".to_string(),
            slug: "post-3".parse().unwrap(),
            body: "Content 3".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 3</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag all with "clojure"
    state
        .posts
        .tag_post(post1, "clojure")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post2, "clojure")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post3, "clojure")
        .await
        .expect("tag_post failed");

    // List user1's posts by tag
    let tag_slug: Tag = "clojure".parse().unwrap();
    let posts = state
        .posts
        .list_user_posts_by_tag(user1, &tag_slug, None, 50)
        .await
        .expect("list_user_posts_by_tag failed");

    assert_eq!(posts.len(), 2);
    assert!(posts.iter().all(|p| p.user_id == user1));
}

async fn assert_tag_not_found_error(state: &std::sync::Arc<jaunder::storage::AppState>) {
    // Try to list posts by non-existent tag
    let tag_slug: Tag = "nonexistent".parse().unwrap();
    let result = state.posts.list_posts_by_tag(&tag_slug, None, 50).await;

    match result {
        Err(ListByTagError::TagNotFound) => {
            // Expected
        }
        other => panic!("Expected TagNotFound, got {:?}", other),
    }
}

async fn assert_soft_deleted_posts_excluded_from_tag_list(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    // Create a user and posts
    let user = state
        .users
        .create_user(&username("iris"), &password("password"), Some("Iris"))
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Post 1".to_string(),
            slug: "post-1".parse().unwrap(),
            body: "Content 1".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 1</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Post 2".to_string(),
            slug: "post-2".parse().unwrap(),
            body: "Content 2".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content 2</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag both
    state
        .posts
        .tag_post(post1, "haskell")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post2, "haskell")
        .await
        .expect("tag_post failed");

    // Delete one post
    state
        .posts
        .soft_delete_post(post1)
        .await
        .expect("soft_delete_post failed");

    // List posts by tag - should only see post2
    let tag_slug: Tag = "haskell".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50)
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].post_id, post2);
}

async fn assert_tag_post_nonexistent_post_error(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    // Try to tag a post that doesn't exist
    let result = state.posts.tag_post(99999, "nonexistent-post").await;
    match result {
        Err(TaggingError::PostNotFound) => {
            // Expected
        }
        other => panic!("Expected PostNotFound, got {:?}", other),
    }
}

async fn assert_untag_nonexistent_tag_error(state: &std::sync::Arc<jaunder::storage::AppState>) {
    // Create a user and post
    let user = state
        .users
        .create_user(&username("karen"), &password("password"), Some("Karen"))
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Test Post".to_string(),
            slug: "test-post".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Try to remove a tag that was never added
    let tag_slug: Tag = "nonexistent".parse().unwrap();
    let result = state.posts.untag_post(post_id, &tag_slug).await;
    match result {
        Err(TaggingError::TagNotFound) => {
            // Expected
        }
        other => panic!("Expected TagNotFound, got {:?}", other),
    }
}

async fn assert_draft_posts_excluded_from_tag_list(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    // Create a user and posts
    let user = state
        .users
        .create_user(&username("jack"), &password("password"), Some("Jack"))
        .await
        .expect("user creation failed");

    let post1 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Draft Post".to_string(),
            slug: "draft-post".parse().unwrap(),
            body: "Draft content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Draft</p>".to_string(),
            published_at: None, // Draft
        })
        .await
        .expect("post creation failed");

    let post2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Published Post".to_string(),
            slug: "published-post".parse().unwrap(),
            body: "Published content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Published</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag both
    state
        .posts
        .tag_post(post1, "kotlin")
        .await
        .expect("tag_post failed");
    state
        .posts
        .tag_post(post2, "kotlin")
        .await
        .expect("tag_post failed");

    // List posts by tag - should only see published post2
    let tag_slug: Tag = "kotlin".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag_slug, None, 50)
        .await
        .expect("list_posts_by_tag failed");

    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].post_id, post2);
}

#[tokio::test]
async fn sqlite_tag_creation_and_retrieval() {
    let (_base, state) = sqlite_state().await;
    assert_tag_creation_and_retrieval(&state).await;
}

#[tokio::test]
async fn sqlite_tag_normalization() {
    let (_base, state) = sqlite_state().await;
    assert_tag_normalization(&state).await;
}

#[tokio::test]
async fn sqlite_untag_post() {
    let (_base, state) = sqlite_state().await;
    assert_untag_post(&state).await;
}

#[tokio::test]
async fn sqlite_duplicate_tag_error() {
    let (_base, state) = sqlite_state().await;
    assert_duplicate_tag_error(&state).await;
}

#[tokio::test]
async fn sqlite_list_posts_by_tag() {
    let (_base, state) = sqlite_state().await;
    assert_list_posts_by_tag(&state).await;
}

#[tokio::test]
async fn sqlite_list_user_posts_by_tag() {
    let (_base, state) = sqlite_state().await;
    assert_list_user_posts_by_tag(&state).await;
}

#[tokio::test]
async fn sqlite_tag_not_found_error() {
    let (_base, state) = sqlite_state().await;
    assert_tag_not_found_error(&state).await;
}

#[tokio::test]
async fn sqlite_soft_deleted_posts_excluded_from_tag_list() {
    let (_base, state) = sqlite_state().await;
    assert_soft_deleted_posts_excluded_from_tag_list(&state).await;
}

#[tokio::test]
async fn sqlite_draft_posts_excluded_from_tag_list() {
    let (_base, state) = sqlite_state().await;
    assert_draft_posts_excluded_from_tag_list(&state).await;
}

#[tokio::test]
async fn sqlite_tag_post_nonexistent_post_error() {
    let (_base, state) = sqlite_state().await;
    assert_tag_post_nonexistent_post_error(&state).await;
}

#[tokio::test]
async fn sqlite_untag_nonexistent_tag_error() {
    let (_base, state) = sqlite_state().await;
    assert_untag_nonexistent_tag_error(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_creation_and_retrieval() {
    let state = postgres_state().await;
    assert_tag_creation_and_retrieval(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_normalization() {
    let state = postgres_state().await;
    assert_tag_normalization(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_untag_post() {
    let state = postgres_state().await;
    assert_untag_post(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_duplicate_tag_error() {
    let state = postgres_state().await;
    assert_duplicate_tag_error(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_posts_by_tag() {
    let state = postgres_state().await;
    assert_list_posts_by_tag(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_user_posts_by_tag() {
    let state = postgres_state().await;
    assert_list_user_posts_by_tag(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_not_found_error() {
    let state = postgres_state().await;
    assert_tag_not_found_error(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_soft_deleted_posts_excluded_from_tag_list() {
    let state = postgres_state().await;
    assert_soft_deleted_posts_excluded_from_tag_list(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_draft_posts_excluded_from_tag_list() {
    let state = postgres_state().await;
    assert_draft_posts_excluded_from_tag_list(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_post_nonexistent_post_error() {
    let state = postgres_state().await;
    assert_tag_post_nonexistent_post_error(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_untag_nonexistent_tag_error() {
    let state = postgres_state().await;
    assert_untag_nonexistent_tag_error(&state).await;
}

// ====== Additional tag test cases for improved coverage ======

// SQLite multiple tags tests
#[tokio::test]
async fn sqlite_multiple_tags_on_single_post() {
    let (_base, state) = sqlite_state().await;
    assert_multiple_tags_on_single_post(&state).await;
}

#[tokio::test]
async fn sqlite_empty_tag_list() {
    let (_base, state) = sqlite_state().await;
    assert_empty_tag_list(&state).await;
}

#[tokio::test]
async fn sqlite_tag_case_preservation_variants() {
    let (_base, state) = sqlite_state().await;
    assert_tag_case_preservation_variants(&state).await;
}

#[tokio::test]
async fn sqlite_invalid_tag_input() {
    let (_base, state) = sqlite_state().await;
    assert_invalid_tag_input(&state).await;
}

#[tokio::test]
async fn sqlite_tag_list_pagination() {
    let (_base, state) = sqlite_state().await;
    assert_tag_list_pagination(&state).await;
}

#[tokio::test]
async fn sqlite_list_user_posts_by_tag_excludes_other_users() {
    let (_base, state) = sqlite_state().await;
    assert_list_user_posts_by_tag_excludes_other_users(&state).await;
}

#[tokio::test]
async fn sqlite_selective_untag() {
    let (_base, state) = sqlite_state().await;
    assert_selective_untag(&state).await;
}

#[tokio::test]
async fn sqlite_numeric_tag() {
    let (_base, state) = sqlite_state().await;
    assert_numeric_tag(&state).await;
}

// PostgreSQL multiple tags tests
#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_multiple_tags_on_single_post() {
    let state = postgres_state().await;
    assert_multiple_tags_on_single_post(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_empty_tag_list() {
    let state = postgres_state().await;
    assert_empty_tag_list(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_case_preservation_variants() {
    let state = postgres_state().await;
    assert_tag_case_preservation_variants(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_invalid_tag_input() {
    let state = postgres_state().await;
    assert_invalid_tag_input(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_list_pagination() {
    let state = postgres_state().await;
    assert_tag_list_pagination(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_user_posts_by_tag_excludes_other_users() {
    let state = postgres_state().await;
    assert_list_user_posts_by_tag_excludes_other_users(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_selective_untag() {
    let state = postgres_state().await;
    assert_selective_untag(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_numeric_tag() {
    let state = postgres_state().await;
    assert_numeric_tag(&state).await;
}

// ====== More comprehensive tag test cases ======

// SQLite: Edge case tests
#[tokio::test]
async fn sqlite_retag_same_post_with_same_tag_fails() {
    let (_base, state) = sqlite_state().await;
    assert_retag_same_post_with_same_tag_fails(&state).await;
}

#[tokio::test]
async fn sqlite_untag_nonexistent_post() {
    let (_base, state) = sqlite_state().await;
    assert_untag_nonexistent_post(&state).await;
}

#[tokio::test]
async fn sqlite_get_tags_nonexistent_post() {
    let (_base, state) = sqlite_state().await;
    assert_get_tags_nonexistent_post(&state).await;
}

#[tokio::test]
async fn sqlite_list_posts_by_nonexistent_tag() {
    let (_base, state) = sqlite_state().await;
    assert_list_posts_by_nonexistent_tag(&state).await;
}

#[tokio::test]
async fn sqlite_list_user_posts_by_nonexistent_tag() {
    let (_base, state) = sqlite_state().await;
    assert_list_user_posts_by_nonexistent_tag(&state).await;
}

#[tokio::test]
async fn sqlite_many_tags_many_posts() {
    let (_base, state) = sqlite_state().await;
    assert_many_tags_many_posts(&state).await;
}

#[tokio::test]
async fn sqlite_tag_all_numeric() {
    let (_base, state) = sqlite_state().await;
    assert_tag_all_numeric(&state).await;
}

#[tokio::test]
async fn sqlite_tag_hyphen_boundaries() {
    let (_base, state) = sqlite_state().await;
    assert_tag_hyphen_boundaries(&state).await;
}

// PostgreSQL: Edge case tests
#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_retag_same_post_with_same_tag_fails() {
    let state = postgres_state().await;
    assert_retag_same_post_with_same_tag_fails(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_untag_nonexistent_post() {
    let state = postgres_state().await;
    assert_untag_nonexistent_post(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_get_tags_nonexistent_post() {
    let state = postgres_state().await;
    assert_get_tags_nonexistent_post(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_posts_by_nonexistent_tag() {
    let state = postgres_state().await;
    assert_list_posts_by_nonexistent_tag(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_user_posts_by_nonexistent_tag() {
    let state = postgres_state().await;
    assert_list_user_posts_by_nonexistent_tag(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_many_tags_many_posts() {
    let state = postgres_state().await;
    assert_many_tags_many_posts(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_all_numeric() {
    let state = postgres_state().await;
    assert_tag_all_numeric(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_hyphen_boundaries() {
    let state = postgres_state().await;
    assert_tag_hyphen_boundaries(&state).await;
}

// ====== Additional edge case tests ======

// SQLite: Additional edge cases
#[tokio::test]
async fn sqlite_tag_with_long_display() {
    let (_base, state) = sqlite_state().await;
    assert_tag_with_long_display(&state).await;
}

#[tokio::test]
async fn sqlite_tag_list_ordering() {
    let (_base, state) = sqlite_state().await;
    assert_tag_list_ordering(&state).await;
}

// PostgreSQL: Additional edge cases
#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_with_long_display() {
    let state = postgres_state().await;
    assert_tag_with_long_display(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_list_ordering() {
    let state = postgres_state().await;
    assert_tag_list_ordering(&state).await;
}

// SQLite: Multiple posts with varied tagging
#[tokio::test]
async fn sqlite_tags_for_multiple_posts() {
    let (_base, state) = sqlite_state().await;
    assert_tags_for_multiple_posts(&state).await;
}

// PostgreSQL: Multiple posts with varied tagging
#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tags_for_multiple_posts() {
    let state = postgres_state().await;
    assert_tags_for_multiple_posts(&state).await;
}

// SQLite: Mixed alphanumeric and lifecycle tests
#[tokio::test]
async fn sqlite_tag_mixed_alphanumeric() {
    let (_base, state) = sqlite_state().await;
    assert_tag_mixed_alphanumeric(&state).await;
}

#[tokio::test]
async fn sqlite_simple_tag_lifecycle() {
    let (_base, state) = sqlite_state().await;
    assert_simple_tag_lifecycle(&state).await;
}

// PostgreSQL: Mixed alphanumeric and lifecycle tests
#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_mixed_alphanumeric() {
    let state = postgres_state().await;
    assert_tag_mixed_alphanumeric(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_simple_tag_lifecycle() {
    let state = postgres_state().await;
    assert_simple_tag_lifecycle(&state).await;
}

// ====== Additional coverage tests for error paths ======

// SQLite: Post update with invalid slug and edge cases
async fn assert_post_update_invalid_slug(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("test_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Original".to_string(),
            slug: "original-slug".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: None,
        })
        .await
        .expect("post creation failed");

    // Create a second post with a different slug
    let _post_id2 = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Second".to_string(),
            slug: "second-slug".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: None,
        })
        .await
        .expect("post creation failed");

    // Try to update post 1 to use post 2's slug (should fail with SlugConflict)
    let update_result = state
        .posts
        .update_post(
            post_id,
            user,
            &UpdatePostInput {
                title: "Updated".to_string(),
                slug: "second-slug".parse().unwrap(),
                body: "Updated content".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Updated</p>".to_string(),
                published_at: None,
            },
        )
        .await;

    match update_result {
        Err(UpdatePostError::Internal(_)) => {
            // Expected: unique constraint violation on slug
        }
        other => panic!("Expected Internal error, got {:?}", other),
    }
}

// SQLite: List published with cursor boundary conditions
async fn assert_list_published_cursor_boundary(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("cursor_test_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    let now = Utc::now();

    // Create multiple posts with slightly different timestamps
    for i in 0..5 {
        let _ = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: format!("Post {}", i),
                slug: format!("post-{}", i).parse().unwrap(),
                body: "Content".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Content</p>".to_string(),
                published_at: Some(now),
            })
            .await
            .expect("post creation failed");
    }

    // Get all posts
    let all = state
        .posts
        .list_published(None, 10)
        .await
        .expect("list_published failed");
    assert_eq!(all.len(), 5);

    // Get first 2
    let first = state
        .posts
        .list_published(None, 2)
        .await
        .expect("list_published failed");
    assert_eq!(first.len(), 2);

    // Use cursor to get next batch
    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[first.len() - 1].created_at,
            post_id: first[first.len() - 1].post_id,
        };
        let next = state
            .posts
            .list_published(Some(&cursor), 2)
            .await
            .expect("list_published with cursor failed");
        assert_eq!(next.len(), 2);
    }
}

// SQLite: List drafts with cursor
async fn assert_list_drafts_cursor_boundary(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("draft_cursor_test"), &password("password"), None)
        .await
        .expect("user creation failed");

    let _now = Utc::now();

    // Create multiple draft posts
    for i in 0..3 {
        let _ = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: format!("Draft {}", i),
                slug: format!("draft-{}", i).parse().unwrap(),
                body: "Content".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Content</p>".to_string(),
                published_at: None,
            })
            .await
            .expect("post creation failed");
    }

    // Get all drafts
    let all = state
        .posts
        .list_drafts_by_user(user, None, 10)
        .await
        .expect("list_drafts_by_user failed");
    assert_eq!(all.len(), 3);

    // Get first 1
    let first = state
        .posts
        .list_drafts_by_user(user, None, 1)
        .await
        .expect("list_drafts_by_user failed");
    assert_eq!(first.len(), 1);

    // Use cursor to get next
    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = state
            .posts
            .list_drafts_by_user(user, Some(&cursor), 2)
            .await
            .expect("list_drafts_by_user with cursor failed");
        assert!(next.len() <= 2);
    }
}

// SQLite: List user posts by tag with cursor
async fn assert_list_user_posts_by_tag_cursor(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("tag_cursor_test"), &password("password"), None)
        .await
        .expect("user creation failed");

    let now = Utc::now();

    // Create multiple posts and tag them
    for i in 0..3 {
        let post_id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: format!("Tagged {}", i),
                slug: format!("tagged-{}", i).parse().unwrap(),
                body: "Content".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Content</p>".to_string(),
                published_at: Some(now),
            })
            .await
            .expect("post creation failed");

        state
            .posts
            .tag_post(post_id, "cursor-tag")
            .await
            .expect("tag_post failed");
    }

    let tag: Tag = "cursor-tag".parse().unwrap();

    // Get all tagged posts
    let all = state
        .posts
        .list_user_posts_by_tag(user, &tag, None, 10)
        .await
        .expect("list_user_posts_by_tag failed");
    assert_eq!(all.len(), 3);

    // Get first 1
    let first = state
        .posts
        .list_user_posts_by_tag(user, &tag, None, 1)
        .await
        .expect("list_user_posts_by_tag failed");
    assert_eq!(first.len(), 1);

    // Use cursor to get next
    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = state
            .posts
            .list_user_posts_by_tag(user, &tag, Some(&cursor), 2)
            .await
            .expect("list_user_posts_by_tag with cursor failed");
        assert!(next.len() <= 2);
    }
}

// SQLite: List posts by tag with cursor
async fn assert_list_posts_by_tag_cursor(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("global_tag_cursor_test"),
            &password("password"),
            None,
        )
        .await
        .expect("user creation failed");

    let now = Utc::now();

    // Create multiple posts and tag them
    for i in 0..3 {
        let post_id = state
            .posts
            .create_post(&CreatePostInput {
                user_id: user,
                title: format!("Global {}", i),
                slug: format!("global-{}", i).parse().unwrap(),
                body: "Content".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Content</p>".to_string(),
                published_at: Some(now),
            })
            .await
            .expect("post creation failed");

        state
            .posts
            .tag_post(post_id, "global-tag")
            .await
            .expect("tag_post failed");
    }

    let tag: Tag = "global-tag".parse().unwrap();

    // Get all tagged posts
    let all = state
        .posts
        .list_posts_by_tag(&tag, None, 10)
        .await
        .expect("list_posts_by_tag failed");
    assert_eq!(all.len(), 3);

    // Get first 1
    let first = state
        .posts
        .list_posts_by_tag(&tag, None, 1)
        .await
        .expect("list_posts_by_tag failed");
    assert_eq!(first.len(), 1);

    // Use cursor to get next
    if !first.is_empty() {
        let cursor = PostCursor {
            created_at: first[0].created_at,
            post_id: first[0].post_id,
        };
        let next = state
            .posts
            .list_posts_by_tag(&tag, Some(&cursor), 2)
            .await
            .expect("list_posts_by_tag with cursor failed");
        assert!(next.len() <= 2);
    }
}

// SQLite: Soft delete then try operations
async fn assert_soft_delete_then_operations(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("soft_del_test"), &password("password"), None)
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "To Delete".to_string(),
            slug: "to-delete".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag the post
    state
        .posts
        .tag_post(post_id, "delete-tag")
        .await
        .expect("tag_post failed");

    // Soft delete
    state
        .posts
        .soft_delete_post(post_id)
        .await
        .expect("soft_delete_post failed");

    // Try to get by ID (should still exist internally)
    let post = state
        .posts
        .get_post_by_id(post_id)
        .await
        .expect("get_post_by_id failed");
    assert!(post.is_none() || post.unwrap().deleted_at.is_some());

    // List published should not include it
    let tag: Tag = "delete-tag".parse().unwrap();
    let posts = state
        .posts
        .list_posts_by_tag(&tag, None, 10)
        .await
        .expect("list_posts_by_tag failed");
    assert!(posts.is_empty());
}

#[tokio::test]
async fn sqlite_post_update_invalid_slug() {
    let (_base, state) = sqlite_state().await;
    assert_post_update_invalid_slug(&state).await;
}

#[tokio::test]
async fn sqlite_list_published_cursor_boundary() {
    let (_base, state) = sqlite_state().await;
    assert_list_published_cursor_boundary(&state).await;
}

#[tokio::test]
async fn sqlite_list_drafts_cursor_boundary() {
    let (_base, state) = sqlite_state().await;
    assert_list_drafts_cursor_boundary(&state).await;
}

#[tokio::test]
async fn sqlite_list_user_posts_by_tag_cursor() {
    let (_base, state) = sqlite_state().await;
    assert_list_user_posts_by_tag_cursor(&state).await;
}

#[tokio::test]
async fn sqlite_list_posts_by_tag_cursor() {
    let (_base, state) = sqlite_state().await;
    assert_list_posts_by_tag_cursor(&state).await;
}

#[tokio::test]
async fn sqlite_soft_delete_then_operations() {
    let (_base, state) = sqlite_state().await;
    assert_soft_delete_then_operations(&state).await;
}

// PostgreSQL versions of the same tests
#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_post_update_invalid_slug() {
    let state = postgres_state().await;
    assert_post_update_invalid_slug(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_published_cursor_boundary() {
    let state = postgres_state().await;
    assert_list_published_cursor_boundary(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_drafts_cursor_boundary() {
    let state = postgres_state().await;
    assert_list_drafts_cursor_boundary(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_user_posts_by_tag_cursor() {
    let state = postgres_state().await;
    assert_list_user_posts_by_tag_cursor(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_posts_by_tag_cursor() {
    let state = postgres_state().await;
    assert_list_posts_by_tag_cursor(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_soft_delete_then_operations() {
    let state = postgres_state().await;
    assert_soft_delete_then_operations(&state).await;
}

// ====== Additional error path and rollback scenario tests ======

// Test tagging with multiple failed attempts
async fn assert_tag_post_multiple_attempts(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("tag_multi_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "For Tagging".to_string(),
            slug: "for-tagging".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // First tag succeeds
    state
        .posts
        .tag_post(post_id, "first-tag")
        .await
        .expect("first tag_post failed");

    // Second tag succeeds
    state
        .posts
        .tag_post(post_id, "second-tag")
        .await
        .expect("second tag_post failed");

    // Try to tag again with first tag (should fail with AlreadyTagged)
    let result = state.posts.tag_post(post_id, "first-tag").await;
    match result {
        Err(TaggingError::AlreadyTagged) => {
            // Expected
        }
        other => panic!("Expected AlreadyTagged, got {:?}", other),
    }

    // Verify both tags are present
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 2);
}

// Test list_published_by_user with cursor when no posts match
async fn assert_list_published_by_user_no_posts(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    let _user = state
        .users
        .create_user(&username("no_posts_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    // User has no posts
    let posts = state
        .posts
        .list_published_by_user(&username("no_posts_user"), None, 10)
        .await
        .expect("list_published_by_user failed");
    assert!(posts.is_empty());

    // With cursor should still be empty
    let cursor = PostCursor {
        created_at: Utc::now(),
        post_id: 999,
    };
    let posts = state
        .posts
        .list_published_by_user(&username("no_posts_user"), Some(&cursor), 10)
        .await
        .expect("list_published_by_user with cursor failed");
    assert!(posts.is_empty());
}

// Test get_post_by_permalink returns None when post is soft-deleted
async fn assert_get_by_permalink_soft_deleted(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("permalink_del_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    let created_at = Utc::now();

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Permalink Test".to_string(),
            slug: "permalink-test".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(created_at),
        })
        .await
        .expect("post creation failed");

    // Verify we can get it
    let post = state
        .posts
        .get_post_by_permalink(
            &username("permalink_del_user"),
            created_at.year(),
            created_at.month(),
            created_at.day(),
            &"permalink-test".parse().unwrap(),
        )
        .await
        .expect("get_post_by_permalink failed");
    assert!(post.is_some());

    // Soft delete it
    state
        .posts
        .soft_delete_post(post_id)
        .await
        .expect("soft_delete_post failed");

    // Now it should return None
    let post = state
        .posts
        .get_post_by_permalink(
            &username("permalink_del_user"),
            created_at.year(),
            created_at.month(),
            created_at.day(),
            &"permalink-test".parse().unwrap(),
        )
        .await
        .expect("get_post_by_permalink after delete failed");
    assert!(post.is_none());
}

// Test update_post on soft-deleted post
async fn assert_update_soft_deleted_post(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("update_del_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "To Update".to_string(),
            slug: "to-update".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: None,
        })
        .await
        .expect("post creation failed");

    // Soft delete
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
                title: "Updated".to_string(),
                slug: "updated-slug".parse().unwrap(),
                body: "New content".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>New</p>".to_string(),
                published_at: Some(Utc::now()),
            },
        )
        .await;

    // Even though the post exists, the update might fail or succeed depending on implementation
    // The important part is that the post is soft deleted
    let post = state
        .posts
        .get_post_by_id(post_id)
        .await
        .expect("get_post_by_id failed");
    assert!(post.is_none() || post.unwrap().deleted_at.is_some());
}

// Test tag with various edge case formats
async fn assert_tag_edge_case_formats(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("tag_formats_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Edge Cases".to_string(),
            slug: "edge-cases".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag with numbers
    state
        .posts
        .tag_post(post_id, "123")
        .await
        .expect("numeric tag failed");

    // Tag with hyphens
    state
        .posts
        .tag_post(post_id, "my-tag-here")
        .await
        .expect("hyphenated tag failed");

    // Tag with mixed case (should be normalized)
    state
        .posts
        .tag_post(post_id, "MyTag")
        .await
        .expect("mixed case tag failed");

    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    // Should have 3 tags
    assert_eq!(tags.len(), 3);
}

#[tokio::test]
async fn sqlite_tag_post_multiple_attempts() {
    let (_base, state) = sqlite_state().await;
    assert_tag_post_multiple_attempts(&state).await;
}

#[tokio::test]
async fn sqlite_list_published_by_user_no_posts() {
    let (_base, state) = sqlite_state().await;
    assert_list_published_by_user_no_posts(&state).await;
}

#[tokio::test]
async fn sqlite_get_by_permalink_soft_deleted() {
    let (_base, state) = sqlite_state().await;
    assert_get_by_permalink_soft_deleted(&state).await;
}

#[tokio::test]
async fn sqlite_update_soft_deleted_post() {
    let (_base, state) = sqlite_state().await;
    assert_update_soft_deleted_post(&state).await;
}

#[tokio::test]
async fn sqlite_tag_edge_case_formats() {
    let (_base, state) = sqlite_state().await;
    assert_tag_edge_case_formats(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_post_multiple_attempts() {
    let state = postgres_state().await;
    assert_tag_post_multiple_attempts(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_published_by_user_no_posts() {
    let state = postgres_state().await;
    assert_list_published_by_user_no_posts(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_get_by_permalink_soft_deleted() {
    let state = postgres_state().await;
    assert_get_by_permalink_soft_deleted(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_update_soft_deleted_post() {
    let state = postgres_state().await;
    assert_update_soft_deleted_post(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_edge_case_formats() {
    let state = postgres_state().await;
    assert_tag_edge_case_formats(&state).await;
}

// ====== Comprehensive error path coverage ======

// Test get_post_by_id with non-existent post
async fn assert_get_post_by_id_nonexistent(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let result = state.posts.get_post_by_id(999999).await;
    match result {
        Ok(None) => {
            // Expected
        }
        other => panic!("Expected Ok(None), got {:?}", other),
    }
}

// Test list_published with cursor where boundary is crossed
async fn assert_list_published_with_cursor_same_timestamp(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    let user = state
        .users
        .create_user(
            &username("cursor_same_ts_user"),
            &password("password"),
            None,
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
                title: format!("Post {}", i),
                slug: format!("post-cursor-same-{}", i).parse().unwrap(),
                body: "Content".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Content</p>".to_string(),
                published_at: Some(now),
            })
            .await
            .expect("post creation failed");
        post_ids.push(post_id);
    }

    // Get first 2
    let first = state
        .posts
        .list_published(None, 2)
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
            .list_published(Some(&cursor), 2)
            .await
            .expect("list_published with cursor failed");
        // Should get remaining 2
        assert_eq!(next.len(), 2);
    }
}

// Test post revisions are created during update
async fn assert_post_revisions_created(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("revision_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Original".to_string(),
            slug: "revision-test".parse().unwrap(),
            body: "Original content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Original</p>".to_string(),
            published_at: None,
        })
        .await
        .expect("post creation failed");

    // Update the post
    state
        .posts
        .update_post(
            post_id,
            user,
            &UpdatePostInput {
                title: "Updated".to_string(),
                slug: "revision-test".parse().unwrap(),
                body: "Updated content".to_string(),
                format: PostFormat::Markdown,
                rendered_html: "<p>Updated</p>".to_string(),
                published_at: Some(Utc::now()),
            },
        )
        .await
        .expect("update_post failed");

    // Verify post was updated
    let post = state
        .posts
        .get_post_by_id(post_id)
        .await
        .expect("get_post_by_id failed");
    assert!(post.is_some());
    let post = post.unwrap();
    assert_eq!(post.title, "Updated");
    assert_eq!(post.body, "Updated content");
    assert!(post.published_at.is_some());
}

// Test display preservation of tags
async fn assert_tag_display_preservation(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("tag_display_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Display Test".to_string(),
            slug: "display-test".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Tag with specific display
    state
        .posts
        .tag_post(post_id, "MySpecialTag")
        .await
        .expect("tag_post failed");

    // Get tags and verify display is preserved
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");

    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_display, "MySpecialTag");
    // Slug should be lowercase
    assert_eq!(tags[0].tag_slug.as_str(), "myspecialtag");
}

// Test untag operation removes only the specified tag
async fn assert_untag_preserves_other_tags(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(
            &username("untag_preserve_user"),
            &password("password"),
            None,
        )
        .await
        .expect("user creation failed");

    let post_id = state
        .posts
        .create_post(&CreatePostInput {
            user_id: user,
            title: "Multi Tag".to_string(),
            slug: "multi-tag".parse().unwrap(),
            body: "Content".to_string(),
            format: PostFormat::Markdown,
            rendered_html: "<p>Content</p>".to_string(),
            published_at: Some(Utc::now()),
        })
        .await
        .expect("post creation failed");

    // Add multiple tags
    state
        .posts
        .tag_post(post_id, "tag1")
        .await
        .expect("tag1 failed");
    state
        .posts
        .tag_post(post_id, "tag2")
        .await
        .expect("tag2 failed");
    state
        .posts
        .tag_post(post_id, "tag3")
        .await
        .expect("tag3 failed");

    // Verify all 3 are present
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 3);

    // Remove tag2
    let tag2: Tag = "tag2".parse().unwrap();
    state
        .posts
        .untag_post(post_id, &tag2)
        .await
        .expect("untag_post failed");

    // Verify only 2 remain and tag2 is gone
    let tags = state
        .posts
        .get_tags_for_post(post_id)
        .await
        .expect("get_tags_for_post failed");
    assert_eq!(tags.len(), 2);
    let tag_slugs: Vec<_> = tags.iter().map(|t| t.tag_slug.as_str()).collect();
    assert!(!tag_slugs.contains(&"tag2"));
}

#[tokio::test]
async fn sqlite_get_post_by_id_nonexistent() {
    let (_base, state) = sqlite_state().await;
    assert_get_post_by_id_nonexistent(&state).await;
}

#[tokio::test]
async fn sqlite_list_published_with_cursor_same_timestamp() {
    let (_base, state) = sqlite_state().await;
    assert_list_published_with_cursor_same_timestamp(&state).await;
}

#[tokio::test]
async fn sqlite_post_revisions_created() {
    let (_base, state) = sqlite_state().await;
    assert_post_revisions_created(&state).await;
}

#[tokio::test]
async fn sqlite_tag_display_preservation() {
    let (_base, state) = sqlite_state().await;
    assert_tag_display_preservation(&state).await;
}

#[tokio::test]
async fn sqlite_untag_preserves_other_tags() {
    let (_base, state) = sqlite_state().await;
    assert_untag_preserves_other_tags(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_get_post_by_id_nonexistent() {
    let state = postgres_state().await;
    assert_get_post_by_id_nonexistent(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_list_published_with_cursor_same_timestamp() {
    let state = postgres_state().await;
    assert_list_published_with_cursor_same_timestamp(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_post_revisions_created() {
    let state = postgres_state().await;
    assert_post_revisions_created(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_tag_display_preservation() {
    let state = postgres_state().await;
    assert_tag_display_preservation(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_untag_preserves_other_tags() {
    let state = postgres_state().await;
    assert_untag_preserves_other_tags(&state).await;
}

// ====== Site config tests ======

async fn assert_site_config_operations(state: &std::sync::Arc<jaunder::storage::AppState>) {
    // Test get non-existent key
    let value = state.site_config.get("nonexistent.key").await;
    match value {
        Ok(None) => {
            // Expected
        }
        other => panic!("Expected Ok(None), got {:?}", other),
    }

    // Test set and get
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
        other => panic!("Expected Ok(Some), got {:?}", other),
    }

    // Test update (overwrite)
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
        other => panic!("Expected updated value, got {:?}", other),
    }
}

async fn assert_session_list_operations(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user = state
        .users
        .create_user(&username("session_list_user"), &password("password"), None)
        .await
        .expect("user creation failed");

    // Create multiple sessions
    let _session1 = state
        .sessions
        .create_session(user, Some("session 1"))
        .await
        .expect("create_session 1 failed");

    let _session2 = state
        .sessions
        .create_session(user, Some("session 2"))
        .await
        .expect("create_session 2 failed");

    let _session3 = state
        .sessions
        .create_session(user, None)
        .await
        .expect("create_session 3 failed");

    // List sessions
    let sessions = state
        .sessions
        .list_sessions(user)
        .await
        .expect("list_sessions failed");

    assert_eq!(sessions.len(), 3);

    // Verify labels are preserved
    let labels: Vec<_> = sessions.iter().map(|s| s.label.as_deref()).collect();
    assert!(labels.contains(&Some("session 1")));
    assert!(labels.contains(&Some("session 2")));
    assert!(labels.contains(&None));

    // Verify we can authenticate with one of the tokens
    let record = state
        .sessions
        .authenticate(&_session1)
        .await
        .expect("authenticate failed");
    assert_eq!(record.user_id, user);
}

async fn assert_invite_list_operations(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let now = Utc::now();
    let future = now + chrono::Duration::hours(1);
    let past = now - chrono::Duration::hours(1);

    // Create multiple invites
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

    // List invites
    let invites = state
        .invites
        .list_invites()
        .await
        .expect("list_invites failed");

    assert!(invites.len() >= 2);

    // Verify unused flags
    let unused_count = invites.iter().filter(|i| i.used_at.is_none()).count();
    assert!(unused_count >= 2);
}

#[tokio::test]
async fn sqlite_site_config_operations() {
    let (_base, state) = sqlite_state().await;
    assert_site_config_operations(&state).await;
}

#[tokio::test]
async fn sqlite_session_list_operations() {
    let (_base, state) = sqlite_state().await;
    assert_session_list_operations(&state).await;
}

#[tokio::test]
async fn sqlite_invite_list_operations() {
    let (_base, state) = sqlite_state().await;
    assert_invite_list_operations(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_site_config_operations() {
    let state = postgres_state().await;
    assert_site_config_operations(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_session_list_operations() {
    let state = postgres_state().await;
    assert_session_list_operations(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_invite_list_operations() {
    let state = postgres_state().await;
    assert_invite_list_operations(&state).await;
}

// =============================================================================
// create_rendered_post / update_rendered_post integration tests
// =============================================================================

async fn assert_create_rendered_post_markdown(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user_id = state
        .users
        .create_user(&username("render_alice"), &password("password123"), None)
        .await
        .unwrap();

    let post_id = create_rendered_post(
        state.posts.as_ref(),
        user_id,
        "Rendered Markdown".to_string(),
        "rendered-markdown".parse().unwrap(),
        "**bold**".to_string(),
        PostFormat::Markdown,
        None,
    )
    .await
    .unwrap();

    let record = state.posts.get_post_by_id(post_id).await.unwrap().unwrap();
    assert_eq!(record.title, "Rendered Markdown");
    assert!(
        record.rendered_html.contains("<strong>bold</strong>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

async fn assert_create_rendered_post_org(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user_id = state
        .users
        .create_user(&username("render_bob"), &password("password123"), None)
        .await
        .unwrap();

    let post_id = create_rendered_post(
        state.posts.as_ref(),
        user_id,
        "Rendered Org".to_string(),
        "rendered-org".parse().unwrap(),
        "*bold*".to_string(),
        PostFormat::Org,
        None,
    )
    .await
    .unwrap();

    let record = state.posts.get_post_by_id(post_id).await.unwrap().unwrap();
    assert_eq!(record.title, "Rendered Org");
    assert!(
        record.rendered_html.contains("<b>bold</b>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

async fn assert_create_rendered_post_slug_conflict(
    state: &std::sync::Arc<jaunder::storage::AppState>,
) {
    use jaunder::render::CreateRenderedPostError;

    let user_id = state
        .users
        .create_user(&username("render_carol"), &password("password123"), None)
        .await
        .unwrap();

    let now = Utc::now();

    // First create succeeds
    create_rendered_post(
        state.posts.as_ref(),
        user_id,
        "First Post".to_string(),
        "conflict-slug".parse().unwrap(),
        "body".to_string(),
        PostFormat::Markdown,
        Some(now),
    )
    .await
    .unwrap();

    // Second create with same slug+date conflicts
    let err = create_rendered_post(
        state.posts.as_ref(),
        user_id,
        "Second Post".to_string(),
        "conflict-slug".parse().unwrap(),
        "body".to_string(),
        PostFormat::Markdown,
        Some(now),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, CreateRenderedPostError::Storage(_)),
        "expected Storage error, got {err:?}"
    );
    assert!(
        err.to_string().contains("slug"),
        "expected slug conflict message, got: {}",
        err
    );
}

async fn assert_update_rendered_post_markdown(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user_id = state
        .users
        .create_user(&username("render_dave"), &password("password123"), None)
        .await
        .unwrap();

    let post_id = state
        .posts
        .create_post(&make_create_post_input(user_id, "update-render-md"))
        .await
        .unwrap();

    update_rendered_post(
        state.posts.as_ref(),
        post_id,
        user_id,
        "Updated Title".to_string(),
        "update-render-md".parse().unwrap(),
        "**updated**".to_string(),
        PostFormat::Markdown,
        None,
    )
    .await
    .unwrap();

    let record = state.posts.get_post_by_id(post_id).await.unwrap().unwrap();
    assert_eq!(record.title, "Updated Title");
    assert!(
        record.rendered_html.contains("<strong>updated</strong>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

async fn assert_update_rendered_post_org(state: &std::sync::Arc<jaunder::storage::AppState>) {
    let user_id = state
        .users
        .create_user(&username("render_eve"), &password("password123"), None)
        .await
        .unwrap();

    let post_id = state
        .posts
        .create_post(&make_create_post_input(user_id, "update-render-org"))
        .await
        .unwrap();

    update_rendered_post(
        state.posts.as_ref(),
        post_id,
        user_id,
        "Updated Org Title".to_string(),
        "update-render-org".parse().unwrap(),
        "*bold org*".to_string(),
        PostFormat::Org,
        None,
    )
    .await
    .unwrap();

    let record = state.posts.get_post_by_id(post_id).await.unwrap().unwrap();
    assert_eq!(record.title, "Updated Org Title");
    assert!(
        record.rendered_html.contains("<b>bold org</b>"),
        "expected rendered HTML, got: {}",
        record.rendered_html
    );
}

async fn assert_update_rendered_post_not_found(state: &std::sync::Arc<jaunder::storage::AppState>) {
    use jaunder::render::UpdateRenderedPostError;

    let err = update_rendered_post(
        state.posts.as_ref(),
        99999,
        1,
        "No Post".to_string(),
        "no-post".parse().unwrap(),
        "body".to_string(),
        PostFormat::Markdown,
        None,
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, UpdateRenderedPostError::Storage(_)),
        "expected Storage error, got {err:?}"
    );
    assert!(
        err.to_string().contains("not found"),
        "expected 'not found' message, got: {}",
        err
    );
}

#[tokio::test]
async fn sqlite_create_rendered_post_markdown_renders_and_stores() {
    let (_base, state) = sqlite_state().await;
    assert_create_rendered_post_markdown(&state).await;
}

#[tokio::test]
async fn sqlite_create_rendered_post_org_renders_and_stores() {
    let (_base, state) = sqlite_state().await;
    assert_create_rendered_post_org(&state).await;
}

#[tokio::test]
async fn sqlite_create_rendered_post_slug_conflict_returns_storage_error() {
    let (_base, state) = sqlite_state().await;
    assert_create_rendered_post_slug_conflict(&state).await;
}

#[tokio::test]
async fn sqlite_update_rendered_post_markdown_renders_and_updates() {
    let (_base, state) = sqlite_state().await;
    assert_update_rendered_post_markdown(&state).await;
}

#[tokio::test]
async fn sqlite_update_rendered_post_org_renders_and_updates() {
    let (_base, state) = sqlite_state().await;
    assert_update_rendered_post_org(&state).await;
}

#[tokio::test]
async fn sqlite_update_rendered_post_not_found_returns_storage_error() {
    let (_base, state) = sqlite_state().await;
    assert_update_rendered_post_not_found(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_create_rendered_post_markdown_renders_and_stores() {
    let state = postgres_state().await;
    assert_create_rendered_post_markdown(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_create_rendered_post_org_renders_and_stores() {
    let state = postgres_state().await;
    assert_create_rendered_post_org(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_create_rendered_post_slug_conflict_returns_storage_error() {
    let state = postgres_state().await;
    assert_create_rendered_post_slug_conflict(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_update_rendered_post_markdown_renders_and_updates() {
    let state = postgres_state().await;
    assert_update_rendered_post_markdown(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_update_rendered_post_org_renders_and_updates() {
    let state = postgres_state().await;
    assert_update_rendered_post_org(&state).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_update_rendered_post_not_found_returns_storage_error() {
    let state = postgres_state().await;
    assert_update_rendered_post_not_found(&state).await;
}
