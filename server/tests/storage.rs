use chrono::Utc;
use jaunder::password::Password;
use jaunder::storage::{
    open_database, open_existing_database, AtomicOps, CreateUserError, DbConnectOptions,
    EmailVerificationStorage, InviteStorage, PasswordResetStorage, ProfileUpdate,
    RegisterWithInviteError, SessionAuthError, SessionStorage, SqliteAtomicOps,
    SqliteEmailVerificationStorage, SqliteInviteStorage, SqlitePasswordResetStorage,
    SqliteSessionStorage, SqliteUserStorage, UseEmailVerificationError, UseInviteError,
    UsePasswordResetError, UserAuthError, UserStorage,
};
use jaunder::username::Username;
use sqlx::PgPool;
use sqlx::SqlitePool;
use tempfile::TempDir;

fn sqlite_url(base: &TempDir) -> DbConnectOptions {
    format!("sqlite:{}", base.path().join("jaunder.db").display())
        .parse()
        .unwrap()
}

async fn open_pool(base: &TempDir) -> SqlitePool {
    let opts: sqlx::sqlite::SqliteConnectOptions =
        format!("sqlite:{}", base.path().join("jaunder.db").display())
            .parse()
            .unwrap();
    let pool = SqlitePool::connect_with(opts.create_if_missing(true))
        .await
        .unwrap();
    sqlx::migrate!("./migrations/sqlite")
        .run(&pool)
        .await
        .unwrap();
    pool
}

fn postgres_url() -> DbConnectOptions {
    std::env::var("JAUNDER_PG_TEST_URL")
        .unwrap_or_else(|_| "postgres://jaunder@127.0.0.1:55432/jaunder".to_owned())
        .parse()
        .unwrap()
}

async fn reset_postgres_schema() {
    let DbConnectOptions::Postgres { options, .. } = postgres_url() else {
        panic!("expected postgres options");
    };
    let pool = PgPool::connect_with(options).await.unwrap();
    sqlx::query("DROP SCHEMA public CASCADE")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
}

async fn postgres_state() -> std::sync::Arc<jaunder::storage::AppState> {
    reset_postgres_schema().await;
    open_database(&postgres_url()).await.unwrap()
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

#[tokio::test]
async fn set_then_get_roundtrips() {
    let base = TempDir::new().unwrap();
    let state = open_database(&sqlite_url(&base)).await.unwrap();

    state
        .site_config
        .set("site.name", "Test Site")
        .await
        .unwrap();

    assert_eq!(
        state.site_config.get("site.name").await.unwrap().as_deref(),
        Some("Test Site")
    );
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
    assert!(state.site_config.get("missing").await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn open_existing_database_runs_postgres_migrations_on_unmigrated_db() {
    reset_postgres_schema().await;
    let state = open_existing_database(&postgres_url()).await.unwrap();
    assert!(state.site_config.get("missing").await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_site_config_set_then_get_roundtrips() {
    let state = postgres_state().await;
    state
        .site_config
        .set("site.name", "Postgres")
        .await
        .unwrap();
    assert_eq!(
        state.site_config.get("site.name").await.unwrap().as_deref(),
        Some("Postgres")
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_create_user_duplicate_and_authenticate_work() {
    let state = postgres_state().await;
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

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_session_lifecycle_works() {
    let state = postgres_state().await;
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

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_invite_and_atomic_registration_work() {
    let state = postgres_state().await;
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

#[tokio::test]
#[ignore = "requires PostgreSQL test VM"]
async fn postgres_email_verification_and_password_reset_work() {
    let state = postgres_state().await;
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
