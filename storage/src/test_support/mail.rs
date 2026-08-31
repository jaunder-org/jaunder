use common::mailer::{MailSender, NoopMailSender};
use std::sync::Arc;

/// Default mailer for tests that don't care about email sending.
#[must_use]
pub fn noop_mailer() -> Arc<dyn MailSender> {
    Arc::new(NoopMailSender)
}
