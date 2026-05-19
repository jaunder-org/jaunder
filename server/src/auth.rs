// RegistrationPolicy, load_registration_policy, generate_token, and hash_token
// live in `storage::auth` (shared by web and server). AuthUser and require_auth
// are defined in `web` (they use Leptos/Axum types). Re-exported here for
// server-crate callers' convenience.
pub use storage::{
    generate_token, hash_token, load_registration_policy, InvalidRegistrationPolicy,
    RegistrationPolicy,
};
pub use web::auth::{require_auth, AuthUser};
