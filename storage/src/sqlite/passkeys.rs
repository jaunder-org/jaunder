use async_trait::async_trait;
use sqlx::Sqlite;

use common::{ids::UserId, time::UtcInstant};

use crate::WriteTransaction;
use crate::passkeys::{
    PasskeyCredential, PasskeyCredentialId, PasskeyDialect, PasskeyLabel, PasskeyStore,
    StoredPasskeySerialization, decode_credential,
};
use crate::sql::QueryStorageExt;

/// SQLite-backed Passkey storage.
pub type SqlitePasskeyStorage = PasskeyStore<Sqlite>;

#[async_trait]
impl PasskeyDialect for Sqlite {
    async fn credential_for_authentication(
        transaction: &mut WriteTransaction,
        credential_id: &PasskeyCredentialId,
    ) -> sqlx::Result<Option<PasskeyCredential>> {
        let connection = <Sqlite as crate::backend::Backend>::write_connection(transaction)?;
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
             FROM passkey_credentials WHERE credential_id = $1",
        )
        .bind_storage(credential_id)
        .fetch_optional(&mut *connection)
        .await?;

        row.map(|(id, user_id, label, encoded, created_at, last_used_at)| {
            decode_credential(id, user_id, label, &encoded.0, created_at, last_used_at)
        })
        .transpose()
    }

    async fn lock_rp_host(_transaction: &mut WriteTransaction) -> sqlx::Result<()> {
        Ok(())
    }
}
