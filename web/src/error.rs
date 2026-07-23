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

impl FromServerFnError for WebError {
    type Encoder = JsonEncoding;

    fn from_server_fn_error(value: ServerFnErrorErr) -> Self {
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
    server_fn: &'static str,
    future: impl std::future::Future<Output = InternalResult<T>>,
) -> WebResult<T> {
    match future.await {
        Ok(value) => Ok(value),
        Err(error) => {
            // The carrier owns its own observability (structured log + metric);
            // `web` only performs the wire projection.
            error.emit_boundary_failure(server_fn);
            Err(project(error.kind(), error.public_message()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WebError;
    #[cfg(feature = "server")]
    use super::{project, server_boundary, ErrorClass, ErrorKind, InternalError, WebResult};
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
        let result: WebResult<u32> = server_boundary("test_fn", async { Ok(42) }).await;
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
            server_boundary("test_fn", async { Err(InternalError::not_found("Post")) }).await;
        assert_eq!(result, Err(WebError::not_found("Post")));
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn server_boundary_logs_client_at_debug_and_returns_public() {
        let result: WebResult<()> = server_boundary("test_fn", async {
            Err(InternalError::validation("bad input"))
        })
        .await;
        assert_eq!(result, Err(WebError::validation("bad input")));
    }

    #[cfg(feature = "server")]
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
