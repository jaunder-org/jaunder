//! Host-tested SMTP form state and non-secret submission decisions.
//!
//! Browser-only input references, event handlers, and secret request assembly
//! remain in `component`; this module owns every decision that determines
//! whether the aggregate can dispatch.

use super::Settings;
use crate::forms::Field;
use common::smtp_host::SmtpHost;
use common::smtp_password::SmtpPasswordShape;
use common::smtp_port::SmtpPort;
use common::smtp_sender::SmtpSender;
use common::smtp_tls_mode::SmtpTlsMode;
use common::smtp_username::SmtpUsername;
use leptos::prelude::*;
/// Whether the component must stage a replacement password in the request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmtpPasswordIntent {
    /// A blank field preserves the password currently stored by the aggregate.
    Keep,
    /// A non-blank field must be converted and staged by the browser component.
    Replace,
}

/// A validated, secret-free description of the SMTP mutation to dispatch.
#[derive(Clone, Debug)]
pub enum SmtpUpdateDraft {
    /// The aggregate must be disabled and cleared.
    Disabled { password: SmtpPasswordIntent },
    /// The aggregate must be saved with the supplied visible values.
    Enabled {
        host: SmtpHost,
        port: SmtpPort,
        tls_mode: SmtpTlsMode,
        sender: SmtpSender,
        authentication_enabled: bool,
        username: Option<SmtpUsername>,
        password: SmtpPasswordIntent,
    },
}

/// The target-independent fields and decisions behind the SMTP settings form.
#[derive(Clone, Copy)]
pub struct SmtpFormState {
    pub(crate) enabled: RwSignal<bool>,
    pub(crate) host: Field<SmtpHost>,
    pub(crate) port: Field<SmtpPort>,
    pub(crate) tls_mode: RwSignal<SmtpTlsMode>,
    pub(crate) sender: Field<SmtpSender>,
    pub(crate) authentication_enabled: RwSignal<bool>,
    pub(crate) username: Field<SmtpUsername>,
    pub(crate) password: Field<SmtpPasswordShape>,
    pub(crate) password_configured: bool,
}

impl SmtpFormState {
    /// Seeds state from the secret-free settings read model.
    #[must_use]
    pub fn new(settings: &Settings) -> Self {
        Self {
            enabled: RwSignal::new(settings.enabled),
            host: Field::prefilled(settings.host.as_deref().unwrap_or_default()),
            port: Field::prefilled(&settings.port.to_string()),
            tls_mode: RwSignal::new(settings.tls_mode),
            sender: Field::prefilled(settings.sender.as_ref()),
            authentication_enabled: RwSignal::new(settings.authentication_enabled),
            username: Field::prefilled(settings.username.as_deref().unwrap_or_default()),
            // Required semantics preserve whitespace-only passwords. Blank is
            // handled separately as keep/absent intent; `Field::optional`
            // deliberately treats whitespace-only text as absence.
            password: Field::new(),
            password_configured: settings.password_configured,
        }
    }

    /// Clears the raw password staging field after every dispatch attempt.
    pub fn clear_password(self) {
        self.password.reset();
    }

    /// Returns whether the current field values may dispatch.
    #[must_use]
    pub fn can_submit(self, pending: bool) -> bool {
        if pending {
            return false;
        }
        if !self.enabled.get() {
            return self.password.value().is_empty();
        }
        if !self.host.is_valid() || !self.port.is_valid() || !self.sender.is_valid() {
            return false;
        }
        if !self.authentication_enabled.get() {
            return self.password.value().is_empty();
        }
        let raw_password = self.password.value();
        self.username.is_valid()
            && if raw_password.is_empty() {
                self.password_configured
            } else {
                self.password.is_valid()
            }
    }

    /// Returns the password field's conditional validation error.
    #[must_use]
    pub fn password_error(self) -> Option<String> {
        if self.password_configured && self.password.value().is_empty() {
            None
        } else {
            self.password.error().get()
        }
    }

    /// Builds a secret-free draft for the browser component to assemble.
    #[must_use]
    pub fn draft(self) -> Option<SmtpUpdateDraft> {
        let password = if self.password.value().is_empty() {
            SmtpPasswordIntent::Keep
        } else {
            SmtpPasswordIntent::Replace
        };
        if !self.enabled.get() {
            return Some(SmtpUpdateDraft::Disabled { password });
        }

        Some(SmtpUpdateDraft::Enabled {
            host: self.host.parsed()?,
            port: self.port.parsed()?,
            tls_mode: self.tls_mode.get(),
            sender: self.sender.parsed()?,
            authentication_enabled: self.authentication_enabled.get(),
            username: self
                .authentication_enabled
                .get()
                .then(|| self.username.parsed())
                .flatten(),
            password,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            enabled: true,
            host: Some("relay.example.com".parse().unwrap()),
            port: "2525".parse().unwrap(),
            tls_mode: SmtpTlsMode::Tls,
            sender: "Jaunder <mail@example.com>".parse().unwrap(),
            authentication_enabled: true,
            username: Some("relay-user".parse().unwrap()),
            password_configured: false,
        }
    }

    #[test]
    fn disabled_form_needs_an_empty_password_and_assembles_a_disabled_draft() {
        Owner::new().with(|| {
            let state = SmtpFormState::new(&Settings::default());
            assert!(state.can_submit(false));
            assert!(matches!(
                state.draft(),
                Some(SmtpUpdateDraft::Disabled {
                    password: SmtpPasswordIntent::Keep
                })
            ));
        });
    }

    #[test]
    fn unauthenticated_form_rejects_password_staging() {
        Owner::new().with(|| {
            let mut initial = settings();
            initial.authentication_enabled = false;
            let state = SmtpFormState::new(&initial);
            assert!(state.can_submit(false));
            state.password.set_value("replacement");
            assert!(!state.can_submit(false));
        });
    }

    #[test]
    fn pending_or_invalid_visible_fields_block_submission() {
        Owner::new().with(|| {
            let state = SmtpFormState::new(&settings());
            state.password.set_value("replacement");
            assert!(!state.can_submit(true));

            state.host.set_value("");
            assert!(!state.can_submit(false));
        });
    }

    #[test]
    fn existing_password_allows_keep_with_an_empty_replacement_field() {
        Owner::new().with(|| {
            let mut initial = settings();
            initial.password_configured = true;
            let state = SmtpFormState::new(&initial);
            assert!(state.can_submit(false));
            assert!(matches!(
                state.draft(),
                Some(SmtpUpdateDraft::Enabled {
                    authentication_enabled: true,
                    password: SmtpPasswordIntent::Keep,
                    ..
                })
            ));
        });
    }

    #[test]
    fn new_password_assembles_replace_intent() {
        Owner::new().with(|| {
            let state = SmtpFormState::new(&settings());
            state.password.set_value("replacement");
            assert!(state.can_submit(false));
            assert!(matches!(
                state.draft(),
                Some(SmtpUpdateDraft::Enabled {
                    password: SmtpPasswordIntent::Replace,
                    ..
                })
            ));
        });
    }

    #[test]
    fn clearing_password_resets_staged_value_and_conditional_error() {
        Owner::new().with(|| {
            let state = SmtpFormState::new(&settings());
            assert!(state.password_error().is_some());
            state.password.set_value("replacement");
            state.clear_password();
            assert!(state.password.value().is_empty());

            let mut configured = settings();
            configured.password_configured = true;
            assert_eq!(SmtpFormState::new(&configured).password_error(), None);
        });
    }

    #[test]
    fn missing_credential_sides_block_an_authenticated_submission() {
        Owner::new().with(|| {
            let mut missing_username = settings();
            missing_username.username = None;
            let state = SmtpFormState::new(&missing_username);
            state.password.set_value("replacement");
            assert!(!state.can_submit(false));

            let state = SmtpFormState::new(&settings());
            assert!(!state.can_submit(false));
        });
    }

    #[test]
    fn password_staging_preserves_exact_submitted_text_without_secret_exposure() {
        Owner::new().with(|| {
            let state = SmtpFormState::new(&settings());
            state.password.set_value("  replacement with spaces  ");
            assert_eq!(state.password.value(), "  replacement with spaces  ");
            assert!(matches!(
                state.draft(),
                Some(SmtpUpdateDraft::Enabled {
                    password: SmtpPasswordIntent::Replace,
                    ..
                })
            ));
        });
    }
}
