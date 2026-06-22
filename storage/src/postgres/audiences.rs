use sqlx::Postgres;

use crate::audiences::{AudienceDialect, AudienceStore};

/// Postgres-backed audience storage.
pub type PostgresAudienceStorage = AudienceStore<Postgres>;

impl AudienceDialect for Postgres {
    const INSERT_AUDIENCE: &'static str =
        "INSERT INTO audiences (author_user_id, name) VALUES ($1, $2) RETURNING audience_id";

    const RENAME_AUDIENCE: &'static str =
        "UPDATE audiences SET name = $1 WHERE author_user_id = $2 AND audience_id = $3 \
         RETURNING audience_id";

    const DELETE_AUDIENCE_MEMBERS: &'static str =
        "DELETE FROM audience_members WHERE author_user_id = $1 AND audience_id = $2";

    const DELETE_AUDIENCE: &'static str =
        "DELETE FROM audiences WHERE author_user_id = $1 AND audience_id = $2";

    const LIST_AUDIENCES: &'static str = "SELECT audience_id, name, created_at FROM audiences \
         WHERE author_user_id = $1 ORDER BY audience_id";

    const INSERT_MEMBER: &'static str =
        "INSERT INTO audience_members (audience_id, subscription_id, author_user_id) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (audience_id, subscription_id) DO NOTHING";

    const DELETE_MEMBER: &'static str =
        "DELETE FROM audience_members WHERE audience_id = $1 AND subscription_id = $2";

    const LIST_MEMBERS: &'static str =
        "SELECT subscription_id FROM audience_members WHERE audience_id = $1 \
         ORDER BY subscription_id";
}
