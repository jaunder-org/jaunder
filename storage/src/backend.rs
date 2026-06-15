//! Backend abstraction for the deduplicated storage layer.
//!
//! `Backend` marks the sqlx databases jaunder supports and carries each one's
//! `db.system` identity. It is used only as a bound on concrete generic stores
//! (e.g. `SessionStore<DB>`) — never as a trait object — so its associated const
//! does not affect the object-safety of the public storage traits.
//!
//! `Backend` deliberately carries NO bind/executor `where`-bounds. Rust does not
//! propagate a trait's `where`-clause to subtraits or `impl` headers (see
//! ADR-0019), so bundling them here would buy nothing — every generic store
//! would have to restate them regardless, including bounds it doesn't use.
//! Instead, each store's `impl` restates exactly the bounds it needs.

/// A sqlx database jaunder supports, carrying its OpenTelemetry `db.system`
/// identity.
pub trait Backend: sqlx::Database {
    /// Value of the `db.system` span field (`"sqlite"` | `"postgres"`).
    const DB_SYSTEM: &'static str;
}

impl Backend for sqlx::Sqlite {
    const DB_SYSTEM: &'static str = "sqlite";
}

impl Backend for sqlx::Postgres {
    const DB_SYSTEM: &'static str = "postgres";
}
