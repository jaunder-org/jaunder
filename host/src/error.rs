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
use std::sync::LazyLock;

use common::client_telemetry::ClientSourceKind;
use opentelemetry::metrics::Counter;
use opentelemetry::{KeyValue, global};
use tracing_error::SpanTrace;

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

/// Whether an error escaped through a public boundary or was intentionally
/// continued after reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorDisposition {
    /// The primary operation returned the failure.
    Boundary,
    /// The primary outcome was preserved after the failure was reported.
    Swallowed,
}

impl ErrorDisposition {
    fn as_str(self) -> &'static str {
        match self {
            Self::Boundary => "boundary",
            Self::Swallowed => "swallowed",
        }
    }
}

/// The bounded runtime in which the error event originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryOrigin {
    /// Native server or command execution.
    Server,
    /// Authenticated browser-side intake.
    Client,
}

impl TelemetryOrigin {
    fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Client => "client",
        }
    }
}

/// The reviewed source attached to a continued-after-error event.
///
/// `Error` is reserved for source chains whose rendering is known to be
/// PII-safe. `Redacted` records that a source existed without accepting
/// arbitrary caller text.
#[derive(Clone, Copy)]
pub enum SwallowedSource<'a> {
    /// A reviewed, PII-safe typed source chain.
    Error(&'a (dyn Error + 'static)),
    /// A source whose text must not be exported.
    Redacted,
}
static ERROR_COUNTER: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter("jaunder")
        .u64_counter("jaunder.errors")
        .build()
});

fn record_error(
    kind: ErrorKind,
    class: ErrorClass,
    disposition: ErrorDisposition,
    origin: TelemetryOrigin,
) {
    ERROR_COUNTER.add(
        1,
        &[
            KeyValue::new("error.kind", kind.as_metric_str()),
            KeyValue::new("error.class", class.as_metric_str()),
            KeyValue::new("error.disposition", disposition.as_str()),
            KeyValue::new("telemetry.origin", origin.as_str()),
        ],
    );
}

fn render_source(source: &(dyn Error + 'static)) -> String {
    let mut rendered = source.to_string();
    let mut next = source.source();
    while let Some(cause) = next {
        rendered.push_str(": ");
        rendered.push_str(&cause.to_string());
        next = cause.source();
    }
    rendered
}

/// Atomically reports an unexpected native failure whose caller intentionally
/// preserves its primary outcome.
pub fn report_swallowed(
    kind: ErrorKind,
    class: ErrorClass,
    context: &'static str,
    source: SwallowedSource<'_>,
) {
    let source = match source {
        SwallowedSource::Error(source) => render_source(source),
        SwallowedSource::Redacted => "redacted".to_owned(),
    };
    let span_trace = SpanTrace::capture();
    record_error(
        kind,
        class,
        ErrorDisposition::Swallowed,
        TelemetryOrigin::Server,
    );
    tracing::warn!(
        error.kind = kind.as_metric_str(), // cov:ignore
        error.class = class.as_metric_str(), // cov:ignore
        error.disposition = "swallowed",
        telemetry.origin = "server",
        error.context = context,
        error.source = %source,
        error.span_trace = %span_trace,
        "error swallowed after reporting",
    );
}
fn client_source_kind_as_str(source_kind: ClientSourceKind) -> &'static str {
    match source_kind {
        ClientSourceKind::StorageUnavailable => "storage_unavailable",
        ClientSourceKind::StorageOperation => "storage_operation",
        ClientSourceKind::InvalidSeed => "invalid_seed",
        ClientSourceKind::DialogUnavailable => "dialog_unavailable",
        ClientSourceKind::FormDataCreate => "form_data_create",
        ClientSourceKind::FormDataAppend => "form_data_append",
    }
}

/// Atomically reports one accepted, bounded browser-side swallowed failure.
///
/// Unlike [`report_swallowed`], this interface cannot accept arbitrary source
/// text: the wire's closed source kind is mapped to a fixed tracing field here.
/// It also deliberately does not capture a server-side [`SpanTrace`]: the
/// failure happened in the browser, and the authenticated intake has only the
/// bounded client payload, not the original browser span stack.
pub fn report_client_swallowed(
    kind: ErrorKind,
    class: ErrorClass,
    context: &'static str,
    source_kind: ClientSourceKind,
) {
    let source_kind = client_source_kind_as_str(source_kind);
    let kind_name = kind.as_metric_str();
    let class_name = class.as_metric_str();
    tracing::warn!(
        error.kind = kind_name,
        error.class = class_name,
        error.disposition = "swallowed",
        telemetry.origin = "client",
        error.context = context,
        error.source_kind = source_kind,
        "client error swallowed after reporting",
    );
    record_error(
        kind,
        class,
        ErrorDisposition::Swallowed,
        TelemetryOrigin::Client,
    );
}

/// Server-side error carrier: the exact wire `public_message` plus structured,
/// queryable operator data (`kind`, `class`, `context`), a captured active span
/// stack, and the preserved `source` cause chain (carried via `anyhow`, never
/// stringified eagerly). The outward wire type is *derived* by `web` from
/// `(kind, public_message)` at the boundary — the carrier holds no wire type, so
/// the operator-side payload is structurally absent from what can cross the
/// wire.
#[derive(Debug)]
pub struct InternalError {
    kind: ErrorKind,
    class: ErrorClass,
    context: Vec<(&'static str, String)>,
    public_message: String,
    span_trace: SpanTrace,
    source: Option<anyhow::Error>,
}

impl std::fmt::Display for InternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Display is safe for a public boundary: operator sources are available
        // only through `Error::source`, never rendered here.
        f.write_str(&self.public_message)
    }
}

impl Error for InternalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn Error + 'static))
    }
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
    fn new(
        kind: ErrorKind,
        class: ErrorClass,
        public_message: impl Into<String>,
        source: Option<anyhow::Error>,
    ) -> Self {
        Self {
            kind,
            class,
            context: Vec::new(),
            public_message: public_message.into(),
            span_trace: SpanTrace::capture(),
            source,
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::masked(
            ErrorKind::Auth,
            ErrorClass::Client,
            String::new(),
            anyhow::Error::msg(message.into()),
        )
    }

    pub fn not_found(resource: impl Into<String>) -> Self {
        Self::new(
            ErrorKind::NotFound,
            ErrorClass::Client,
            format!("{} not found", resource.into()),
            None,
        )
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(
            ErrorKind::Validation,
            ErrorClass::Client,
            message.into(),
            None,
        )
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(
            ErrorKind::Conflict,
            ErrorClass::Client,
            message.into(),
            None,
        )
    }

    pub fn storage(error: impl Error + Send + Sync + 'static) -> Self {
        Self::new(
            ErrorKind::Storage,
            ErrorClass::Bug,
            "storage operation failed",
            Some(anyhow::Error::new(error)),
        )
    }

    pub fn server(error: impl Error + Send + Sync + 'static) -> Self {
        Self::new(
            ErrorKind::Internal,
            ErrorClass::Bug,
            "server operation failed",
            Some(anyhow::Error::new(error)),
        )
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
        Self::new(
            ErrorKind::Internal,
            ErrorClass::Bug,
            "server operation failed",
            Some(anyhow::Error::msg(message.into())),
        )
    }

    /// A downstream dependency failure (mail, `WebSub`, …). Masks as a 500
    /// outwardly but classes as `External` so a dependency outage is
    /// distinguishable from a Jaunder bug during triage.
    pub fn external(error: impl Error + Send + Sync + 'static) -> Self {
        Self::new(
            ErrorKind::External,
            ErrorClass::External,
            "server operation failed",
            Some(anyhow::Error::new(error)),
        )
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
        Self::new(kind, class, public_message, Some(source))
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

    /// Captured active span stack from the construction site. Operator-only:
    /// `web` never projects this into its public wire error.
    #[must_use]
    pub fn span_trace(&self) -> &SpanTrace {
        &self.span_trace
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
    /// level derived from the error class, the captured active span stack, and
    /// the `jaunder.errors` metric with bounded kind, class, disposition, and
    /// origin attributes. `context` is emitted as a single serialized field;
    /// promoting each k/v to a span field is deferred to §4.6 (kq8w.22). Called
    /// by `web`'s `server_boundary`; the outward wire projection stays in `web`.
    ///
    /// **Which server fn failed is not a field here.** The event is emitted inside
    /// the fn's ADR-0011 `#[tracing::instrument]` span, and both configured sinks
    /// render span context unconditionally — the JSON formatter via
    /// `display_current_span`/`display_span_list`, the plain formatter by walking
    /// `event_scope()`. So the span name (`web.<vertical>.<ident>`) already
    /// identifies it, and more precisely than a bare ident could — an ident like
    /// `create` is ambiguous across verticals (#684, #714).
    /// `web::error::boundary_failure_event_carries_the_enclosing_instrument_span`
    /// pins the span-scope premise.
    pub fn emit_boundary_failure(&self) {
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
                    error.kind = ?self.kind,
                    error.class = ?self.class,
                    error.public = %self.public_message,
                    error.source = %source,
                    error.context = ?self.context,
                    error.span_trace = %self.span_trace,
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
        record_error(
            self.kind,
            self.class,
            ErrorDisposition::Boundary,
            TelemetryOrigin::Server,
        );
    }
}

// ---------------------------------------------------------------------------
// Typed `From` conversions (ADR-0017 §3, ADR-0058)
// ---------------------------------------------------------------------------
//
// The `(kind, class)` and wire `public_message` are fixed here, at each
// conversion's home, so a call site can never silently move the wire class by
// switching to bare `?`/`.into()`; the *typed* source is preserved on the
// `anyhow` chain for the operator rather than eagerly stringified
// (ADR-0017 §3, A19).

impl From<sqlx::Error> for InternalError {
    /// A storage-driver failure: masks as `"storage operation failed"` (kind
    /// `Storage`, class `Bug`) while preserving the `sqlx::Error` as a typed,
    /// downcastable source. Behavior-identical to `InternalError::storage(error)`.
    ///
    /// Lifting *every* `sqlx::Error` to class `Bug` is right for what this impl
    /// actually carries: pool timeouts, I/O, protocol and constraint failures,
    /// all genuinely pageable. `Error::RowNotFound` reaching here would mean a
    /// call site used `fetch_one` on a row that can be absent — a caller
    /// defect. Absence is named at its source instead, by
    /// `storage::error::MissingRow` (#343).
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
/// message = the source's `Display`, with the typed source preserved on the
/// operator side (A19).
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
    common::password::InvalidPassword,
    common::render::InvalidPostFormat,
    common::media::InvalidMediaSource,
);

#[cfg(test)]
mod tests {
    use super::{
        ErrorClass, ErrorDisposition, ErrorKind, InternalError, SwallowedSource, TelemetryOrigin,
        report_swallowed,
    };
    use std::error::Error;
    use std::fmt;

    #[derive(Debug)]
    struct SourceError;

    impl fmt::Display for SourceError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("source context")
        }
    }

    #[derive(Clone)]
    struct SharedWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("capture lock")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        // cov:ignore-start
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        } // cov:ignore-stop
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedWriter {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
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
        // The operator-facing rendering walks the cause chain via the preserved
        // anyhow source.
        assert_eq!(error.operator_message(), "outer failure: source context");
    }

    #[test]
    fn internal_error_captures_active_span_stack_and_fields() {
        use tracing_subscriber::prelude::*;

        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_test_writer())
            .with(tracing_error::ErrorLayer::default());
        let _guard = tracing::subscriber::set_default(subscriber);

        let parent = tracing::info_span!(
            "web.registration.register",
            registration.policy = tracing::field::Empty
        );
        let _parent_guard = parent.enter();
        parent.record(
            "registration.policy",
            tracing::field::display("operator_invites"),
        );
        let child = tracing::info_span!("storage.user.create_user", db.system = "postgres");
        let _child_guard = child.enter();

        let error = InternalError::server_message("boom");
        let trace = error.span_trace().to_string();

        assert!(
            trace.contains("web.registration.register"),
            "span trace missed parent: {trace}"
        );
        assert!(
            trace.contains("storage.user.create_user"),
            "span trace missed child: {trace}"
        );
        assert!(
            trace.contains("registration.policy") && trace.contains("operator_invites"),
            "span trace missed recorded determinant field: {trace}"
        );
        assert!(
            trace.contains("db.system") && trace.contains("postgres"),
            "span trace missed child field: {trace}"
        );
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

        assert_eq!(ErrorDisposition::Boundary.as_str(), "boundary");
        assert_eq!(ErrorDisposition::Swallowed.as_str(), "swallowed");
        assert_eq!(TelemetryOrigin::Server.as_str(), "server");
        assert_eq!(TelemetryOrigin::Client.as_str(), "client");
        assert_eq!(ErrorClass::Bug.as_metric_str(), "bug");
        assert_eq!(ErrorClass::External.as_metric_str(), "external");
    }
    #[test]
    fn client_source_kinds_map_to_fixed_bounded_strings() {
        use common::client_telemetry::ClientSourceKind;

        let cases = [
            (ClientSourceKind::StorageUnavailable, "storage_unavailable"),
            (ClientSourceKind::StorageOperation, "storage_operation"),
            (ClientSourceKind::InvalidSeed, "invalid_seed"),
            (ClientSourceKind::DialogUnavailable, "dialog_unavailable"),
            (ClientSourceKind::FormDataCreate, "form_data_create"),
            (ClientSourceKind::FormDataAppend, "form_data_append"),
        ];
        for (source_kind, expected) in cases {
            assert_eq!(super::client_source_kind_as_str(source_kind), expected);
        }
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
        .emit_boundary_failure();
        // Client → DEBUG arm, no source (the `None`/`unwrap_or_default` branch).
        InternalError::validation("bad input").emit_boundary_failure();
        // External → WARN arm.
        InternalError::external(OuterError {
            source: SourceError,
        })
        .emit_boundary_failure();
    }

    #[test]
    fn boundary_failure_event_emits_captured_span_trace() {
        use tracing_subscriber::prelude::*;

        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let fmt_layer = tracing_subscriber::fmt::layer()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()));
        let subscriber = tracing_subscriber::registry()
            .with(fmt_layer)
            .with(tracing_error::ErrorLayer::default());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "web.example.do_thing",
                decision.path = tracing::field::Empty
            );
            let _guard = span.enter();
            span.record("decision.path", tracing::field::display("example"));
            InternalError::server_message("operator-only").emit_boundary_failure();
        });

        let text =
            String::from_utf8(output.lock().expect("capture lock").clone()).expect("utf8 output");
        let event = text
            .lines()
            .find(|line| line.contains("server function failed"))
            .unwrap_or_else(|| panic!("boundary event missing: {text}"));
        assert!(
            event.contains(r#""error.kind":"Internal""#),
            "event: {event}"
        );
        assert!(event.contains("error.span_trace"), "event: {event}");
        assert!(event.contains("web.example.do_thing"), "event: {event}");
        assert!(event.contains("decision.path"), "event: {event}");
        assert!(event.contains("example"), "event: {event}");
    }

    #[tokio::test]
    async fn report_swallowed_emits_one_warn_and_one_metric() {
        use opentelemetry::global;
        use opentelemetry_sdk::metrics::{
            InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
            data::{AggregatedMetrics, MetricData},
        };
        use tracing_subscriber::prelude::*;

        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        global::set_meter_provider(provider.clone());

        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let fmt_layer = tracing_subscriber::fmt::layer()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()));
        let subscriber = tracing_subscriber::registry()
            .with(fmt_layer)
            .with(tracing_error::ErrorLayer::default());
        tracing::subscriber::with_default(subscriber, || {
            let span =
                tracing::info_span!("server.test.work", cleanup.mode = tracing::field::Empty);
            let _guard = span.enter();
            span.record("cleanup.mode", tracing::field::display("best_effort"));
            let source = OuterError {
                source: SourceError,
            };
            report_swallowed(
                ErrorKind::Storage,
                ErrorClass::Transient,
                "server.test.cleanup",
                SwallowedSource::Error(&source),
            );
        });
        provider.force_flush().expect("flush");

        let text =
            String::from_utf8(output.lock().expect("capture lock").clone()).expect("utf8 output");
        let warn_events: Vec<_> = text
            .lines()
            .filter(|line| line.contains(r#""level":"WARN""#))
            .collect();
        assert_eq!(warn_events.len(), 1, "exactly one WARN event: {text}");
        let event = warn_events[0];
        assert!(event.contains(r#""error.kind":"storage""#));
        assert!(event.contains(r#""error.class":"transient""#));
        assert!(event.contains(r#""error.disposition":"swallowed""#));
        assert!(event.contains(r#""telemetry.origin":"server""#));
        assert!(event.contains(r#""error.context":"server.test.cleanup""#));
        assert!(event.contains(r#""error.source":"outer failure: source context""#));
        assert!(event.contains("server.test.work"), "event: {event}");
        assert!(event.contains("cleanup.mode"), "event: {event}");
        assert!(event.contains("best_effort"), "event: {event}");

        let metrics = exporter.get_finished_metrics().expect("metrics");
        let points: Vec<_> = metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .filter(|metric| metric.name() == "jaunder.errors")
            .filter_map(|metric| match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => Some(sum),
                _ => None, // cov:ignore
            })
            .flat_map(opentelemetry_sdk::metrics::data::Sum::data_points)
            .collect();
        assert_eq!(points.len(), 1);
        let attrs: std::collections::BTreeSet<_> = points[0]
            .attributes()
            .map(|kv| (kv.key.as_str().to_owned(), kv.value.to_string()))
            .collect();
        assert_eq!(
            attrs,
            [
                ("error.kind".to_owned(), "storage".to_owned()),
                ("error.class".to_owned(), "transient".to_owned()),
                ("error.disposition".to_owned(), "swallowed".to_owned()),
                ("telemetry.origin".to_owned(), "server".to_owned()),
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn report_swallowed_redacted_source_needs_no_arbitrary_text() {
        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_writer(SharedWriter(output.clone()))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            report_swallowed(
                ErrorKind::Internal,
                ErrorClass::Bug,
                "server.test.redacted",
                SwallowedSource::Redacted,
            );
        });
        let text =
            String::from_utf8(output.lock().expect("capture lock").clone()).expect("utf8 output");
        assert!(text.contains(r#""error.source":"redacted""#));
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
        assert!(
            error
                .operator_message()
                .contains("mail sender is not configured")
        );
    }

    #[test]
    fn from_common_validation_sources_preserve_display_as_public_and_are_client() {
        // Each common value-object parse error lifts to Validation/Client with
        // the source's `Display` as the wire message and the typed source
        // preserved on the operator side.
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
        check!(common::tag::TagValidationError::TooMany {
            count: common::tag::MAX_TAGS_PER_POST + 1,
            max: common::tag::MAX_TAGS_PER_POST,
        });
        check!(common::password::InvalidPassword::PasswordTooShort);
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
