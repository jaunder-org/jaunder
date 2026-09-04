use std::sync::LazyLock;

pub fn ensure_server_fns_registered() {
    static REGISTRATIONS: LazyLock<()> = LazyLock::new(|| {
        server_fn::axum::register_explicit::<web::auth::GetSession>();
        server_fn::axum::register_explicit::<web::backup::IsWarningVisible>();
        server_fn::axum::register_explicit::<web::backup::GetSettings>();
        server_fn::axum::register_explicit::<web::backup::UpdateSettings>();
        server_fn::axum::register_explicit::<web::registration::GetPolicy>();
        server_fn::axum::register_explicit::<web::registration::Register>();
        server_fn::axum::register_explicit::<web::auth::Login>();
        server_fn::axum::register_explicit::<web::auth::Logout>();
        server_fn::axum::register_explicit::<web::email::RequestVerification>();
        server_fn::axum::register_explicit::<web::email::Verify>();
        server_fn::axum::register_explicit::<web::profile::Get>();
        server_fn::axum::register_explicit::<web::profile::Update>();
        server_fn::axum::register_explicit::<web::profile::GetDefaultPostFormat>();
        server_fn::axum::register_explicit::<web::profile::SetDefaultPostFormat>();
        server_fn::axum::register_explicit::<web::profile::GetYourPagesTheme>();
        server_fn::axum::register_explicit::<web::profile::SetYourPagesTheme>();
        server_fn::axum::register_explicit::<web::profile::ResetYourPagesTheme>();
        server_fn::axum::register_explicit::<web::profile::GetSiteTheme>();
        server_fn::axum::register_explicit::<web::profile::SetSiteTheme>();
        server_fn::axum::register_explicit::<web::sessions::List>();
        server_fn::axum::register_explicit::<web::sessions::Revoke>();
        server_fn::axum::register_explicit::<web::sessions::CreateAppPassword>();
        server_fn::axum::register_explicit::<web::invites::Create>();
        server_fn::axum::register_explicit::<web::invites::List>();
        server_fn::axum::register_explicit::<web::password_reset::Request>();
        server_fn::axum::register_explicit::<web::password_reset::Confirm>();
        server_fn::axum::register_explicit::<web::posts::Create>();
        server_fn::axum::register_explicit::<web::posts::Get>();
        server_fn::axum::register_explicit::<web::posts::GetPreview>();
        server_fn::axum::register_explicit::<web::posts::Update>();
        server_fn::axum::register_explicit::<web::posts::ListDrafts>();
        server_fn::axum::register_explicit::<web::posts::ListScheduled>();
        server_fn::axum::register_explicit::<web::posts::Publish>();
        server_fn::axum::register_explicit::<web::posts::ListHistory>();
        server_fn::axum::register_explicit::<web::posts::GetPostHistory>();
        server_fn::axum::register_explicit::<web::posts::GetRevisionHistoryDetail>();
        server_fn::axum::register_explicit::<web::timeline::ListByUser>();
        server_fn::axum::register_explicit::<web::timeline::ListLocalTimeline>();
        server_fn::axum::register_explicit::<web::timeline::ListHomeFeed>();
        server_fn::axum::register_explicit::<web::timeline::ListByTag>();
        server_fn::axum::register_explicit::<web::timeline::ListByUserAndTag>();
        server_fn::axum::register_explicit::<web::posts::Delete>();
        server_fn::axum::register_explicit::<web::posts::Unpublish>();
        server_fn::axum::register_explicit::<web::posts::GetDefaultAudienceSelection>();
        server_fn::axum::register_explicit::<web::posts::GetAudienceSelection>();
        server_fn::axum::register_explicit::<web::site::GetIdentity>();
        server_fn::axum::register_explicit::<web::site::UpdateIdentity>();
        server_fn::axum::register_explicit::<web::site::IsBaseUrlWarningVisible>();
        server_fn::axum::register_explicit::<web::websub::GetWebsubSettings>();
        server_fn::axum::register_explicit::<web::websub::UpdateWebsubHub>();
        server_fn::axum::register_explicit::<web::websub::ListDeadLetters>();
        server_fn::axum::register_explicit::<web::websub::RedriveDeadLetters>();
        server_fn::axum::register_explicit::<web::media::ListMine>();
        server_fn::axum::register_explicit::<web::media::GetUsage>();
        server_fn::axum::register_explicit::<web::media::Delete>();
        server_fn::axum::register_explicit::<web::media::Upload>();
        server_fn::axum::register_explicit::<web::tags::List>();
        server_fn::axum::register_explicit::<web::subscriptions::Subscribe>();
        server_fn::axum::register_explicit::<web::subscriptions::Unsubscribe>();
        server_fn::axum::register_explicit::<web::subscriptions::IsSubscribed>();
        server_fn::axum::register_explicit::<web::audiences::Create>();
        server_fn::axum::register_explicit::<web::audiences::Rename>();
        server_fn::axum::register_explicit::<web::audiences::Delete>();
        server_fn::axum::register_explicit::<web::audiences::ListMine>();
        server_fn::axum::register_explicit::<web::audiences::ListMySubscribers>();
        server_fn::axum::register_explicit::<web::audiences::AddSubscriber>();
        server_fn::axum::register_explicit::<web::audiences::RemoveSubscriber>();
        server_fn::axum::register_explicit::<web::audiences::ListMembers>();
    });
    LazyLock::force(&REGISTRATIONS);
}

/// How many `#[server]` fns the list above registers.
///
/// Kept beside that list deliberately — a `macro_rules!` that both registered and
/// counted would hide the `register_explicit::<…>()` calls inside a token stream,
/// and `server_fn_registrar_check` finds them by parsing this file with `syn`. It
/// would then enumerate **zero** entries and pass, which is the fail-open this
/// whole area exists to prevent.
///
/// So the chain is: the registrar gate proves this list covers every `#[server]`
/// fn in `web/src`; this constant tracks the list; and
/// `server_fn_wire::every_server_fn_path_is_api_vertical_ident_and_distinct`
/// checks itself against the constant. Each link is short enough to keep honest.
pub const REGISTERED_SERVER_FN_COUNT: usize = 68;
