#[cfg(feature = "server")]
use leptos::server_fn::{Bytes, Decodes, Encodes};
use leptos::server_fn::{
    codec::JsonEncoding,
    error::{FromServerFnError, ServerFnErrorErr},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    #[doc(hidden)]
    #[error("server function error: {message}")]
    ServerFunctionInput { message: String },
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
    /// Rewrites the private input-decode carrier to the stable public wire error.
    #[cfg(feature = "server")]
    pub fn normalize_server_fn_error_body(body: Bytes) -> Option<Bytes> {
        let error = <JsonEncoding as Decodes<Self>>::decode(body).ok()?;
        let Self::ServerFunctionInput { message } = error else {
            return None;
        };
        <JsonEncoding as Encodes<Self>>::encode(&Self::server_function(message)).ok()
    }
}

impl FromServerFnError for WebError {
    type Encoder = JsonEncoding;

    fn from_server_fn_error(value: ServerFnErrorErr) -> Self {
        // The response adapter consumes this server-only classification before
        // the error crosses the public wire boundary.
        #[cfg(feature = "server")]
        super::server::emit_arg_decode_failure(&value);
        let message = value.to_string();
        #[cfg(feature = "server")]
        if matches!(
            value,
            ServerFnErrorErr::Args(_)
                | ServerFnErrorErr::MissingArg(_)
                | ServerFnErrorErr::Deserialization(_)
        ) {
            return Self::ServerFunctionInput { message };
        }
        Self::server_function(message)
    }
}

#[cfg(test)]
mod tests {
    use super::WebError;
    use leptos::prelude::FromServerFnError;
    use leptos::server_fn::{Decodes, Encodes, codec::JsonEncoding, error::ServerFnErrorErr};

    #[test]
    fn server_function_errors_preserve_the_framework_message() {
        let error = WebError::from_server_fn_error(ServerFnErrorErr::Args("bad arg".to_string()));

        assert!(error.to_string().contains("bad arg"));
    }

    #[cfg(feature = "server")]
    #[test]
    fn only_input_decode_errors_have_a_normalizable_body() {
        for error in [
            ServerFnErrorErr::Args("args".to_string()),
            ServerFnErrorErr::MissingArg("missing".to_string()),
            ServerFnErrorErr::Deserialization("input".to_string()),
        ] {
            let body = WebError::from_server_fn_error(error).ser();
            assert!(WebError::normalize_server_fn_error_body(body).is_some());
        }

        for error in [
            ServerFnErrorErr::Registration("registration".to_string()),
            ServerFnErrorErr::UnsupportedRequestMethod("method".to_string()),
            ServerFnErrorErr::Request("request".to_string()),
            ServerFnErrorErr::ServerError("server".to_string()),
            ServerFnErrorErr::MiddlewareError("middleware".to_string()),
            ServerFnErrorErr::Serialization("output".to_string()),
            ServerFnErrorErr::Response("response".to_string()),
        ] {
            let body = WebError::from_server_fn_error(error).ser();
            assert!(WebError::normalize_server_fn_error_body(body).is_none());
        }
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
    fn json_encoding_uses_stable_snake_case_variant_names() {
        let encoded = <JsonEncoding as Encodes<WebError>>::encode(&WebError::Unauthorized).unwrap();
        assert_eq!(encoded.as_ref(), br#""unauthorized""#);

        let decoded = <JsonEncoding as Decodes<WebError>>::decode(encoded).unwrap();
        assert_eq!(decoded, WebError::Unauthorized);
    }
}
