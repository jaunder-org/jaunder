use async_trait::async_trait;
use sqlx::Postgres;

use common::{ids::UserId, time::UtcInstant};

use crate::WriteTransaction;
use crate::passkeys::{
    PASSKEY_RP_ADVISORY_LOCK_KEY, PasskeyCredential, PasskeyCredentialId, PasskeyDialect,
    PasskeyLabel, PasskeyStore, StoredPasskeySerialization, decode_credential,
};
use crate::sql::QueryStorageExt;

/// PostgreSQL-backed Passkey storage.
pub type PostgresPasskeyStorage = PasskeyStore<Postgres>;

#[async_trait]
impl PasskeyDialect for Postgres {
    async fn credential_for_authentication(
        transaction: &mut WriteTransaction,
        credential_id: &PasskeyCredentialId,
    ) -> sqlx::Result<Option<PasskeyCredential>> {
        let connection = <Postgres as crate::backend::Backend>::write_connection(transaction)?;
        let row = sqlx::query_as::<
            _,
            (
                PasskeyCredentialId,
                UserId,
                PasskeyLabel,
                StoredPasskeySerialization,
                UtcInstant,
                Option<UtcInstant>,
            ),
        >(
            "SELECT credential_id, user_id, label, credential, created_at, last_used_at
             FROM passkey_credentials WHERE credential_id = $1 FOR UPDATE",
        )
        .bind_storage(credential_id)
        .fetch_optional(&mut *connection)
        .await?;

        row.map(|(id, user_id, label, encoded, created_at, last_used_at)| {
            decode_credential(id, user_id, label, &encoded.0, created_at, last_used_at)
        })
        .transpose()
    }

    async fn lock_rp_host(transaction: &mut WriteTransaction) -> sqlx::Result<()> {
        let connection = <Postgres as crate::backend::Backend>::write_connection(transaction)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind_storage(PASSKEY_RP_ADVISORY_LOCK_KEY)
            .execute(&mut *connection)
            .await?;
        Ok(())
    }
}
