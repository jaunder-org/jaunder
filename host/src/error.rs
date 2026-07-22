//! The server-side error carrier: a structured, queryable operator payload
//! (`kind`, `class`, `context`, preserved `source` cause chain) plus the exact
//! wire `public_message`, decoupled from any wire type. `web` projects the
//! carrier's `(kind, public_message)` to its outward wire type at the
//! server-fn boundary; the operator-side payload has no projection and so is
//! structurally absent from what can cross the wire.
//!
//! `host` never compiles to wasm (ADR-0058), so this whole module is
//! unconditional — no `#[cfg(feature = "server")]` gating.

use std::error::Error;

pub type InternalResult<T> = Result<T, InternalError>;

/// The category of an internal failure, derived at construction. Drives
/// outward mapping and is emitted as a discrete `error.kind` field at the
/// boundary for queryable triage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Auth,
    NotFound,
    Validation,
    Conflict,
    Storage,
    Internal,
    /// Downstream dependency (mail, `WebSub`, …).
    External,
}

impl ErrorKind {
    /// The bounded `error.kind` attribute value emitted on the `jaunder.errors`
    /// metric — the same stable names logged as the boundary's `error.kind`
    /// field, kept low-cardinality by construction.
    fn as_metric_str(self) -> &'static str {
        match self {
            ErrorKind::Auth => "auth",
            ErrorKind::NotFound => "not_found",
            ErrorKind::Validation => "validation",
            ErrorKind::Conflict => "conflict",
            ErrorKind::Storage => "storage",
            ErrorKind::Internal => "internal",
            ErrorKind::External => "external",
        }
    }
}

/// Operational severity, derived at construction so triage (and the
/// boundary's log level) is mechanical rather than guessed from the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// Expected 4xx (validation, not-found, unauthorized) — never alert.
    Client,
    /// Retryable infrastructure failure. Not produced by `web` itself (which
    /// only sees opaque/typed errors); reserved for classification nearer the
    /// source.
    Transient,
    /// "Can't happen" invariant violation or opaque internal failure — page.
    Bug,
    /// Downstream dependency failure. Reserved (see `ErrorKind::External`).
    External,
}

impl ErrorClass {
    /// The tracing level the boundary logs this class at.
    #[must_use]
    pub fn log_level(self) -> tracing::Level {
        match self {
            ErrorClass::Client => tracing::Level::DEBUG,
            ErrorClass::Transient | ErrorClass::External => tracing::Level::WARN,
            ErrorClass::Bug => tracing::Level::ERROR,
        }
    }

    /// The bounded `error.class` attribute value emitted on the `jaunder.errors`
    /// metric — the same stable names logged as the boundary's `error.class`
    /// field, kept low-cardinality by construction.
    fn as_metric_str(self) -> &'static str {
        match self {
            ErrorClass::Client => "client",
            ErrorClass::Transient => "transient",
            ErrorClass::Bug => "bug",
            ErrorClass::External => "external",
        }
    }
}

/// Server-side error carrier: the exact wire `public_message` plus structured,
/// queryable operator data (`kind`, `class`, `context`) and the preserved
/// `source` cause chain (carried via `anyhow`, never stringified eagerly). The
/// outward wire type is *derived* by `web` from `(kind, public_message)` at the
/// boundary — the carrier holds no wire type, so the operator-side payload is
/// structurally absent from what can cross the wire.
#[derive(Debug)]
pub struct InternalError {
    kind: ErrorKind,
    class: ErrorClass,
    context: Vec<(&'static str, String)>,
    public_message: String,
    source: Option<anyhow::Error>,
}

/// A transparent [`Error`] wrapper around a `Box<dyn Error + Send + Sync>` so an
/// already-boxed error can be carried as an `anyhow` source (the box itself does
/// not implement `Error`). Forwards `Display` and `source`, so it is invisible
/// in the cause chain.
#[derive(Debug)]
struct BoxedError(Box<dyn Error + Send + Sync>);

impl std::fmt::Display for BoxedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl Error for BoxedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

impl InternalError {
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::masked(
            ErrorKind::Auth,
            ErrorClass::Client,
            String::new(),
            anyhow::Error::msg(message.into()),
        )
    }

    pub fn not_found(resource: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::NotFound,
            class: ErrorClass::Client,
            context: Vec::new(),
            public_message: format!("{} not found", resource.into()),
            source: None,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Validation,
            class: ErrorClass::Client,
            context: Vec::new(),
            public_message: message.into(),
            source: None,
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Conflict,
            class: ErrorClass::Client,
            context: Vec::new(),
            public_message: message.into(),
            source: None,
        }
    }

    pub fn storage(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            kind: ErrorKind::Storage,
            class: ErrorClass::Bug,
            context: Vec::new(),
            public_message: "storage operation failed".to_string(),
            source: Some(anyhow::Error::new(error)),
        }
    }

    pub fn server(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            kind: ErrorKind::Internal,
            class: ErrorClass::Bug,
            context: Vec::new(),
            public_message: "server operation failed".to_string(),
            source: Some(anyhow::Error::new(error)),
        }
    }

    /// Like [`Self::server`] but for an already-boxed error. `Box<dyn Error + ...>`
    /// does not itself implement `Error` (so it can't go through `server`), and
    /// this anyhow build has no `From<Box<dyn Error + ...>>`; a transparent
    /// wrapper carries it as a structured source, preserving its cause chain for
    /// operator logs instead of flattening it to a string.
    #[must_use]
    pub fn server_boxed(error: Box<dyn Error + Send + Sync>) -> Self {
        Self::server(BoxedError(error))
    }

    pub fn server_message(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Internal,
            class: ErrorClass::Bug,
            context: Vec::new(),
            public_message: "server operation failed".to_string(),
            source: Some(anyhow::Error::msg(message.into())),
        }
    }

    /// A downstream dependency failure (mail, `WebSub`, …). Masks as a 500
    /// outwardly but classes as `External` so a dependency outage is
    /// distinguishable from a Jaunder bug during triage.
    pub fn external(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            kind: ErrorKind::External,
            class: ErrorClass::External,
            context: Vec::new(),
            public_message: "server operation failed".to_string(),
            source: Some(anyhow::Error::new(error)),
        }
    }

    /// Constructs a masked error directly from its projected `(kind, class)`, the
    /// exact wire `public_message`, and an operator-only `source`. The public and
    /// operator sides are supplied independently, so the source cause chain stays
    /// on the operator side and is never inferred from the wire message.
    pub fn masked(
        kind: ErrorKind,
        class: ErrorClass,
        public_message: impl Into<String>,
        source: anyhow::Error,
    ) -> Self {
        Self {
            kind,
            class,
            context: Vec::new(),
            public_message: public_message.into(),
            source: Some(source),
        }
    }

    /// Lifts a typed error into a `Validation` (client / 400) carrier while
    /// *supplementing* it with a site-specific `public_message`: the message crosses
    /// the wire, the typed `source` is preserved on the operator side (downcastable),
    /// never flattened via `to_string()`. Use this at a call site when the public
    /// message needs context the error type doesn't carry (which field failed to parse);
    /// use a `From` impl / the `validation_from!` macro when the type has one canonical
    /// lift. This is the single home of the masked-validation-with-source shape — the
    /// macro and the storage `From` impls delegate here.
    pub fn validation_source(
        public_message: impl Into<String>,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self::masked(
            ErrorKind::Validation,
            ErrorClass::Client,
            public_message,
            anyhow::Error::new(source),
        )
    }

    /// Attaches a structured key/value to the operator-side context, emitted
    /// at the boundary (see `emit_boundary_failure`). Never reaches the client.
    #[must_use]
    pub fn with_context(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.context.push((key, value.into()));
        self
    }

    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    #[must_use]
    pub fn class(&self) -> ErrorClass {
        self.class
    }

    #[must_use]
    pub fn context(&self) -> &[(&'static str, String)] {
        &self.context
    }

    /// The exact wire message for this error's `kind`, projected to a wire type
    /// by `web` at the boundary. Empty for kinds whose wire variant carries no
    /// message (e.g. `Auth` → unauthorized).
    #[must_use]
    pub fn public_message(&self) -> &str {
        &self.public_message
    }

    /// Renders the operator-facing detail (the preserved source cause chain,
    /// falling back to the public message). For logs and re-masking only;
    /// never sent to the client.
    #[must_use]
    pub fn operator_message(&self) -> String {
        match &self.source {
            Some(source) => format!("{source:#}"),
            None => self.public_message.clone(),
        }
    }

    /// Emits the structured boundary observability for a failed server function:
    /// discrete, queryable tracing fields (not one concatenated string) at the
    /// level derived from the error class, and the `jaunder.errors` metric with
    /// the bounded `error.kind`/`error.class` attributes. `context` is emitted as
    /// a single serialized field; promoting each k/v to a span field is deferred
    /// to §4.6 (kq8w.22). Called by `web`'s `server_boundary`; the outward wire
    /// projection stays in `web`.
    pub fn emit_boundary_failure(&self, server_fn: &'static str) {
        // Render the preserved cause chain once; empty when there is no source
        // (e.g. pure client errors).
        let source = self
            .source
            .as_ref()
            .map(|s| format!("{s:#}"))
            .unwrap_or_default();
        macro_rules! emit {
            ($macro:ident) => {
                tracing::$macro!(
                    server_fn,
                    error.kind = ?self.kind,
                    error.class = ?self.class,
                    error.public = %self.public_message,
                    error.source = %source,
                    error.context = ?self.context,
                    "server function failed",
                )
            };
        }
        // `ErrorClass::log_level` is the single source of truth; the match only
        // exists because `tracing`'s macros require a statically-known level.
        match self.class.log_level() {
            tracing::Level::DEBUG => emit!(debug),
            tracing::Level::WARN => emit!(warn),
            _ => emit!(error),
        }
        crate::metrics::error(self.kind.as_metric_str(), self.class.as_metric_str());
    }
}

// ---------------------------------------------------------------------------
// Typed `From` conversions (ADR-0017 §3, ADR-0058)
// ---------------------------------------------------------------------------
//
// Each lift reproduces the exact wire `public_message` its call sites produced
// before, so switching a site to bare `?`/`.into()` is behavior-preserving; the
// `(kind, class)` is fixed here, at the conversion's home, so a site can never
// silently move the wire class by switching to `?`. The improvement is purely
// operator-side: the *typed* source is now preserved on the `anyhow` chain
// instead of being eagerly stringified (ADR-0017 §3, A19).

impl From<sqlx::Error> for InternalError {
    /// A storage-driver failure: masks as `"storage operation failed"` (kind
    /// `Storage`, class `Bug`) while preserving the `sqlx::Error` as a typed,
    /// downcastable source. Behavior-identical to `InternalError::storage(error)`.
    fn from(error: sqlx::Error) -> Self {
        Self::storage(error)
    }
}

impl From<common::mailer::MailError> for InternalError {
    /// A mail-transport failure. Matches the pre-existing
    /// `mailer.send_email(...).map_err(InternalError::server)` classification
    /// (kind `Internal`, class `Bug`, public `"server operation failed"`) while
    /// preserving the typed `MailError` (and its boxed transport source) for
    /// operator logs.
    fn from(error: common::mailer::MailError) -> Self {
        Self::server(error)
    }
}

/// Generates `From<T> for InternalError` for each `common` value-object
/// parse/validation error `T`: kind `Validation`, class `Client`, public
/// message = the source's `Display` (byte-for-byte what the old
/// `.map_err(|e| InternalError::validation(e.to_string()))` lifts produced),
/// with the typed source now preserved on the operator side (A19).
macro_rules! validation_from {
    ($($ty:ty),+ $(,)?) => {$(
        impl From<$ty> for InternalError {
            fn from(error: $ty) -> Self {
                Self::validation_source(error.to_string(), error)
            }
        }
    )+};
}

validation_from!(
    common::slug::InvalidSlug,
    common::username::InvalidUsername,
    common::tag::InvalidTag,
    common::tag::TagValidationError,
    common::password::PasswordError,
    common::render::InvalidPostFormat,
    common::media::InvalidMediaSource,
);

#[cfg(test)]
mod tests {
    use super::{ErrorClass, ErrorKind, InternalError};
    use std::error::Error;
    use std::fmt;

    #[derive(Debug)]
    struct SourceError;

    impl fmt::Display for SourceError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("source context")
        }
    }

    impl Error for SourceError {}

    #[derive(Debug)]
    struct OuterError {
        source: SourceError,
    }

    impl fmt::Display for OuterError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("outer failure")
        }
    }

    impl Error for OuterError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.source)
        }
    }

    #[test]
    fn constructors_set_kind_and_class() {
        let unauth = InternalError::unauthorized("nope");
        assert_eq!(unauth.kind(), ErrorKind::Auth);
        assert_eq!(unauth.class(), ErrorClass::Client);

        let validation = InternalError::validation("bad");
        assert_eq!(validation.kind(), ErrorKind::Validation);
        assert_eq!(validation.class(), ErrorClass::Client);

        let not_found = InternalError::not_found("Post");
        assert_eq!(not_found.kind(), ErrorKind::NotFound);
        assert_eq!(not_found.class(), ErrorClass::Client);

        let conflict = InternalError::conflict("dup");
        assert_eq!(conflict.kind(), ErrorKind::Conflict);
        assert_eq!(conflict.class(), ErrorClass::Client);

        let storage = InternalError::storage(OuterError {
            source: SourceError,
        });
        assert_eq!(storage.kind(), ErrorKind::Storage);
        assert_eq!(storage.class(), ErrorClass::Bug);

        let server = InternalError::server(OuterError {
            source: SourceError,
        });
        assert_eq!(server.kind(), ErrorKind::Internal);
        assert_eq!(server.class(), ErrorClass::Bug);
    }

    #[test]
    fn constructors_set_public_message() {
        assert_eq!(
            InternalError::not_found("Post").public_message(),
            "Post not found"
        );
        assert_eq!(
            InternalError::validation("bad input").public_message(),
            "bad input"
        );
        assert_eq!(
            InternalError::conflict("already exists").public_message(),
            "already exists"
        );
    }

    #[test]
    fn unauthorized_masks_public_and_preserves_operator() {
        let error = InternalError::unauthorized("not allowed");
        assert_eq!(error.kind(), ErrorKind::Auth);
        assert_eq!(error.class(), ErrorClass::Client);
        // The wire variant carries no message, so the public side is empty.
        assert_eq!(error.public_message(), "");
        // The construction message is retained on the operator side only.
        assert_eq!(error.operator_message(), "not allowed");
    }

    #[test]
    fn masking_constructors_keep_public_generic_and_operator_detailed() {
        let storage = InternalError::storage(OuterError {
            source: SourceError,
        });
        assert_eq!(storage.public_message(), "storage operation failed");
        assert!(storage.operator_message().contains("source context"));

        let server = InternalError::server(OuterError {
            source: SourceError,
        });
        assert_eq!(server.public_message(), "server operation failed");
        assert_eq!(server.operator_message(), "outer failure: source context");

        let server_message = InternalError::server_message("operator-only context");
        assert_eq!(server_message.public_message(), "server operation failed");
        assert_eq!(server_message.operator_message(), "operator-only context");
    }

    #[test]
    fn external_constructor_sets_external_kind_class_and_masks_public() {
        let error = InternalError::external(OuterError {
            source: SourceError,
        });
        assert_eq!(error.kind(), ErrorKind::External);
        assert_eq!(error.class(), ErrorClass::External);
        assert_eq!(error.public_message(), "server operation failed");
        assert_eq!(error.operator_message(), "outer failure: source context");
    }

    #[test]
    fn masked_keeps_public_and_operator_messages_separate() {
        let error = InternalError::masked(
            ErrorKind::NotFound,
            ErrorClass::Client,
            "Post not found",
            anyhow::Error::msg("draft access denied for missing session token"),
        );
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert_eq!(error.class(), ErrorClass::Client);
        assert_eq!(error.public_message(), "Post not found");
        assert_eq!(
            error.operator_message(),
            "draft access denied for missing session token"
        );
    }

    #[test]
    fn server_boxed_preserves_source_chain_not_stringified() {
        let boxed: Box<dyn Error + Send + Sync> = Box::new(OuterError {
            source: SourceError,
        });
        let error = InternalError::server_boxed(boxed);
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(error.class(), ErrorClass::Bug);
        assert_eq!(error.public_message(), "server operation failed");
        // The transparent `BoxedError` wrapper forwards `Display`/`source`, so the
        // preserved cause chain still renders via the operator message.
        assert_eq!(error.operator_message(), "outer failure: source context");
    }

    #[test]
    fn storage_error_captures_source_chain_not_stringified() {
        let error = InternalError::storage(OuterError {
            source: SourceError,
        });
        // The operator-facing rendering still walks the cause chain (now via
        // the preserved anyhow source instead of an eager concatenation).
        assert_eq!(error.operator_message(), "outer failure: source context");
    }

    #[test]
    fn client_error_operator_message_falls_back_to_public() {
        // A client error carries no source, so the operator rendering falls
        // back to the public message.
        let error = InternalError::not_found("Post");
        assert_eq!(error.operator_message(), "Post not found");
    }

    #[test]
    fn with_context_accumulates_pairs_in_order() {
        let error = InternalError::server_message("boom")
            .with_context("post_id", "42")
            .with_context("user_id", "7");
        assert_eq!(
            error.context(),
            &[("post_id", "42".to_string()), ("user_id", "7".to_string()),]
        );
    }

    #[test]
    fn error_class_maps_to_log_level() {
        use tracing::Level;
        assert_eq!(ErrorClass::Client.log_level(), Level::DEBUG);
        assert_eq!(ErrorClass::Transient.log_level(), Level::WARN);
        assert_eq!(ErrorClass::External.log_level(), Level::WARN);
        assert_eq!(ErrorClass::Bug.log_level(), Level::ERROR);
    }

    #[test]
    fn error_kind_and_class_metric_strings_are_stable_and_bounded() {
        // Every variant maps to a fixed, low-cardinality attribute value; these
        // are the strings emitted on the `jaunder.errors` metric at the boundary.
        assert_eq!(ErrorKind::Auth.as_metric_str(), "auth");
        assert_eq!(ErrorKind::NotFound.as_metric_str(), "not_found");
        assert_eq!(ErrorKind::Validation.as_metric_str(), "validation");
        assert_eq!(ErrorKind::Conflict.as_metric_str(), "conflict");
        assert_eq!(ErrorKind::Storage.as_metric_str(), "storage");
        assert_eq!(ErrorKind::Internal.as_metric_str(), "internal");
        assert_eq!(ErrorKind::External.as_metric_str(), "external");
        assert_eq!(ErrorClass::Client.as_metric_str(), "client");
        assert_eq!(ErrorClass::Transient.as_metric_str(), "transient");
        assert_eq!(ErrorClass::Bug.as_metric_str(), "bug");
        assert_eq!(ErrorClass::External.as_metric_str(), "external");
    }

    #[test]
    fn emit_boundary_failure_emits_at_class_derived_level() {
        use tracing_subscriber::fmt;

        // An active subscriber forces the tracing macros to evaluate their
        // fields (covering the field-formatting lines and every level arm).
        let subscriber = fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        // Bug → ERROR arm, source present (the `Some` render branch).
        InternalError::server(OuterError {
            source: SourceError,
        })
        .emit_boundary_failure("test_fn");
        // Client → DEBUG arm, no source (the `None`/`unwrap_or_default` branch).
        InternalError::validation("bad input").emit_boundary_failure("test_fn");
        // External → WARN arm.
        InternalError::external(OuterError {
            source: SourceError,
        })
        .emit_boundary_failure("test_fn");
    }

    #[test]
    fn from_sqlx_error_matches_storage_constructor() {
        // `?` on a `sqlx::Error` produces exactly `InternalError::storage(err)`.
        let error: InternalError = sqlx::Error::RowNotFound.into();
        assert_eq!(error.kind(), ErrorKind::Storage);
        assert_eq!(error.class(), ErrorClass::Bug);
        // Same wire projection inputs as `InternalError::storage(...)`.
        assert_eq!(error.public_message(), "storage operation failed");
        // The typed `sqlx::Error` is preserved on the operator side, not the wire.
        assert!(error.operator_message().contains("no rows returned"));
    }

    #[test]
    fn from_mail_error_matches_server_constructor() {
        // Mirrors `send_email(...).map_err(InternalError::server)`: Internal/Bug,
        // masked public, typed `MailError` preserved for operators.
        let error: InternalError = common::mailer::MailError::NotConfigured.into();
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(error.class(), ErrorClass::Bug);
        assert_eq!(error.public_message(), "server operation failed");
        assert!(error
            .operator_message()
            .contains("mail sender is not configured"));
    }

    #[test]
    fn from_common_validation_sources_preserve_display_as_public_and_are_client() {
        // Each common value-object parse error lifts to Validation/Client with
        // the source's `Display` as the wire message (identical to the old
        // `.map_err(|e| InternalError::validation(e.to_string()))`) and the
        // typed source preserved on the operator side.
        macro_rules! check {
            ($value:expr) => {{
                let display = $value.to_string();
                let error: InternalError = $value.into();
                assert_eq!(error.kind(), ErrorKind::Validation);
                assert_eq!(error.class(), ErrorClass::Client);
                assert_eq!(error.public_message(), display);
                assert!(error.operator_message().contains(&display));
            }};
        }
        check!(common::slug::InvalidSlug);
        check!(common::username::InvalidUsername);
        check!(common::tag::InvalidTag);
        check!(common::tag::TagValidationError::TooMany { count: 99, max: 25 });
        check!(common::password::PasswordError::PasswordTooShort);
        check!(common::render::InvalidPostFormat);
        check!(common::media::InvalidMediaSource);
    }

    #[test]
    fn masked_pairs_a_site_validation_message_with_a_typed_source() {
        // Mirrors the chrono parse call sites: a specific public message plus a
        // typed source on the anyhow chain. `host` has no `chrono` dep, so a
        // stand-in typed error stands for `chrono::ParseError`; the real chrono
        // wiring is guarded by the web suite.
        let error = InternalError::masked(
            ErrorKind::Validation,
            ErrorClass::Client,
            "invalid publish_at: premature end of input",
            anyhow::Error::new(OuterError {
                source: SourceError,
            }),
        );
        assert_eq!(error.kind(), ErrorKind::Validation);
        assert_eq!(error.class(), ErrorClass::Client);
        assert_eq!(
            error.public_message(),
            "invalid publish_at: premature end of input"
        );
        // Typed source on the operator side, downcastable, not stringified onto
        // the wire.
        assert!(error.operator_message().contains("outer failure"));
    }
}
