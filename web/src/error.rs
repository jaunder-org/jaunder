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

    pub fn storage(error: impl Error + 'static) -> Self {
        Self::Storage {
            message: error_with_sources(&error),
        }
    }

    pub fn server(error: impl Error + 'static) -> Self {
        Self::Server {
            message: error_with_sources(&error),
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

#[cfg(feature = "ssr")]
#[derive(Debug)]
pub struct InternalError {
    public: WebError,
    operator_message: String,
}

#[cfg(feature = "ssr")]
impl InternalError {
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(WebError::Unauthorized, message)
    }

    pub fn not_found(resource: impl Into<String>) -> Self {
        let public = WebError::not_found(resource);
        let operator_message = public.to_string();
        Self {
            public,
            operator_message,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        let public = WebError::validation(message);
        let operator_message = public.to_string();
        Self {
            public,
            operator_message,
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        let public = WebError::conflict(message);
        let operator_message = public.to_string();
        Self {
            public,
            operator_message,
        }
    }

    pub fn storage(error: impl Error + 'static) -> Self {
        let operator_message = error_with_sources(&error);
        Self {
            public: WebError::Storage {
                message: "storage operation failed".to_string(),
            },
            operator_message,
        }
    }

    pub fn server(error: impl Error + 'static) -> Self {
        let operator_message = error_with_sources(&error);
        Self {
            public: WebError::Server {
                message: "server operation failed".to_string(),
            },
            operator_message,
        }
    }

    pub fn server_message(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            public: WebError::Server {
                message: "server operation failed".to_string(),
            },
            operator_message: message,
        }
    }

    pub fn masked(public: WebError, operator_message: impl Into<String>) -> Self {
        Self::new(public, operator_message)
    }

    pub fn public(&self) -> &WebError {
        &self.public
    }

    pub fn operator_message(&self) -> &str {
        &self.operator_message
    }

    pub fn into_public(self) -> WebError {
        self.public
    }

    fn new(public: WebError, operator_message: impl Into<String>) -> Self {
        Self {
            public,
            operator_message: operator_message.into(),
        }
    }
}

#[cfg(feature = "ssr")]
pub async fn server_boundary<T>(
    server_fn: &'static str,
    future: impl std::future::Future<Output = InternalResult<T>>,
) -> WebResult<T> {
    match future.await {
        Ok(value) => Ok(value),
        Err(error) => {
            tracing::error!(
                server_fn,
                public_error = ?error.public(),
                operator_message = %error.operator_message(),
                "server function failed"
            );
            Err(error.into_public())
        }
    }
}

pub fn error_with_sources(error: &(dyn Error + 'static)) -> String {
    let mut message = error.to_string();
    let mut source = error.source();

    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }

    message
}

#[cfg(test)]
mod tests {
    use super::{error_with_sources, WebError};
    #[cfg(feature = "ssr")]
    use super::{server_boundary, InternalError};
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
    fn error_with_sources_includes_source_chain() {
        let error = OuterError {
            source: SourceError,
        };

        assert_eq!(error_with_sources(&error), "outer failure: source context");
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

    #[test]
    fn storage_and_server_errors_preserve_source_chain() {
        let storage = WebError::storage(OuterError {
            source: SourceError,
        });
        let server = WebError::server(OuterError {
            source: SourceError,
        });

        assert_eq!(
            storage,
            WebError::Storage {
                message: "outer failure: source context".to_string()
            }
        );
        assert_eq!(
            server,
            WebError::Server {
                message: "outer failure: source context".to_string()
            }
        );
    }

    #[test]
    fn json_encoding_uses_stable_snake_case_variant_names() {
        let encoded = <JsonEncoding as Encodes<WebError>>::encode(&WebError::Unauthorized).unwrap();
        assert_eq!(encoded.as_ref(), br#""unauthorized""#);

        let decoded = <JsonEncoding as Decodes<WebError>>::decode(encoded).unwrap();
        assert_eq!(decoded, WebError::Unauthorized);
    }

    #[cfg(feature = "ssr")]
    #[test]
    fn server_boundary_logs_and_returns_public_error() {
        use std::future::Future;
        use std::task::{Context, Poll, Waker};

        let mut future = Box::pin(server_boundary("test_fn", async {
            Err(InternalError::storage(OuterError {
                source: SourceError,
            }))
        }));
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let result: Result<(), WebError> = match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("server_boundary test future should complete immediately"),
        };

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
}
