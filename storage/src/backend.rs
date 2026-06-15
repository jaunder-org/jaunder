//! Backend abstraction for the deduplicated storage layer.
//!
//! `Backend` bundles, once, the sqlx scalar-bind and pool-executor bounds that
//! every generic storage helper needs, and carries the `db.system` value. It is
//! used only as a bound on concrete generic stores (e.g. `SessionStore<DB>`) —
//! never as a trait object — so its associated const does not affect the
//! object-safety of the public storage traits.

use chrono::{DateTime, Utc};

/// A sqlx database jaunder supports, with the common bind/executor bounds and
/// its OpenTelemetry `db.system` identity.
pub trait Backend: sqlx::Database
where
    for<'q> i64: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'q> &'q str: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'c> &'c sqlx::Pool<Self>: sqlx::Executor<'c, Database = Self>,
{
    /// Value of the `db.system` span field (`"sqlite"` | `"postgres"`).
    const DB_SYSTEM: &'static str;
}

impl Backend for sqlx::Sqlite {
    const DB_SYSTEM: &'static str = "sqlite";
}

impl Backend for sqlx::Postgres {
    const DB_SYSTEM: &'static str = "postgres";
}
