use std::fmt::{Debug, Display, Formatter};

/// Owned third-party detail approved only for a user-facing response.
///
/// The wrapper deliberately has no `Display` or
/// [`TraceField`](crate::trace_field::TraceField) implementation. Callers must
/// choose [`Self::as_str`] at the user sink, while incidental debug output stays
/// redacted.
pub struct UserFacingMessage(String);

impl UserFacingMessage {
    /// Capture intentionally exposed third-party detail at its reviewed source
    /// boundary.
    #[must_use]
    pub fn from_external(value: impl Display) -> Self {
        Self(value.to_string())
    }

    /// The detail approved for the submitting user's response.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Debug for UserFacingMessage {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("UserFacingMessage([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::UserFacingMessage;

    #[test]
    fn external_display_is_available_only_through_the_user_surface() {
        // server-fn-wire-arg-error:allow exercise the reviewed external Display capture door
        let message = UserFacingMessage::from_external(format_args!("detail {}", 42));
        assert_eq!(message.as_str(), "detail 42");
        assert_eq!(format!("{message:?}"), "UserFacingMessage([redacted])");
    }
}
