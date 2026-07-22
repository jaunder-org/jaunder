use crate::error::WebError;
use crate::forms::{Field, ValidatedInput};
use crate::invites::{list_invites, CreateInvite, InviteInfo};
use crate::pages::Topbar;
use crate::registration::get_registration_policy;
use common::email::Email;
use common::registration::RegistrationPolicy;
use leptos::prelude::*;

/// Invites page — lists invites (metadata only; raw codes are never sent to the client,
/// #400) and creates new ones, **emailing the invitation link** to a recipient (#433).
/// A code is never shown here — it reaches the invitee only as the link in the email
/// (or the `jaunder user invite` CLI URL for manual sharing).
/// Returns 404 (via SSR response options) when the registration policy is not
/// `invite_only`.
#[component]
pub fn InvitesPage() -> impl IntoView {
    let create_action = ServerAction::<CreateInvite>::new();
    let recipient = Field::<Email>::new();
    let policy = Resource::new(|| (), |()| get_registration_policy());
    let invites = Resource::new(move || create_action.version().get(), |_| list_invites());

    view! {
        <Topbar title="Invites" sub="Manage codes" />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        if policy.await != Ok(RegistrationPolicy::InviteOnly) {
                            #[cfg(feature = "server")]
                            {
                                use leptos::context::use_context;
                                use leptos_axum::ResponseOptions;
                                if let Some(opts) = use_context::<ResponseOptions>() {
                                    opts.set_status(axum::http::StatusCode::NOT_FOUND);
                                }
                            }
                            return // Set 404 status when rendered server-side.
                            view! { <p>"Page not found."</p> }
                                .into_any();
                        }
                        match invites.await {
                            Ok(list) => {
                                view! {
                                    <ActionForm action=create_action>
                                        <ValidatedInput<
                                        Email,
                                    >
                                            label="Invitee email"
                                            name="recipient_email"
                                            input_type="email"
                                            autocomplete="email"
                                            field=recipient
                                        />
                                        <label>
                                            "Expires in hours"
                                            <input type="number" name="expires_in_hours" />
                                        </label>
                                        <button
                                            type="submit"
                                            class="j-btn is-primary"
                                            prop:disabled=move || !recipient.is_valid()
                                        >
                                            "Send Invite"
                                        </button>
                                    </ActionForm>
                                    {move || {
                                        create_action
                                            .value()
                                            .get()
                                            .map(|r: Result<(), WebError>| match r {
                                                Ok(()) => {
                                                    let to = create_action
                                                        .input()
                                                        .get()
                                                        .map(|args| args.recipient_email.to_string())
                                                        .unwrap_or_default();
                                                    // Echo the recipient the operator just submitted
                                                    // (from the action's input) to confirm delivery.
                                                    view! {
                                                        <p class="j-form-note">"Invitation emailed to " {to} "."</p>
                                                    }
                                                        .into_any()
                                                }
                                                Err(e) => {
                                                    view! { <p class="error">{e.to_string()}</p> }.into_any()
                                                }
                                            })
                                    }}
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
            </div>
        </div>
    }
}

/// Renders a single invite row: its expiry and, if used, when.
fn render_invite_row(i: &InviteInfo) -> impl IntoView {
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
