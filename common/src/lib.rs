// The sqlx storage bridge the `StrNewtype` derive emits (#438) is server-only:
// `common`'s `sqlx` feature must never be enabled for a wasm target. Enabling it there
// would already fail to build — `sqlx` pulls native deps that don't compile for
// wasm32 — but that surfaces as a wall of downstream errors; this guard turns the
// mis-wiring into one clear message at the source instead.
#[cfg(all(target_arch = "wasm32", feature = "sqlx"))]
compile_error!("common's `sqlx` feature must not be enabled for wasm32 targets (#438)");

pub mod absolute_url;
pub mod atompub;
pub mod audience;
pub mod auth;
pub mod backup;
pub mod bio;
pub mod config_key;
pub mod display_name;
pub mod email;
pub mod etag;
pub mod feed;
pub mod ids;
pub mod invite;
pub mod list_state;
pub mod mailbox;
pub mod mailer;
pub mod media;
pub mod pagination;
pub mod password;
pub mod pg_identifier;
pub mod pg_role_password;
pub mod post_body;
pub mod post_summary;
pub mod post_title;
pub mod registration;
pub mod render;
pub mod root_relative_url;
pub mod seed;
pub mod session_label;
pub mod session_user;
pub mod site;
pub mod slug;
pub mod smtp_host;
pub mod smtp_password;
pub mod smtp_port;
pub mod smtp_sender;
pub mod smtp_tls_mode;
pub mod smtp_username;
pub mod stored_password_hash;
pub mod tag;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod text;
pub mod time;
pub mod token;
pub mod username;
pub mod visibility;

/// True only when the test-only cheap Argon2 parameters are compiled in.
/// Production builds (no `cheap-kdf`) leave this `false`; downstream binaries
/// assert on it at startup as a fail-closed guard.
pub const CHEAP_KDF_ENABLED: bool = cfg!(feature = "cheap-kdf");

// A release/optimized build must never carry the cheap KDF params. Test builds
// (debug_assertions on) are unaffected; an optimized build with the feature on
// fails to compile here rather than producing a weak-hashing artifact.
#[cfg(all(feature = "cheap-kdf", not(debug_assertions)))]
compile_error!("cheap-kdf must not be enabled in a release/optimized build");
