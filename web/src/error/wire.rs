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
        // Telemetry only — the returned wire error is unchanged.
        #[cfg(feature = "server")]
        super::server::emit_arg_decode_failure(&value);
        Self::server_function(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::WebError;
    use leptos::prelude::FromServerFnError;
    use leptos::server_fn::{Decodes, Encodes, codec::JsonEncoding, error::ServerFnErrorErr};

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
    fn json_encoding_uses_stable_snake_case_variant_names() {
        let encoded = <JsonEncoding as Encodes<WebError>>::encode(&WebError::Unauthorized).unwrap();
        assert_eq!(encoded.as_ref(), br#""unauthorized""#);

        let decoded = <JsonEncoding as Decodes<WebError>>::decode(encoded).unwrap();
        assert_eq!(decoded, WebError::Unauthorized);
    }
}
