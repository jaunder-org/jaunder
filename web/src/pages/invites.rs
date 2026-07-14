use crate::auth::get_registration_policy;
use crate::error::WebError;
use crate::invites::{list_invites, CreateInvite};
use crate::pages::Topbar;
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
    let policy = crate::server_resource(|| (), |()| get_registration_policy());
    let invites = crate::server_resource(move || create_action.version().get(), |_| list_invites());

    view! {
        <Topbar title="Invites".to_string() sub="Manage codes".to_string() />
        <div class="j-scroll">
            <div class="j-page">
                <Suspense fallback=|| {
                    view! { <p class="j-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        let policy_str = policy.await.unwrap_or_default();
                        if policy_str != "invite_only" {
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
                                        <label>
                                            "Invitee email"
                                            <input type="email" name="recipient_email" required=true />
                                        </label>
                                        <label>
                                            "Expires in hours"
                                            <input type="number" name="expires_in_hours" />
                                        </label>
                                        <button type="submit" class="j-btn is-primary">
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
                                                        .map(|args| args.recipient_email)
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
                                            .map(|i| {
                                                view! {
                                                    <li>
                                                        "Expires: " {i.expires_at.clone()}
                                                        {i
                                                            .used_at
                                                            .clone()
                                                            .map(|t| {
                                                                view! {
                                                                    " (used at "
                                                                    {t}
                                                                    ")"
                                                                }
                                                            })}
                                                    </li>
                                                }
                                            })
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
