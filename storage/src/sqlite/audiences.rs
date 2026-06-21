use sqlx::Sqlite;

use crate::audiences::{AudienceDialect, AudienceStore};

/// SQLite-backed audience storage.
pub type SqliteAudienceStorage = AudienceStore<Sqlite>;

impl AudienceDialect for Sqlite {
    const INSERT_AUDIENCE: &'static str =
        "INSERT INTO audiences (author_user_id, name) VALUES (?, ?) RETURNING audience_id";

    const RENAME_AUDIENCE: &'static str =
        "UPDATE audiences SET name = ? WHERE author_user_id = ? AND audience_id = ? \
         RETURNING audience_id";

    const DELETE_AUDIENCE_MEMBERS: &'static str =
        "DELETE FROM audience_members WHERE author_user_id = ? AND audience_id = ?";

    const DELETE_AUDIENCE: &'static str =
        "DELETE FROM audiences WHERE author_user_id = ? AND audience_id = ?";

    const LIST_AUDIENCES: &'static str = "SELECT audience_id, name, created_at FROM audiences \
         WHERE author_user_id = ? ORDER BY audience_id";

    const INSERT_MEMBER: &'static str =
        "INSERT INTO audience_members (audience_id, subscription_id, author_user_id) \
         VALUES (?, ?, ?) \
         ON CONFLICT (audience_id, subscription_id) DO NOTHING";

    const DELETE_MEMBER: &'static str =
        "DELETE FROM audience_members WHERE audience_id = ? AND subscription_id = ?";

    const LIST_MEMBERS: &'static str =
        "SELECT subscription_id FROM audience_members WHERE audience_id = ? \
         ORDER BY subscription_id";
}
