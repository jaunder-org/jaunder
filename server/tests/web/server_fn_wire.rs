//! The wire contract of every `#[macros::server]` fn (#714).
//!
//! These read `ServerFn::PATH` off the **generated types** rather than off source,
//! so they hold even if xtask's `syn` enumeration breaks. That matters more than it
//! sounds: every server-fn gate reports "no problems" on an empty enumeration, so a
//! predicate that stops matching is a silent green across all of them. This file is
//! the assertion that does not depend on that machinery working.
//!
//! It also carries the uniqueness backstop. `(vertical, ident)` is a primary key
//! only because the macro refuses to expand outside `web/src/<vertical>/api.rs`; if
//! that rule were relaxed, two fns in one vertical's submodules would derive one URL
//! and the compiler would not catch it — a glob re-export lets one shadow the other,
//! so the pair compiles and the loser 404s (#358). Pairwise distinctness fails
//! loudly in that case.

use server_fn::ServerFn;

/// Declares the expected wire path of each server fn as
/// `<type> => "<vertical>" / "<ident>"`, and generates the collector that asserts
/// it.
///
/// A table rather than 56 inline assertions: the list is the interesting part, and
/// one line per fn keeps it readable — and short enough not to trip
/// `clippy::too_many_lines`, which an unrolled version does at 138 lines.
macro_rules! wire_contract {
    ($( $ty:ty => $vertical:literal / $ident:literal ),* $(,)?) => {
        /// Every asserted path, in declaration order.
        fn asserted_paths() -> Vec<&'static str> {
            let mut out = Vec::new();
            $(
                assert_eq!(
                    <$ty as ServerFn>::PATH,
                    concat!("/api/", $vertical, "/", $ident),
                    "derived wire path changed for {}::{}",
                    $vertical,
                    $ident,
                );
                out.push(<$ty as ServerFn>::PATH);
            )*
            out
        }
    };
}

wire_contract! {
    web::audiences::AddSubscriber => "audiences" / "add_subscriber",
    web::audiences::Create => "audiences" / "create",
    web::audiences::Delete => "audiences" / "delete",
    web::audiences::ListMembers => "audiences" / "list_members",
    web::audiences::ListMine => "audiences" / "list_mine",
    web::audiences::ListMySubscribers => "audiences" / "list_my_subscribers",
    web::audiences::RemoveSubscriber => "audiences" / "remove_subscriber",
    web::audiences::Rename => "audiences" / "rename",
    web::auth::GetSession => "auth" / "get_session",
    web::auth::Login => "auth" / "login",
    web::auth::Logout => "auth" / "logout",
    web::backup::GetSettings => "backup" / "get_settings",
    web::backup::IsWarningVisible => "backup" / "is_warning_visible",
    web::backup::UpdateSettings => "backup" / "update_settings",
    web::email::RequestVerification => "email" / "request_verification",
    web::email::Verify => "email" / "verify",
    web::invites::Create => "invites" / "create",
    web::invites::List => "invites" / "list",
    web::media::Delete => "media" / "delete",
    web::media::GetUsage => "media" / "get_usage",
    web::media::ListMine => "media" / "list_mine",
    web::media::Upload => "media" / "upload",
    web::password_reset::Confirm => "password_reset" / "confirm",
    web::password_reset::Request => "password_reset" / "request",
    web::posts::Create => "posts" / "create",
    web::posts::Delete => "posts" / "delete",
    web::posts::Get => "posts" / "get",
    web::posts::GetAudienceSelection => "posts" / "get_audience_selection",
    web::posts::GetDefaultAudienceSelection => "posts" / "get_default_audience_selection",
    web::posts::GetPreview => "posts" / "get_preview",
    web::posts::GetPostHistory => "posts" / "get_post_history",
    web::posts::GetRevisionHistoryDetail => "posts" / "get_revision_history_detail",
    web::posts::ListDrafts => "posts" / "list_drafts",
    web::posts::ListScheduled => "posts" / "list_scheduled",
    web::posts::ListHistory => "posts" / "list_history",
    web::posts::Publish => "posts" / "publish",
    web::posts::Unpublish => "posts" / "unpublish",
    web::posts::Update => "posts" / "update",
    web::profile::Get => "profile" / "get",
    web::profile::GetDefaultPostFormat => "profile" / "get_default_post_format",
    web::profile::GetSiteTheme => "profile" / "get_site_theme",
    web::profile::GetYourPagesTheme => "profile" / "get_your_pages_theme",
    web::profile::ResetYourPagesTheme => "profile" / "reset_your_pages_theme",
    web::profile::SetDefaultPostFormat => "profile" / "set_default_post_format",
    web::profile::SetSiteTheme => "profile" / "set_site_theme",
    web::profile::SetYourPagesTheme => "profile" / "set_your_pages_theme",
    web::profile::Update => "profile" / "update",
    web::registration::GetPolicy => "registration" / "get_policy",
    web::registration::Register => "registration" / "register",
    web::sessions::CreateAppPassword => "sessions" / "create_app_password",
    web::sessions::List => "sessions" / "list",
    web::sessions::Revoke => "sessions" / "revoke",
    web::site::GetIdentity => "site" / "get_identity",
    web::site::IsBaseUrlWarningVisible => "site" / "is_base_url_warning_visible",
    web::site::UpdateIdentity => "site" / "update_identity",
    web::subscriptions::IsSubscribed => "subscriptions" / "is_subscribed",
    web::subscriptions::Subscribe => "subscriptions" / "subscribe",
    web::subscriptions::Unsubscribe => "subscriptions" / "unsubscribe",
    web::tags::List => "tags" / "list",
    web::websub::GetWebsubSettings => "websub" / "get_websub_settings",
    web::websub::ListDeadLetters => "websub" / "list_dead_letters",
    web::websub::RedriveDeadLetters => "websub" / "redrive_dead_letters",
    web::websub::UpdateWebsubHub => "websub" / "update_websub_hub",
    // The five timeline listings (#714). Their vertical is the whole reason
    // these assertions exist: nothing in source says `timeline`, only the file
    // path does.
    web::timeline::ListByTag => "timeline" / "list_by_tag",
    web::timeline::ListByUser => "timeline" / "list_by_user",
    web::timeline::ListByUserAndTag => "timeline" / "list_by_user_and_tag",
    web::timeline::ListHomeFeed => "timeline" / "list_home_feed",
    web::timeline::ListLocalTimeline => "timeline" / "list_local_timeline",
}

/// Every server fn's wire path is exactly `/api/<vertical>/<ident>`.
///
/// The vertical is the directory under `web/src` and the ident is the fn name;
/// both are derived by the macro, and nothing in source states either. This is
/// where the derived values become checkable again.
#[test]
fn every_server_fn_path_is_api_vertical_ident() {
    let paths = asserted_paths();
    assert_eq!(
        paths.len(),
        crate::helpers::REGISTERED_SERVER_FN_COUNT,
        "every registered server fn must have its wire path asserted here"
    );
}

/// No two server fns share a wire path.
///
/// Enumeration-independent, so it survives a stale `syn` predicate — and it is the
/// backstop if the placement rule that makes `(vertical, ident)` unique is ever
/// relaxed.
#[test]
fn server_fn_wire_paths_are_pairwise_distinct() {
    let paths = asserted_paths();
    let mut unique = paths.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        paths.len(),
        "a duplicate means two fns derive one URL and one of them silently 404s (#358)"
    );
}
