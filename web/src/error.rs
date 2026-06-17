use leptos::server_fn::{
    codec::JsonEncoding,
    error::{FromServerFnError, ServerFnErrorErr},
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use thiserror::Error;

pub type WebResult<T> = Result<T, WebError>;

#[cfg(feature = "ssr")]
pub type InternalResult<T> = Result<T, InternalError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "snake_case")]
pub enum WebError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("{message}")]
    NotFound { message: String },
    #[error("{message}")]
    Validation { message: String },
    #[error("{message}")]
    Conflict { message: String },
    #[error("storage error: {message}")]
    Storage { message: String },
    #[error("server error: {message}")]
    Server { message: String },
    #[error("server function error: {message}")]
    ServerFunction { message: String },
}

impl WebError {
    pub fn not_found(resource: impl Into<String>) -> Self {
        Self::NotFound {
            message: format!("{} not found", resource.into()),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
        }
    }

    pub fn server_message(message: impl Into<String>) -> Self {
        Self::Server {
            message: message.into(),
        }
    }

    pub fn server_function(message: impl Into<String>) -> Self {
        Self::ServerFunction {
            message: message.into(),
        }
    }
}

impl FromServerFnError for WebError {
    type Encoder = JsonEncoding;

    fn from_server_fn_error(value: ServerFnErrorErr) -> Self {
        Self::server_function(value.to_string())
    }
}

/// The category of an internal failure, derived at construction. Drives
/// outward mapping and is emitted as a discrete `error.kind` field at the
/// boundary for queryable triage.
#[cfg(feature = "ssr")]
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

/// Operational severity, derived at construction so triage (and the
/// boundary's log level) is mechanical rather than guessed from the message.
#[cfg(feature = "ssr")]
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

#[cfg(feature = "ssr")]
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
}

/// Server-side error carrier: an outward `public` view plus structured,
/// queryable operator data (`kind`, `class`, `context`) and the preserved
/// `source` cause chain (carried via `anyhow`, never stringified eagerly).
#[cfg(feature = "ssr")]
#[derive(Debug)]
pub struct InternalError {
    public: WebError,
    kind: ErrorKind,
    class: ErrorClass,
    context: Vec<(&'static str, String)>,
    source: Option<anyhow::Error>,
}

#[cfg(feature = "ssr")]
impl InternalError {
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::masked(WebError::Unauthorized, message)
    }

    pub fn not_found(resource: impl Into<String>) -> Self {
        WebError::not_found(resource).into()
    }

    pub fn validation(message: impl Into<String>) -> Self {
        WebError::validation(message).into()
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        WebError::conflict(message).into()
    }

    pub fn storage(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            public: WebError::Storage {
                message: "storage operation failed".to_string(),
            },
            kind: ErrorKind::Storage,
            class: ErrorClass::Bug,
            context: Vec::new(),
            source: Some(anyhow::Error::new(error)),
        }
    }

    pub fn server(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            public: WebError::Server {
                message: "server operation failed".to_string(),
            },
            kind: ErrorKind::Internal,
            class: ErrorClass::Bug,
            context: Vec::new(),
            source: Some(anyhow::Error::new(error)),
        }
    }

    pub fn server_message(message: impl Into<String>) -> Self {
        Self {
            public: WebError::Server {
                message: "server operation failed".to_string(),
            },
            kind: ErrorKind::Internal,
            class: ErrorClass::Bug,
            context: Vec::new(),
            source: Some(anyhow::Error::msg(message.into())),
        }
    }

    /// A downstream dependency failure (mail, `WebSub`, …). Masks as a 500
    /// outwardly but classes as `External` so a dependency outage is
    /// distinguishable from a Jaunder bug during triage.
    pub fn external(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            public: WebError::Server {
                message: "server operation failed".to_string(),
            },
            kind: ErrorKind::External,
            class: ErrorClass::External,
            context: Vec::new(),
            source: Some(anyhow::Error::new(error)),
        }
    }

    /// Masks an arbitrary public error with an operator-only message; the
    /// `kind`/`class` are inferred from the public variant.
    pub fn masked(public: WebError, operator_message: impl Into<String>) -> Self {
        let (kind, class) = kind_class_for(&public);
        Self {
            public,
            kind,
            class,
            context: Vec::new(),
            source: Some(anyhow::Error::msg(operator_message.into())),
        }
    }

    /// Attaches a structured key/value to the operator-side context, emitted
    /// at the boundary (see [`server_boundary`]). Never reaches the client.
    #[must_use]
    pub fn with_context(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.context.push((key, value.into()));
        self
    }

    #[must_use]
    pub fn public(&self) -> &WebError {
        &self.public
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

    /// Renders the operator-facing detail (the preserved source cause chain,
    /// falling back to the public message). For logs and re-masking only;
    /// never sent to the client.
    #[must_use]
    pub fn operator_message(&self) -> String {
        render_operator_message(&self.public, self.source.as_ref())
    }

    #[must_use]
    pub fn into_public(self) -> WebError {
        self.public
    }
}

/// Builds a client/expected `InternalError` from its public form: no `source`,
/// with `(kind, class)` inferred from the variant via [`kind_class_for`], so
/// the operator message falls back to the public text. This is the shared body
/// behind the `not_found`/`validation`/`conflict` delegates. Masking failures
/// (`storage`/`server`/`server_message`/`unauthorized`/`external`) keep their
/// explicit constructors — do not route a `WebError::Storage`/`Server` through
/// here, which would carry its message unmasked rather than as a generic 500.
#[cfg(feature = "ssr")]
impl From<WebError> for InternalError {
    fn from(public: WebError) -> Self {
        let (kind, class) = kind_class_for(&public);
        Self {
            public,
            kind,
            class,
            context: Vec::new(),
            source: None,
        }
    }
}

/// Infers `(kind, class)` from a public error variant, for `masked` errors
/// where the operator detail is supplied separately from a typed source.
#[cfg(feature = "ssr")]
fn kind_class_for(public: &WebError) -> (ErrorKind, ErrorClass) {
    match public {
        WebError::Unauthorized => (ErrorKind::Auth, ErrorClass::Client),
        WebError::NotFound { .. } => (ErrorKind::NotFound, ErrorClass::Client),
        WebError::Validation { .. } => (ErrorKind::Validation, ErrorClass::Client),
        WebError::Conflict { .. } => (ErrorKind::Conflict, ErrorClass::Client),
        WebError::Storage { .. } => (ErrorKind::Storage, ErrorClass::Bug),
        WebError::Server { .. } | WebError::ServerFunction { .. } => {
            (ErrorKind::Internal, ErrorClass::Bug)
        }
    }
}

/// Renders the operator-facing detail: the preserved cause chain (alternate
/// `anyhow` formatting) when a source is present, else the public message.
#[cfg(feature = "ssr")]
fn render_operator_message(public: &WebError, source: Option<&anyhow::Error>) -> String {
    match source {
        Some(source) => format!("{source:#}"),
        None => public.to_string(),
    }
}

/// Awaits the given future, converting any `InternalError` to its public `WebError` form.
///
/// # Errors
///
/// Returns `Err(ServerFnError)` if the wrapped future returns an `InternalError`.
#[cfg(feature = "ssr")]
pub async fn server_boundary<T>(
    server_fn: &'static str,
    future: impl std::future::Future<Output = InternalResult<T>>,
) -> WebResult<T> {
    match future.await {
        Ok(value) => Ok(value),
        Err(error) => {
            log_boundary_failure(server_fn, &error);
            Err(error.into_public())
        }
    }
}

/// Emits the structured boundary log for a failed server function: discrete,
/// queryable fields (not one concatenated string), at the level derived from
/// the error class. `context` is emitted as a single serialized field;
/// promoting each k/v to a span field is deferred to §4.6 (kq8w.22).
#[cfg(feature = "ssr")]
fn log_boundary_failure(server_fn: &'static str, error: &InternalError) {
    // Render the preserved cause chain once; empty when there is no source
    // (e.g. pure client errors).
    let source = error
        .source
        .as_ref()
        .map(|s| format!("{s:#}"))
        .unwrap_or_default();
    macro_rules! emit {
        ($macro:ident) => {
            tracing::$macro!(
                server_fn,
                error.kind = ?error.kind,
                error.class = ?error.class,
                error.public = ?error.public,
                error.source = %source,
                error.context = ?error.context,
                "server function failed",
            )
        };
    }
    // `ErrorClass::log_level` is the single source of truth; the match only
    // exists because `tracing`'s macros require a statically-known level.
    match error.class.log_level() {
        tracing::Level::DEBUG => emit!(debug),
        tracing::Level::WARN => emit!(warn),
        _ => emit!(error),
    }
}

#[cfg(test)]
mod tests {
    use super::WebError;
    #[cfg(feature = "ssr")]
    use super::{server_boundary, InternalError, WebResult};
    use leptos::prelude::FromServerFnError;
    use leptos::server_fn::{codec::JsonEncoding, error::ServerFnErrorErr, Decodes, Encodes};
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
    fn server_function_errors_map_to_web_error() {
        let error = WebError::from_server_fn_error(ServerFnErrorErr::Args("bad arg".to_string()));

        assert!(matches!(error, WebError::ServerFunction { .. }));
        assert!(error.to_string().contains("bad arg"));
    }

    #[test]
    fn constructors_create_expected_variants() {
        assert_eq!(
            WebError::not_found("Post"),
            WebError::NotFound {
                message: "Post not found".to_string()
            }
        );
        assert_eq!(
            WebError::validation("bad input"),
            WebError::Validation {
                message: "bad input".to_string()
            }
        );
        assert_eq!(
            WebError::conflict("already exists"),
            WebError::Conflict {
                message: "already exists".to_string()
            }
        );
        assert_eq!(
            WebError::server_message("boom"),
            WebError::Server {
                message: "boom".to_string()
            }
        );
        assert_eq!(
            WebError::server_function("bad args"),
            WebError::ServerFunction {
                message: "bad args".to_string()
            }
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn masked_internal_errors_never_leak_source_chain_to_public() {
        // §2.4 regression guard: storage/server failures reach the client only
        // through `InternalError`, which must mask. The raw source chain may
        // appear in the operator message (logged) but never in the public
        // `WebError` sent to the browser. The leaky `WebError::storage`/`server`
        // constructors that embedded the chain were removed for this reason.
        for internal in [
            InternalError::storage(OuterError {
                source: SourceError,
            }),
            InternalError::server(OuterError {
                source: SourceError,
            }),
        ] {
            assert!(
                internal.operator_message().contains("source context"),
                "operator message should retain the source chain for logs"
            );
            let public = internal.into_public();
            assert!(
                !public.to_string().contains("source context"),
                "public error leaked source detail: {public}"
            );
        }
    }

    #[test]
    fn json_encoding_uses_stable_snake_case_variant_names() {
        let encoded = <JsonEncoding as Encodes<WebError>>::encode(&WebError::Unauthorized).unwrap();
        assert_eq!(encoded.as_ref(), br#""unauthorized""#);

        let decoded = <JsonEncoding as Decodes<WebError>>::decode(encoded).unwrap();
        assert_eq!(decoded, WebError::Unauthorized);
    }

    #[cfg(feature = "ssr")]
    #[tokio::test]
    async fn server_boundary_logs_and_returns_public_error() {
        let result: Result<(), WebError> = server_boundary("test_fn", async {
            Err(InternalError::storage(OuterError {
                source: SourceError,
            }))
        })
        .await;

        assert_eq!(
            result,
            Err(WebError::Storage {
                message: "storage operation failed".to_string()
            })
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn internal_error_preserves_operator_message() {
        let error = InternalError::server(OuterError {
            source: SourceError,
        });

        assert_eq!(error.operator_message(), "outer failure: source context");
        assert_eq!(
            error.public(),
            &WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn internal_error_server_message_keeps_operator_detail_and_generic_public_message() {
        let error = InternalError::server_message("operator-only context");
        assert_eq!(error.operator_message(), "operator-only context");
        assert_eq!(
            error.public(),
            &WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "ssr")]
    #[tokio::test]
    async fn server_boundary_evaluates_tracing_fields_when_subscriber_is_active() {
        use tracing_subscriber::fmt;

        let subscriber = fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let result = server_boundary("test_fn", async {
            Err::<(), _>(InternalError::server(OuterError {
                source: SourceError,
            }))
        })
        .await;

        assert_eq!(
            result,
            Err(WebError::Server {
                message: "server operation failed".to_string()
            })
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn internal_error_constructors_set_correct_public_variants() {
        let unauth = InternalError::unauthorized("not allowed");
        assert_eq!(unauth.public(), &WebError::Unauthorized);
        assert_eq!(unauth.operator_message(), "not allowed");

        let not_found = InternalError::not_found("Post");
        assert_eq!(not_found.public(), &WebError::not_found("Post"));

        let validation = InternalError::validation("bad input");
        assert_eq!(validation.public(), &WebError::validation("bad input"));

        let conflict = InternalError::conflict("already exists");
        assert_eq!(conflict.public(), &WebError::conflict("already exists"));
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn into_public_consumes_error_and_returns_public_variant() {
        let error = InternalError::unauthorized("reason");
        assert_eq!(error.into_public(), WebError::Unauthorized);
    }

    #[cfg(feature = "ssr")]
    #[tokio::test]
    async fn server_boundary_passes_through_ok_value() {
        let result: WebResult<u32> = server_boundary("test_fn", async { Ok(42) }).await;
        assert_eq!(result, Ok(42));
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn masked_internal_error_keeps_public_and_operator_messages_separate() {
        let error = InternalError::masked(
            WebError::not_found("Post"),
            "draft access denied for missing session token",
        );

        assert_eq!(
            error.operator_message(),
            "draft access denied for missing session token"
        );
        assert_eq!(error.public(), &WebError::not_found("Post"));
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn error_class_maps_to_log_level() {
        use super::ErrorClass;
        use tracing::Level;
        assert_eq!(ErrorClass::Client.log_level(), Level::DEBUG);
        assert_eq!(ErrorClass::Transient.log_level(), Level::WARN);
        assert_eq!(ErrorClass::External.log_level(), Level::WARN);
        assert_eq!(ErrorClass::Bug.log_level(), Level::ERROR);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn constructors_set_kind_and_class() {
        use super::{ErrorClass, ErrorKind};

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

    #[cfg(feature = "ssr")]
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

    #[cfg(feature = "ssr")]
    #[test]
    fn storage_error_captures_source_chain_not_stringified() {
        let error = InternalError::storage(OuterError {
            source: SourceError,
        });
        // The operator-facing rendering still walks the cause chain (now via
        // the preserved anyhow source instead of an eager concatenation).
        assert_eq!(error.operator_message(), "outer failure: source context");
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn external_constructor_sets_external_kind_and_class() {
        use super::{ErrorClass, ErrorKind};
        let error = InternalError::external(OuterError {
            source: SourceError,
        });
        assert_eq!(error.kind(), ErrorKind::External);
        assert_eq!(error.class(), ErrorClass::External);
        assert_eq!(error.operator_message(), "outer failure: source context");
        // Outward it still masks as a generic 500.
        assert_eq!(
            error.public(),
            &WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn masked_infers_kind_and_class_from_public_variant() {
        use super::{ErrorClass, ErrorKind};
        let cases = [
            (WebError::Unauthorized, ErrorKind::Auth, ErrorClass::Client),
            (
                WebError::not_found("x"),
                ErrorKind::NotFound,
                ErrorClass::Client,
            ),
            (
                WebError::validation("x"),
                ErrorKind::Validation,
                ErrorClass::Client,
            ),
            (
                WebError::conflict("x"),
                ErrorKind::Conflict,
                ErrorClass::Client,
            ),
            (
                WebError::Storage {
                    message: "x".to_string(),
                },
                ErrorKind::Storage,
                ErrorClass::Bug,
            ),
            (
                WebError::Server {
                    message: "x".to_string(),
                },
                ErrorKind::Internal,
                ErrorClass::Bug,
            ),
            (
                WebError::ServerFunction {
                    message: "x".to_string(),
                },
                ErrorKind::Internal,
                ErrorClass::Bug,
            ),
        ];
        for (public, kind, class) in cases {
            let error = InternalError::masked(public, "operator detail");
            assert_eq!(error.kind(), kind);
            assert_eq!(error.class(), class);
        }
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn client_error_operator_message_falls_back_to_public() {
        // A client error carries no source, so the operator rendering falls
        // back to the public message.
        let error = InternalError::not_found("Post");
        assert_eq!(error.operator_message(), "Post not found");
    }

    #[cfg(feature = "ssr")]
    #[tokio::test]
    async fn server_boundary_logs_client_at_debug_and_returns_public() {
        let result: WebResult<()> = server_boundary("test_fn", async {
            Err(InternalError::validation("bad input"))
        })
        .await;
        assert_eq!(result, Err(WebError::validation("bad input")));
    }

    #[cfg(feature = "ssr")]
    #[tokio::test]
    async fn server_boundary_logs_external_at_warn_and_returns_public() {
        let result: WebResult<()> = server_boundary("test_fn", async {
            Err(InternalError::external(OuterError {
                source: SourceError,
            }))
        })
        .await;
        assert_eq!(
            result,
            Err(WebError::Server {
                message: "server operation failed".to_string()
            })
        );
    }
}
