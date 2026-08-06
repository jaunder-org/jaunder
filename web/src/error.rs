use leptos::server_fn::{
    codec::JsonEncoding,
    error::{FromServerFnError, ServerFnErrorErr},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// The server-side error carrier lives in `host` (ADR-0058); `web` keeps only the
// wire type and the `kind → WebError` projection. Re-exported so every vertical's
// `InternalError::storage(…)`/`?` call site names it unchanged through `web::error`.
#[cfg(feature = "server")]
pub use host::error::{ErrorClass, ErrorKind, InternalError, InternalResult};

pub type WebResult<T> = Result<T, WebError>;

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

/// Emits the boundary telemetry for a failed `#[server]` **argument decode**.
///
/// Arg deserialization happens in leptos's `from_req`, *before* the generated
/// `__server_<ident>` fn runs — so neither the `web.<vertical>.<ident>` span nor
/// [`server_boundary`] is reached, and a malformed request would otherwise leave no
/// trace at all (#822). This restores the standard boundary event and error metric for
/// that path, reusing the existing vocabulary: `Validation`/`Client`, plus a
/// `stage = decode` context entry distinguishing it from an in-body validation failure.
///
/// Only arg-decode variants emit. `ServerError`/`MiddlewareError`/`Response` are
/// server-side too, but they arise *downstream* of decode and are already covered by the
/// in-body boundary; the predicate here is "this request's arguments were malformed".
///
/// The failing fn is identified by the enclosing request span's `uri`
/// (`server/src/observability.rs`), not by a span name — ADR-0011's "identity comes free
/// from span context" argument does not hold this early.
#[cfg(feature = "server")]
fn emit_arg_decode_failure(value: &ServerFnErrorErr) {
    if !matches!(
        value,
        ServerFnErrorErr::Args(_)
            | ServerFnErrorErr::MissingArg(_)
            | ServerFnErrorErr::Deserialization(_)
    ) {
        return;
    }
    // `validation_source`, not `validation`: the latter carries no source and would emit
    // an empty `error.source`, which is the whole diagnostic payload. `ServerFnErrorErr`
    // is `Clone + thiserror::Error`, so it satisfies the source bound directly.
    InternalError::validation_source("invalid request arguments", value.clone())
        .with_context("stage", "decode")
        .emit_boundary_failure();
}

impl FromServerFnError for WebError {
    type Encoder = JsonEncoding;

    fn from_server_fn_error(value: ServerFnErrorErr) -> Self {
        // Telemetry only — the returned wire error is unchanged.
        #[cfg(feature = "server")]
        emit_arg_decode_failure(&value);
        Self::server_function(value.to_string())
    }
}

/// Projects an `InternalError`'s `(kind, public_message)` to its outward
/// `WebError` wire form — the total, message-carrying counterpart to the
/// carrier's construction. This is the single point where the carrier becomes a
/// wire type (see [`server_boundary`]); the operator-side `source`/`context`
/// have no projection and so cannot leak. Masking kinds
/// (`Storage`/`Internal`/`External`) carry only their generic public message,
/// never the source detail.
#[cfg(feature = "server")]
pub(crate) fn project(kind: ErrorKind, public_message: &str) -> WebError {
    match kind {
        ErrorKind::Auth => WebError::Unauthorized,
        ErrorKind::NotFound => WebError::NotFound {
            message: public_message.to_string(),
        },
        ErrorKind::Validation => WebError::Validation {
            message: public_message.to_string(),
        },
        ErrorKind::Conflict => WebError::Conflict {
            message: public_message.to_string(),
        },
        ErrorKind::Storage => WebError::Storage {
            message: public_message.to_string(),
        },
        ErrorKind::Internal | ErrorKind::External => WebError::Server {
            message: public_message.to_string(),
        },
    }
}

/// Awaits the given future, converting any `InternalError` to its public
/// `WebError` form. This is a thin error-projection boundary: it owns no leptos
/// reactive-owner lifetime concerns. (Owner-pinning against context loss across an
/// `.await` was removed in #594 — see the ADR-0016 retirement addendum; the sole
/// server-fn invocation path, `leptos_axum`'s `/api` handler, holds the owner strong
/// for the whole future itself.)
///
/// # Errors
///
/// Returns `Err(WebError)` if the wrapped future returns an `InternalError`.
#[cfg(feature = "server")]
pub async fn server_boundary<T>(
    future: impl std::future::Future<Output = InternalResult<T>>,
) -> WebResult<T> {
    match future.await {
        Ok(value) => Ok(value),
        Err(error) => {
            // The carrier owns its own observability (structured log + metric);
            // `web` only performs the wire projection.
            error.emit_boundary_failure();
            Err(project(error.kind(), error.public_message()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WebError;
    #[cfg(feature = "server")]
    use super::{ErrorClass, ErrorKind, InternalError, WebResult, project, server_boundary};
    use leptos::prelude::FromServerFnError;
    use leptos::server_fn::{Decodes, Encodes, codec::JsonEncoding, error::ServerFnErrorErr};

    /// The error-chain fixtures, gated once for the whole group rather than
    /// per item.
    ///
    /// Every user is a `server`-feature test below, so in a no-`server` build the
    /// definitions were live while all their uses were compiled out — which reads
    /// as `dead_code` (#826). Their exclusive imports (`std::error::Error`,
    /// `std::fmt`) live in here too, for the same reason; leaving those ungated
    /// just moves the same warning onto them.
    ///
    /// Note no build in the gate compiles this crate's tests *without* `server`
    /// (`wasm-clippy` omits `--all-targets`), so a mis-gate here is invisible to
    /// CI — hence stating the rule in one place.
    #[cfg(feature = "server")]
    mod fixtures {
        use std::error::Error;
        use std::fmt;

        #[derive(Debug)]
        pub(super) struct SourceError;

        impl fmt::Display for SourceError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("source context")
            }
        }

        impl Error for SourceError {}

        #[derive(Debug)]
        pub(super) struct OuterError {
            pub(super) source: SourceError,
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
    }

    #[cfg(feature = "server")]
    use fixtures::{OuterError, SourceError};

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

    #[cfg(feature = "server")]
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
            let public = project(internal.kind(), internal.public_message());
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

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_logs_and_returns_public_error() {
        let result: Result<(), WebError> = server_boundary(async {
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

    #[cfg(feature = "server")]
    #[test]
    fn internal_error_preserves_operator_message() {
        let error = InternalError::server(OuterError {
            source: SourceError,
        });

        assert_eq!(error.operator_message(), "outer failure: source context");
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn internal_error_server_message_keeps_operator_detail_and_generic_public_message() {
        let error = InternalError::server_message("operator-only context");
        assert_eq!(error.operator_message(), "operator-only context");
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_evaluates_tracing_fields_when_subscriber_is_active() {
        use tracing_subscriber::fmt;

        let subscriber = fmt()
            .with_test_writer()
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let result = server_boundary(async {
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

    #[cfg(feature = "server")]
    #[test]
    fn internal_error_constructors_set_correct_public_variants() {
        let unauth = InternalError::unauthorized("not allowed");
        assert_eq!(
            project(unauth.kind(), unauth.public_message()),
            WebError::Unauthorized
        );
        assert_eq!(unauth.operator_message(), "not allowed");

        let not_found = InternalError::not_found("Post");
        assert_eq!(
            project(not_found.kind(), not_found.public_message()),
            WebError::not_found("Post")
        );

        let validation = InternalError::validation("bad input");
        assert_eq!(
            project(validation.kind(), validation.public_message()),
            WebError::validation("bad input")
        );

        let conflict = InternalError::conflict("already exists");
        assert_eq!(
            project(conflict.kind(), conflict.public_message()),
            WebError::conflict("already exists")
        );
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_passes_through_ok_value() {
        let result: WebResult<u32> = server_boundary(async { Ok(42) }).await;
        assert_eq!(result, Ok(42));
    }

    #[cfg(feature = "server")]
    #[test]
    fn masked_internal_error_keeps_public_and_operator_messages_separate() {
        let error = InternalError::masked(
            ErrorKind::NotFound,
            ErrorClass::Client,
            "Post not found",
            anyhow::Error::msg("draft access denied for missing session token"),
        );

        assert_eq!(
            error.operator_message(),
            "draft access denied for missing session token"
        );
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::not_found("Post")
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn external_constructor_sets_external_kind_and_class() {
        let error = InternalError::external(OuterError {
            source: SourceError,
        });
        assert_eq!(error.kind(), ErrorKind::External);
        assert_eq!(error.class(), ErrorClass::External);
        assert_eq!(error.operator_message(), "outer failure: source context");
        // Outward it still masks as a generic 500.
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn project_is_the_total_kind_to_web_error_map() {
        assert_eq!(project(ErrorKind::Auth, "ignored"), WebError::Unauthorized);
        assert_eq!(
            project(ErrorKind::NotFound, "x not found"),
            WebError::NotFound {
                message: "x not found".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::Validation, "bad"),
            WebError::Validation {
                message: "bad".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::Conflict, "dupe"),
            WebError::Conflict {
                message: "dupe".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::Storage, "storage operation failed"),
            WebError::Storage {
                message: "storage operation failed".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::Internal, "server operation failed"),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
        assert_eq!(
            project(ErrorKind::External, "server operation failed"),
            WebError::Server {
                message: "server operation failed".to_string()
            }
        );
    }

    #[cfg(feature = "server")]
    #[test]
    fn masking_constructors_set_generic_public_message_and_preserve_source() {
        let error = InternalError::storage(OuterError {
            source: SourceError,
        });
        assert_eq!(error.kind(), ErrorKind::Storage);
        assert_eq!(
            project(error.kind(), error.public_message()),
            WebError::Storage {
                message: "storage operation failed".to_string()
            }
        );
        // The source chain is preserved for operator logs, never on the wire.
        assert!(error.operator_message().contains("source context"));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_err_path_projects_the_same_wire_error() {
        let result: WebResult<()> =
            server_boundary(async { Err(InternalError::not_found("Post")) }).await;
        assert_eq!(result, Err(WebError::not_found("Post")));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_logs_client_at_debug_and_returns_public() {
        let result: WebResult<()> =
            server_boundary(async { Err(InternalError::validation("bad input")) }).await;
        assert_eq!(result, Err(WebError::validation("bad input")));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_logs_external_at_warn_and_returns_public() {
        let result: WebResult<()> = server_boundary(async {
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

    /// The ADR-0011 span must be in scope when the boundary logs a failure.
    ///
    /// This was the premise of #714, which deleted the `boundary!` label: it was
    /// redundant *because* the enclosing `#[tracing::instrument]` span already names
    /// the failing fn on the very same event — and more precisely, since a bare
    /// ident like `create` is ambiguous across verticals. If this ever fails, that
    /// premise is gone and the deletion cost observability.
    ///
    /// Deliberately uses a hand-written `#[tracing::instrument]` rather than
    /// `#[macros::server]`: the property under test is `tracing`'s, not the macro's,
    /// and this fixture must remain valid wherever it lives.
    /// Records `(field, value)` pairs for every event, so a test can assert on the
    /// boundary event's structured fields rather than its rendered text.
    ///
    /// Distinct from `ScopeRecorder` below, which captures span *scopes*: the decode
    /// path has no enclosing `web.<vertical>.<ident>` span to capture (#822), so what
    /// matters there is the fields.
    /// One `Vec<(field, value)>` per recorded event, shared with the test that
    /// installed the layer.
    #[cfg(feature = "server")]
    type RecordedFields = std::sync::Arc<std::sync::Mutex<Vec<Vec<(String, String)>>>>;

    #[cfg(feature = "server")]
    struct FieldRecorder(RecordedFields);

    #[cfg(feature = "server")]
    impl<S: tracing::Subscriber> tracing_subscriber::layer::Layer<S> for FieldRecorder {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Visitor(Vec<(String, String)>);
            impl tracing::field::Visit for Visitor {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    self.0
                        .push((field.name().to_string(), format!("{value:?}")));
                }
            }
            let mut visitor = Visitor(Vec::new());
            event.record(&mut visitor);
            self.0.lock().expect("field recorder mutex").push(visitor.0);
        }
    }

    /// Arg decode happens in leptos's `from_req`, before the instrumented body — so
    /// without an explicit emit a malformed request leaves no trace at all (#822).
    #[cfg(feature = "server")]
    #[test]
    fn arg_decode_failure_emits_a_boundary_event() {
        use tracing_subscriber::prelude::*;

        let events: RecordedFields = std::sync::Arc::default();
        let subscriber =
            tracing_subscriber::registry().with(FieldRecorder(std::sync::Arc::clone(&events)));
        let _guard = tracing::subscriber::set_default(subscriber);

        let error = WebError::from_server_fn_error(ServerFnErrorErr::Args(
            "invalid value for `password`: password must be at least 8 characters".into(),
        ));

        // The response is unchanged: the telemetry is purely additive.
        assert!(matches!(error, WebError::ServerFunction { .. }));

        let recorded = events.lock().expect("field recorder mutex").clone();
        let fields: Vec<(String, String)> = recorded.into_iter().flatten().collect();
        let get = |name: &str| {
            fields
                .iter()
                .find(|(f, _)| f == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };

        assert!(
            get("error.kind").contains("Validation"),
            "expected a Validation-kind boundary event; got {fields:?}"
        );
        assert!(get("error.class").contains("Client"), "fields: {fields:?}");
        // `stage = decode` is what separates this from an in-body validation failure.
        assert!(
            get("error.context").contains("decode"),
            "fields: {fields:?}"
        );
        // The deserializer's message reaches `error.source` — the diagnostic payload.
        assert!(
            get("error.source").contains("at least 8 characters"),
            "fields: {fields:?}"
        );
        // Pin the event's identity so a refactor cannot quietly emit a different one.
        assert!(
            get("message").contains("server function failed"),
            "fields: {fields:?}"
        );
        assert!(
            get("error.public").contains("invalid request arguments"),
            "fields: {fields:?}"
        );
    }

    /// The predicate is "this request's arguments were malformed" — not "this variant
    /// happens on the server". A transport failure must stay silent (#822).
    #[cfg(feature = "server")]
    #[test]
    fn non_decode_server_fn_errors_emit_nothing() {
        use tracing_subscriber::prelude::*;

        let events: RecordedFields = std::sync::Arc::default();
        let subscriber =
            tracing_subscriber::registry().with(FieldRecorder(std::sync::Arc::clone(&events)));
        let _guard = tracing::subscriber::set_default(subscriber);

        let _ =
            WebError::from_server_fn_error(ServerFnErrorErr::Request("connection reset".into()));

        assert!(
            events.lock().expect("field recorder mutex").is_empty(),
            "a non-decode variant must not emit a boundary event"
        );
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn boundary_failure_event_carries_the_enclosing_instrument_span() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::layer::{Context, Layer};
        use tracing_subscriber::prelude::*;
        use tracing_subscriber::registry::LookupSpan;

        /// Records the span-scope names in effect for each event.
        struct ScopeRecorder(Arc<Mutex<Vec<Vec<String>>>>);

        impl<S> Layer<S> for ScopeRecorder
        where
            S: tracing::Subscriber + for<'a> LookupSpan<'a>,
        {
            fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
                let names = ctx
                    .event_scope(event)
                    .map(|scope| {
                        scope
                            .from_root()
                            .map(|s| s.metadata().name().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                self.0.lock().expect("scope recorder mutex").push(names);
            }
        }

        // Stands in for a `#[server]` fn: same attribute, same boundary call.
        #[tracing::instrument(name = "web.example.do_thing")]
        async fn do_thing() -> WebResult<()> {
            server_boundary(async {
                Err(InternalError::server(OuterError {
                    source: SourceError,
                }))
            })
            .await
        }

        let events: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
        let subscriber = tracing_subscriber::registry().with(ScopeRecorder(Arc::clone(&events)));
        let _guard = tracing::subscriber::set_default(subscriber);

        assert!(
            do_thing().await.is_err(),
            "the fixture must take the failure path"
        );

        let recorded = events.lock().expect("scope recorder mutex").clone();
        assert!(
            recorded
                .iter()
                .any(|scope| scope.iter().any(|n| n == "web.example.do_thing")),
            "the boundary failure event must be emitted inside the instrument span; \
             recorded scopes: {recorded:?}"
        );
    }
}
