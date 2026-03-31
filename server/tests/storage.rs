use server::storage::{open_database, DbConnectOptions};
use tempfile::TempDir;

fn sqlite_url(base: &TempDir) -> DbConnectOptions {
    format!("sqlite:{}", base.path().join("jaunder.db").display())
        .parse()
        .unwrap()
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
