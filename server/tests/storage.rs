use server::password::Password;
use server::storage::{
    open_database, CreateUserError, DbConnectOptions, ProfileUpdate, SqliteUserStorage,
    UserAuthError, UserStorage,
};
use server::username::Username;
use sqlx::SqlitePool;
use tempfile::TempDir;

fn sqlite_url(base: &TempDir) -> DbConnectOptions {
    format!("sqlite:{}", base.path().join("jaunder.db").display())
        .parse()
        .unwrap()
}

async fn user_storage(base: &TempDir) -> SqliteUserStorage {
    let opts: sqlx::sqlite::SqliteConnectOptions =
        format!("sqlite:{}", base.path().join("jaunder.db").display())
            .parse()
            .unwrap();
    let pool = SqlitePool::connect_with(opts.create_if_missing(true))
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    SqliteUserStorage::new(pool)
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
    let db = open_database(&sqlite_url(&base)).await.unwrap();

    db.set("site.name", "Test Site").await.unwrap();

    assert_eq!(
        db.get("site.name").await.unwrap().as_deref(),
        Some("Test Site")
    );
}

#[tokio::test]
async fn get_missing_key_returns_none() {
    let base = TempDir::new().unwrap();
    let db = open_database(&sqlite_url(&base)).await.unwrap();

    assert!(db.get("nonexistent").await.unwrap().is_none());
}

#[tokio::test]
async fn set_overwrites_existing_value() {
    let base = TempDir::new().unwrap();
    let db = open_database(&sqlite_url(&base)).await.unwrap();

    db.set("site.name", "First").await.unwrap();
    db.set("site.name", "Second").await.unwrap();

    assert_eq!(
        db.get("site.name").await.unwrap().as_deref(),
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
fn non_sqlite_url_is_rejected_at_parse_time() {
    let result = "postgres://localhost/test".parse::<DbConnectOptions>();
    assert!(result.is_err());
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
