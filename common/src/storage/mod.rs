//! Abstract storage interfaces and data models.
//!
//! This module defines the traits and types that constitute Jaunder's
//! persistence layer. By defining these in the `common` crate, we allow
//! both the `server` and `web` crates to interact with storage while
//! remaining agnostic of the underlying database engine (e.g., `SQLite` or
//! `PostgreSQL`).
//!
//! # Organization
//!
//! The storage layer is divided into functional modules, each defining a
//! trait for operations on a specific domain (e.g., [`UserStorage`],
//! [`PostStorage`]). The [`AppState`] struct bundles these traits together
//! for easy access across the application.
//!
//! For cross-table operations that must be atomic, see [`AtomicOps`].

mod app_state;
mod atomic;
mod email;
mod invites;
mod media;
mod password;
mod posts;
mod sessions;
mod site_config;
mod user_config;
mod users;

pub use app_state::*;
pub use atomic::*;
pub use email::*;
pub use invites::*;
pub use media::*;
pub use password::*;
pub use posts::*;
pub use sessions::*;
pub use site_config::*;
pub use user_config::*;
pub use users::*;
