pub mod atompub;
pub mod audience;
pub mod auth;
pub mod backup;
pub mod email;
pub mod feed;
pub mod invite;
pub mod mailer;
pub mod media;
pub mod password;
pub mod render;
pub mod site;
pub mod slug;
pub mod tag;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod text;
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
