//! Invites vertical — wasm-only UI (ADR-0070): the invite management page.

use super::{Create, CreateInviteRequest, Info, PageAccess};
use crate::auth;
use crate::error::WebError;
use crate::forms::{self, Field, ValidatedInput};
use crate::registration;
use crate::topbar::Topbar;
use common::{MutationOutcome, email::Email, invite::InviteTtlHours};
use leptos::prelude::*;

/// Invites vertical — creates new invitations, emailing each invitation link to its recipient
/// (#433). The ledger contains metadata only; raw codes never reach the client (#400).
///
/// An unavailable policy or an unauthorized viewer sees the client-side "Page not found."
/// fallback — authed routes are static CSR shells, so there is no SSR 404 (ADR-0040/0041/#180).
#[component]
pub fn InvitesPage() -> impl IntoView {
    let create_action = ServerAction::<Create>::new();
    let successful_creates = RwSignal::new(0_u32);
    Effect::new(move |_| {
        if let Some(Ok(MutationOutcome::Confirmed(()) | MutationOutcome::CommitIndeterminate(()))) =
            create_action.value().get()
        {
            successful_creates.update(|version| *version += 1);
        }
    });
    let session = auth::use_session();
    let access = Resource::new(
        || (),
        move |()| {
            let reconcile = session.reconcile;
            async move { super::resolve_page_access(registration::get_policy().await, reconcile.await) }
        },
    );

    view! {
        <Topbar title="Invites" sub="Manage invitations" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match access.await {
                            Ok(PageAccess::Issuer { show_ledger }) => {
                                view! {
                                    <InviteIssuer
                                        action=create_action
                                        successful_creates
                                        show_ledger
                                    />
                                }
                                    .into_any()
                            }
                            Ok(PageAccess::Unavailable) => {
                                view! { <p>"Page not found."</p> }.into_any()
                            }
                            Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        }
                    })}
                </Suspense>
            </div>
        </div>
    }
}

#[component]
fn InviteIssuer(
    action: ServerAction<Create>,
    successful_creates: RwSignal<u32>,
    show_ledger: bool,
) -> impl IntoView {
    view! {
        <InviteCreateForm action />
        <InviteCreateOutcome action />
        {show_ledger.then(|| view! { <InviteLedger successful_creates /> })}
    }
}

#[component]
fn InviteLedger(successful_creates: RwSignal<u32>) -> impl IntoView {
    let invites = Resource::new(move || successful_creates.get(), |_| super::list());

    view! {
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match invites.await {
                    Ok(list) => {
                        view! {
                            <ul>
                                {list
                                    .into_iter()
                                    .map(|i| render_invite_row(&i))
                                    .collect::<Vec<_>>()}
                            </ul>
                        }
                            .into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}

#[component]
fn InviteCreateForm(action: ServerAction<Create>) -> impl IntoView {
    let recipient = Field::<Email>::new();
    // Optional TTL: empty dispatches `None` for the server's 168-hour default. A
    // non-empty value is dispatched only after `Field::parsed()` validates it.
    let ttl = Field::<InviteTtlHours>::optional();
    let (disabled, submit) = forms::server_action_submit(action, move || {
        let expires_in_hours = ttl
            .value()
            .trim()
            .is_empty()
            .then_some(None)
            .or_else(|| ttl.parsed().map(Some));
        recipient
            .parsed()
            .zip(expires_in_hours)
            .map(|(recipient_email, expires_in_hours)| Create {
                request: CreateInviteRequest {
                    expires_in_hours,
                    recipient_email,
                },
            })
    });

    view! {
        <form on:submit=submit>
            <ValidatedInput<Email>
                label="Invitee email"
                name="recipient_email"
                input_type="email"
                autocomplete="email"
                field=recipient
            />
            <ValidatedInput<InviteTtlHours>
                label="Expires in hours"
                name="expires_in_hours"
                input_type="number"
                field=ttl
            />
            <button type="submit" class="j-btn is-primary" prop:disabled=move || disabled.get()>
                "Send Invite"
            </button>
        </form>
    }
}

/// The create-invite feedback line: on success a "Invitation emailed to …" note that
/// echoes the recipient the issuer just submitted (read back from the action's own input, since
/// the server fn returns nothing), on failure the error.
///
/// Split out of [`InvitesPage`] (#306) so that page's `view!` carries only creation feedback.
#[component]
fn InviteCreateOutcome(action: ServerAction<Create>) -> impl IntoView {
    view! {
        {move || {
            action
                .value()
                .get()
                .map(|r: Result<MutationOutcome<()>, WebError>| match r {
                    Ok(MutationOutcome::Confirmed(())) => {
                        let to = action
                            .input()
                            .get()
                            .map(|args| args.request.recipient_email.to_string())
                            .unwrap_or_default();
                        view! { <p class="j-form-note">"Invitation emailed to " {to} "."</p> }
                            .into_any()
                    }
                    Ok(MutationOutcome::CommitIndeterminate(())) => {
                        view! {
                            <p class="error">
                                "The invitation may have been created, but its status could not be confirmed. Refresh to check."
                            </p>
                        }
                            .into_any()
                    }
                    Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                })
        }}
    }
}

/// Renders a single invite row: its expiry and, if used, when.
///
/// `+ use<>` is precise capturing (ADR-0100): under edition 2024 a return-position
/// `impl Trait` captures every in-scope lifetime, and Leptos requires a stored view
/// to be `'static`. This body derives owned values before the `view!` and lends
/// nothing across it, so capturing nothing is the truth — and it keeps `&Info` in
/// the signature rather than forcing the caller to hand over ownership.
fn render_invite_row(i: &Info) -> impl IntoView + use<> {
    view! {
        <li>
            "Expires: " {i.expires_at.to_string()}
            {i
                .used_at
                .map(|t| {
                    view! {
                        " (used at "
                        {t.to_string()}
                        ")"
                    }
                })}
        </li>
    }
}
