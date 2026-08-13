use thiserror::Error;

/// An `AtomPub` document could not be **written**.
///
/// There is deliberately no read counterpart. `atom_syndication` owns parsing, so
/// a document the client sent that will not parse surfaces as [`AtomError`] and
/// each consumer maps it at its own boundary (the server: a `400`). Failing to
/// write a document we composed ourselves is never the client's fault, so the two
/// directions are separate types rather than two variants of one — which is what
/// keeps a serialization failure off the `400` path.
///
/// [`AtomError`]: crate::atompub::AtomError
#[derive(Debug, Error)]
#[error("failed to serialize AtomPub document: {0}")]
pub struct AtomPubError(String);

impl AtomPubError {
    /// Wraps the cause of a failed write.
    #[must_use]
    pub fn new(cause: impl Into<String>) -> Self {
        Self(cause.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_error_displays_its_cause() {
        assert!(AtomPubError::new("boom").to_string().contains("boom"));
    }
}
