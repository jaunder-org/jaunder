use super::{
    SmtpFormState, SmtpPasswordIntent, SmtpUpdateDraft, UpdateSettings, UpdateSettingsRequest,
};
use crate::error::WebError;
use crate::forms::ValidatedInput;
use crate::mutation_feedback;
use crate::topbar::Topbar;
use common::MutationOutcome;
use common::smtp_host::SmtpHost;
use common::smtp_password::ProfferedSmtpPassword;
use common::smtp_port::SmtpPort;
use common::smtp_sender::SmtpSender;
use common::smtp_tls_mode::SmtpTlsMode;
use common::smtp_username::SmtpUsername;
use leptos::html;
use leptos::prelude::*;

#[component]
pub fn SmtpSettingsPage() -> impl IntoView {
    let update_action = ServerAction::<UpdateSettings>::new();
    let settings = Resource::new(
        move || update_action.version().get(),
        |_| super::get_settings(),
    );

    view! {
        <Topbar title="SMTP Relay" sub="Operations" />
        <div class="j-scroll">
            <div class="j-settings j-site-settings">
                <Suspense fallback=|| {
                    view! { <p class="j-loading j-settings-loading">"Loading\u{2026}"</p> }
                }>
                    {move || Suspend::new(async move {
                        match settings.await {
                            Ok(settings) => smtp_settings_form(&settings, update_action).into_any(),
                            Err(error) => {
                                view! { <p class="error j-settings-error">{error.to_string()}</p> }
                                    .into_any()
                            }
                        }
                    })}
                </Suspense>
                {move || {
                    update_action
                        .value()
                        .get()
                        .map(|result: Result<MutationOutcome<()>, WebError>| {
                            match mutation_feedback::classify(
                                result,
                                "Save acknowledgement was lost; reload to verify the SMTP settings.",
                            ) {
                                mutation_feedback::MutationFeedback::Confirmed(()) => {
                                    view! {
                                        <p class="j-settings-saved" role="status">
                                            "SMTP settings saved. Restart Jaunder through its service manager to apply them."
                                        </p>
                                    }
                                        .into_any()
                                }
                                mutation_feedback::MutationFeedback::Error(message) => {
                                    view! { <p class="error j-settings-error">{message}</p> }
                                        .into_any()
                                }
                            }
                        })
                }}
            </div>
        </div>
    }
}

fn clear_password(state: SmtpFormState, password_input: NodeRef<html::Input>) {
    state.clear_password();
    if let Some(input) = password_input.get() {
        input.set_value("");
    }
}

fn assemble_request(state: SmtpFormState, draft: SmtpUpdateDraft) -> Option<UpdateSettingsRequest> {
    let password = if matches!(
        &draft,
        SmtpUpdateDraft::Disabled {
            password: SmtpPasswordIntent::Replace
        } | SmtpUpdateDraft::Enabled {
            password: SmtpPasswordIntent::Replace,
            ..
        }
    ) {
        Some(
            state
                .password
                .value()
                .parse::<ProfferedSmtpPassword>()
                .ok()?,
        )
    } else {
        None
    };
    Some(match draft {
        SmtpUpdateDraft::Disabled { .. } => UpdateSettingsRequest {
            enabled: false,
            host: None,
            port: SmtpPort::default(),
            tls_mode: SmtpTlsMode::default(),
            sender: SmtpSender::default(),
            authentication_enabled: false,
            username: None,
            password,
        },
        SmtpUpdateDraft::Enabled {
            host,
            port,
            tls_mode,
            sender,
            authentication_enabled,
            username,
            ..
        } => UpdateSettingsRequest {
            enabled: true,
            host: Some(host),
            port,
            tls_mode,
            sender,
            authentication_enabled,
            username,
            password,
        },
    })
}

fn smtp_relay_fields(state: SmtpFormState, initial_tls_mode: SmtpTlsMode) -> impl IntoView {
    view! {
        <ValidatedInput<SmtpHost>
            label="Relay Host"
            name="host"
            field=state.host
            field_class="j-site-field j-site-field-wide"
            class="j-site-input"
        />
        <ValidatedInput<SmtpPort>
            label="Port"
            name="port"
            input_type="number"
            field=state.port
            field_class="j-site-field"
            class="j-site-input"
        />
        <label class="j-site-field">
            <span class="j-form-label">"TLS Mode"</span>
            <select
                class="j-site-input"
                name="tls_mode"
                prop:disabled=move || !state.enabled.get()
                on:change=move |event| {
                    if let Ok(mode) = event_target_value(&event).parse::<SmtpTlsMode>() {
                        state.tls_mode.set(mode);
                    }
                }
            >
                <option value="plain" selected=initial_tls_mode == SmtpTlsMode::Plain>
                    "Plain"
                </option>
                <option value="starttls" selected=initial_tls_mode == SmtpTlsMode::StartTls>
                    "STARTTLS"
                </option>
                <option value="tls" selected=initial_tls_mode == SmtpTlsMode::Tls>
                    "TLS"
                </option>
            </select>
        </label>
        <ValidatedInput<SmtpSender>
            label="Sender Mailbox"
            name="sender"
            field=state.sender
            field_class="j-site-field j-site-field-wide"
            class="j-site-input"
            help="An email address, optionally with a display name."
        />
    }
}

fn smtp_authentication_fields(
    state: SmtpFormState,
    password_error: Memo<Option<String>>,
    password_input: NodeRef<html::Input>,
) -> impl IntoView {
    view! {
        <label class="j-site-field j-site-field-wide">
            <input
                type="checkbox"
                name="authentication_enabled"
                prop:checked=state.authentication_enabled
                prop:disabled=move || !state.enabled.get()
                on:change=move |event| {
                    let next = event_target_checked(&event);
                    state.authentication_enabled.set(next);
                    if !next {
                        clear_password(state, password_input);
                    }
                }
            />
            <span class="j-form-label">"Use authentication"</span>
        </label>
        <ValidatedInput<SmtpUsername>
            label="Username"
            name="username"
            field=state.username
            field_class="j-site-field j-site-field-wide"
            class="j-site-input"
        />
        <label class="j-site-field j-site-field-wide">
            <span class="j-form-label">"Password"</span>
            <input
                node_ref=password_input
                class="j-site-input"
                type="password"
                name="password"
                autocomplete="new-password"
                prop:value=move || state.password.value()
                prop:disabled=move || !state.enabled.get() || !state.authentication_enabled.get()
                aria-describedby="smtp-password-help"
                on:input=move |event| state.password.set_value(&event_target_value(&event))
                on:blur=move |_| state.password.touch()
            />
            <span id="smtp-password-help" class="j-form-help">
                {if state.password_configured {
                    "A password is configured. Leave blank to keep it, or enter a replacement."
                } else {
                    "No password is configured. Enter one to enable authentication."
                }}
            </span>
            {crate::forms::validated_error(
                password_error,
                Signal::derive(move || state.password.is_touched()),
                |message| view! { <span class="error">{message}</span> }.into_any(),
            )}
        </label>
    }
}

fn smtp_settings_form(
    settings: &super::Settings,
    update_action: ServerAction<UpdateSettings>,
) -> impl IntoView {
    let state = SmtpFormState::new(settings);
    let password_input = NodeRef::new();
    let initial_tls_mode = settings.tls_mode;
    let password_error = Memo::new(move |_| state.password_error());
    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        if !state.can_submit(update_action.pending().get()) {
            return;
        }
        if let Some(request) = state
            .draft()
            .and_then(|draft| assemble_request(state, draft))
        {
            update_action.dispatch(UpdateSettings { request });
            clear_password(state, password_input);
        }
    };

    view! {
        <form class="j-card j-site-form" on:submit=submit>
            <div class="j-card-head">
                <div>
                    <h2>"SMTP Relay"</h2>
                    <div class="j-sub">"Configure the outbound relay used for Jaunder email."</div>
                </div>
            </div>
            <div class="j-site-form-body">
                <label class="j-site-field j-site-field-wide">
                    <input
                        type="checkbox"
                        name="enabled"
                        prop:checked=state.enabled
                        on:change=move |event| {
                            let next = event_target_checked(&event);
                            state.enabled.set(next);
                            if !next {
                                state.authentication_enabled.set(false);
                                clear_password(state, password_input);
                            }
                        }
                    />
                    <span class="j-form-label">"Enable SMTP relay"</span>
                </label>
                {smtp_relay_fields(state, initial_tls_mode)}
                {smtp_authentication_fields(state, password_error, password_input)}
            </div>
            <div class="j-site-form-actions">
                <button
                    type="submit"
                    class="j-btn is-primary"
                    prop:disabled=move || !state.can_submit(update_action.pending().get())
                >
                    "Save SMTP Settings"
                </button>
            </div>
        </form>
    }
}
