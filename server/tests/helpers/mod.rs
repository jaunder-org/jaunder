// Shared test helpers for the server integration suite. Compiled once as the single
// `mod helpers;` of the `integration` test binary, so every item is reachable from
// some subsystem module and no dead-code/unused suppression is needed.
//
// The both-backend test harness — `Backend`, `TestEnv`, per-test DB provisioning,
// and the `backends`/`sqlite_only`/`postgres_only` rstest templates — lives in
// `storage::test_support` (gated by storage's `test-support` feature; ADR-0033) so
// `storage`'s own tests can use it from the same crate instance. Test files import
// what they need directly from `storage::test_support`.

mod atompub;
pub(crate) mod error_signal;
mod http;
mod posts;
mod registrar;
mod session;
mod site_config;
mod websub_capturing;
pub(crate) use http::make_app_macro as make_app;

// `basic_header` is consumed only by `atompub.rs`, so it is not re-exported.
// Exporting it would create an unused import, which this crate denies. The
// definition remains `pub` for sibling-module access.
pub use atompub::{
    atompub, atompub_at, atompub_authed, atompub_get, atompub_location, atompub_post_xml,
    atompub_put_xml, atompub_upload, atompub_xml,
};
pub use http::{
    ForeignReferenceResolver, MultipartFile, TestHttpResponse, body_string, confirmed_mutation,
    get_asset, post_form, post_form_with_bearer, post_form_with_credentials,
    post_form_with_secure_flag, post_json, post_json_with_credentials, post_multipart,
    post_password_reset_form_with_dependencies, post_password_reset_request_with_dependencies,
    post_server_fn, post_server_fn_request_fixture,
    post_server_fn_request_fixture_with_secure_flag, post_server_fn_with_media_ownership_resolver,
    post_server_fn_with_secure_flag, post_server_fn_with_ua, prepare_app,
};
pub use posts::{create_post_json, update_post_json};
pub use registrar::{REGISTERED_SERVER_FN_COUNT, ensure_server_fns_registered};
pub use session::{
    SeededSession, assert_no_email, assert_one_absolute_link_email, create_operator_and_session,
    create_session_for, create_user_and_session, session_cookie, token_from_set_cookie,
};
pub use site_config::{delete_site_config, set_site_config};
// The capturing WebSub client used by `feed_worker.rs`.
pub use websub_capturing::CapturingWebSubClient;
