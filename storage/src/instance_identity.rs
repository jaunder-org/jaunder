use crate::sql::QueryStorageExt;

use std::str::FromStr;

use sqlx::{Database, Decode, Encode, Executor, Pool, Result, Type};
use thiserror::Error;
use uuid::Uuid;

/// Canonical public identifier of one Jaunder database instance.
///
/// The value is always a lowercase, hyphenated UUID. It is created once after
/// migrations and survives backup/restore, so a restored clone remains the
/// same logical instance.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct InstanceId(String);

impl InstanceId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for InstanceId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("instance identity must be a canonical UUID")]
pub struct ParseInstanceIdError;

impl FromStr for InstanceId {
    type Err = ParseInstanceIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(value).map_err(|_| ParseInstanceIdError)?;
        if uuid.to_string() != value {
            return Err(ParseInstanceIdError);
        }
        Ok(Self(value.to_owned()))
    }
}
/// Atomically creates the database identity if absent, then validates and
/// returns the sole persisted identity.
pub(crate) async fn ensure<DB>(pool: &Pool<DB>) -> Result<InstanceId>
where
    DB: Database,
    for<'c> &'c Pool<DB>: Executor<'c, Database = DB>,
    DB::Arguments: sqlx::IntoArguments<DB>,
    usize: sqlx::ColumnIndex<DB::Row>,
    for<'q> InstanceId: Encode<'q, DB> + Type<DB>,
    for<'r> InstanceId: Decode<'r, DB> + Type<DB>,
{
    let generated = InstanceId::new();
    sqlx::query(
        "INSERT INTO instance_identity (singleton, instance_id) VALUES (1, $1) ON CONFLICT DO NOTHING",
    )
    .bind_storage(generated)
    .execute(pool)
    .await?;

    sqlx::query_scalar::<_, InstanceId>("SELECT instance_id FROM instance_identity")
        .fetch_one(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Backend, backends};
    use rstest::*;
    use rstest_reuse::*;

    #[test]
    fn instance_id_accepts_only_canonical_uuid() {
        let id = "123e4567-e89b-12d3-a456-426614174000";
        assert_eq!(id.parse::<InstanceId>().unwrap().to_string(), id);
        assert!(
            "123E4567-E89B-12D3-A456-426614174000"
                .parse::<InstanceId>()
                .is_err()
        );
        assert!(
            "123e4567e89b12d3a456426614174000"
                .parse::<InstanceId>()
                .is_err()
        );
        assert!("not-a-uuid".parse::<InstanceId>().is_err());
    }

    #[test]
    fn default_instance_id_is_canonical() {
        let id = InstanceId::default();
        assert_eq!(id.to_string().parse::<InstanceId>().unwrap(), id);
    }

    #[apply(backends)]
    #[tokio::test]
    async fn opening_rejects_a_noncanonical_persisted_instance_id(#[case] backend: Backend) {
        let env = backend.setup().await;
        env.base
            .pool()
            .execute("UPDATE instance_identity SET instance_id = 'not-a-uuid'")
            .await
            .expect("seed malformed persisted identity");

        let error = crate::with_closeable_pool!(env.base.pool(), pool, { ensure(pool).await })
            .expect_err("opening must reject malformed persisted identity");

        let sqlx::Error::ColumnDecode { source, .. } = error else {
            panic!("malformed identity must be a typed column-decode failure");
        };
        assert!(source.downcast_ref::<ParseInstanceIdError>().is_some());
    }

    #[apply(backends)]
    #[tokio::test]
    async fn opening_persists_one_canonical_instance_id(#[case] backend: Backend) {
        let env = backend.setup().await;
        let persisted = env
            .base
            .pool()
            .string_quintuples("SELECT instance_id, '', '', '', '' FROM instance_identity")
            .await
            .expect("identity query succeeds");
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].0, env.base.instance_id().to_string());
        assert!(persisted[0].0.parse::<InstanceId>().is_ok());
    }
}
