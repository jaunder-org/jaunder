//! Host-side HTTP credential parsing and session-cookie construction.
//!
//! The axum request extractor (`AuthUser: FromRequestParts`) and the leptos
//! cookie-setting adapters stay in `web`; this module holds the target-agnostic
//! cores they delegate to — resolving a session credential out of raw `http`
//! headers and building the `Set-Cookie` header string. `host` names the raw
//! `http` header types for parsing but no `web`/`storage`/leptos abstraction
//! (ADR-0058 floor invariant).

use std::str::FromStr;

use common::token::RawToken;

/// Deployment cookie settings derived from the public scheme. Provided into
/// leptos context by the `server` composition root and read by `web`'s cookie
/// adapters; carried here as plain configuration data.
#[derive(Clone, Copy)]
pub struct CookieSettings {
    /// Whether to emit the `; Secure` cookie attribute (HTTPS deployments).
    pub secure: bool,
}

/// How a request presented a resolved session credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialTransport {
    /// The `session=` cookie.
    Cookie,
    /// An `Authorization: Bearer` header.
    Bearer,
    /// An `Authorization: Basic` app password.
    Basic,
}

/// Why request headers did not resolve to a credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialResolutionError {
    /// No syntactically valid cookie or explicit credential was present.
    Missing,
    /// An `Authorization` header was present but unsupported or malformed.
    InvalidAuthorization,
}

/// A session credential resolved from request headers.
#[derive(Debug)]
pub struct Credential {
    /// The raw session token to authenticate.
    pub token: RawToken,
    /// For Basic auth, the validated username supplied alongside the token, which
    /// must match the authenticated session's user. `None` for cookie/Bearer auth.
    pub expected_username: Option<common::username::Username>,
    /// The request transport that supplied this credential.
    pub transport: CredentialTransport,
}
/// Resolves only the ordinary browser `session=` cookie from request headers.
///
/// Authorization headers are deliberately outside this parser's surface. A
/// malformed or absent cookie returns `None`; callers decide whether another
/// credential source is appropriate for their endpoint.
#[must_use]
pub fn resolve_session_cookie(headers: &http::HeaderMap) -> Option<RawToken> {
    let token = headers
        .get(http::header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|cookie| cookie.strip_prefix("session="))?;
    RawToken::from_str(token).ok()
}

/// A resolved credential and request context relevant after authentication.
#[derive(Debug)]
pub struct CredentialResolution {
    /// The authoritative credential selected from the request.
    pub credential: Credential,
    /// Whether the request also carried a `session=` cookie, valid or not.
    pub session_cookie_present: bool,
}

/// Resolves the authoritative session credential from request headers.
///
/// Any `Authorization` header is explicit request intent and is resolved before
/// cookies. Unsupported or malformed authorization therefore rejects rather than
/// falling back to ambient browser identity. Without that header, a valid
/// `session=` cookie is accepted.
///
/// # Errors
///
/// Returns [`CredentialResolutionError::InvalidAuthorization`] for any present
/// but unsupported or malformed `Authorization` value, and
/// [`CredentialResolutionError::Missing`] when no valid cookie is available.
pub fn resolve_credential(
    headers: &http::HeaderMap,
) -> Result<CredentialResolution, CredentialResolutionError> {
    let session_cookie = headers
        .get(http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(';')
                .map(str::trim)
                .find_map(|cookie| cookie.strip_prefix("session="))
        });
    let session_cookie_present = session_cookie.is_some();

    if let Some(value) = headers.get(http::header::AUTHORIZATION) {
        let header = value
            .to_str()
            .map_err(|_| CredentialResolutionError::InvalidAuthorization)?;
        let (token, expected_username, transport) =
            if let Some(token) = header.strip_prefix("Bearer ") {
                (
                    RawToken::from_str(token)
                        .map_err(|_| CredentialResolutionError::InvalidAuthorization)?,
                    None,
                    CredentialTransport::Bearer,
                )
            } else if header.starts_with("Basic ") {
                let credential = common::auth::parse_basic_auth(header)
                    .map_err(|_| CredentialResolutionError::InvalidAuthorization)?;
                (
                    credential.token,
                    Some(credential.username),
                    CredentialTransport::Basic,
                )
            } else {
                return Err(CredentialResolutionError::InvalidAuthorization);
            };

        return Ok(CredentialResolution {
            credential: Credential {
                token,
                expected_username,
                transport,
            },
            session_cookie_present,
        });
    }

    let token = RawToken::from_str(session_cookie.ok_or(CredentialResolutionError::Missing)?)
        .map_err(|_| CredentialResolutionError::Missing)?;
    Ok(CredentialResolution {
        credential: Credential {
            token,
            expected_username: None,
            transport: CredentialTransport::Cookie,
        },
        session_cookie_present,
    })
}

/// Builds the `Set-Cookie` header value that stores the session token. `secure`
/// appends the `; Secure` attribute (production/HTTPS deployments).
#[must_use]
pub fn session_cookie_header(token: &RawToken, secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    // `token` is a `RawToken`, so its value is base64url by construction: the
    // charset cannot contain the `;`/newline separators a cookie header uses, so
    // interpolating it here (via its `Display`) cannot inject extra attributes or
    // headers (#344 item 3).
    format!("session={token}; HttpOnly; SameSite=Lax; Path=/{secure_attr}")
}

/// Builds the `Set-Cookie` header value that clears the session cookie
/// (`Max-Age=0`). `secure` mirrors [`session_cookie_header`].
#[must_use]
pub fn clear_session_cookie_header(secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("session=; HttpOnly; SameSite=Lax; Path=/{secure_attr}; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::parse_raw_token;

    fn headers_with(name: http::HeaderName, value: &str) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert(name, http::HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn resolve_credential_prefers_authorization_and_reports_cookie_presence() {
        let mut headers = headers_with(http::header::COOKIE, "session=cookie-token");
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer bearer-token"),
        );
        let CredentialResolution {
            credential,
            session_cookie_present,
        } = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token, "bearer-token");
        assert_eq!(credential.transport, CredentialTransport::Bearer);
        assert!(session_cookie_present);
    }
    #[test]
    fn resolve_session_cookie_reads_only_the_cookie() {
        let mut headers = headers_with(http::header::COOKIE, "other=x; session=cookie-token");
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer bearer-token"),
        );

        assert_eq!(
            resolve_session_cookie(&headers).expect("session cookie"),
            "cookie-token"
        );
        assert!(
            resolve_session_cookie(&headers_with(
                http::header::AUTHORIZATION,
                "Bearer bearer-token"
            ))
            .is_none()
        );
    }

    #[test]
    fn resolve_credential_reads_bearer_token() {
        let headers = headers_with(http::header::AUTHORIZATION, "Bearer bearer-token");
        let CredentialResolution {
            credential,
            session_cookie_present,
        } = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token, "bearer-token");
        assert_eq!(credential.expected_username, None);
        assert_eq!(credential.transport, CredentialTransport::Bearer);
        assert!(!session_cookie_present);
    }

    #[test]
    fn resolve_credential_reads_basic_app_password_and_canonical_username() {
        // base64("Alice:tok123") == "QWxpY2U6dG9rMTIz"
        let headers = headers_with(http::header::AUTHORIZATION, "Basic QWxpY2U6dG9rMTIz");
        let CredentialResolution {
            credential,
            session_cookie_present,
        } = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token, "tok123");
        assert_eq!(credential.expected_username.as_deref(), Some("alice"));
        assert_eq!(credential.transport, CredentialTransport::Basic);
        assert!(!session_cookie_present);
    }

    #[test]
    fn resolve_credential_uses_cookie_without_authorization() {
        let headers = headers_with(http::header::COOKIE, "session=cookie-token");
        let CredentialResolution {
            credential,
            session_cookie_present,
        } = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token, "cookie-token");
        assert_eq!(credential.expected_username, None);
        assert_eq!(credential.transport, CredentialTransport::Cookie);
        assert!(session_cookie_present);
    }

    #[test]
    fn resolve_credential_rejects_bad_authorization_instead_of_using_cookie() {
        for value in ["Negotiate xyz", "Bearer has space", "Basic !!!notbase64!!!"] {
            let mut headers = headers_with(http::header::COOKIE, "session=cookie-token");
            headers.insert(http::header::AUTHORIZATION, value.parse().unwrap());
            assert!(matches!(
                resolve_credential(&headers),
                Err(CredentialResolutionError::InvalidAuthorization)
            ));
        }
    }

    #[test]
    fn resolve_credential_reports_missing_without_a_valid_credential() {
        assert!(matches!(
            resolve_credential(&http::HeaderMap::new()),
            Err(CredentialResolutionError::Missing)
        ));
        let headers = headers_with(http::header::COOKIE, "session=");
        assert!(matches!(
            resolve_credential(&headers),
            Err(CredentialResolutionError::Missing)
        ));
    }

    #[test]
    fn resolve_credential_empty_session_cookie_falls_through_to_header() {
        // #344 item 2: an empty `session=` cookie must NOT short-circuit; a valid
        // Authorization header on the same request must still authenticate.
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::COOKIE, "session=".parse().unwrap());
        headers.insert(
            http::header::AUTHORIZATION,
            "Bearer abcABC012-_".parse().unwrap(),
        );
        let CredentialResolution {
            credential,
            session_cookie_present,
        } = resolve_credential(&headers).expect("credential from header");
        assert_eq!(credential.token, "abcABC012-_");
        assert_eq!(credential.transport, CredentialTransport::Bearer);
        assert!(session_cookie_present);
    }

    #[test]
    fn session_cookie_header_secure_matches_current_string() {
        assert_eq!(
            session_cookie_header(&parse_raw_token("token"), true),
            "session=token; HttpOnly; SameSite=Lax; Path=/; Secure"
        );
    }

    #[test]
    fn session_cookie_header_insecure_matches_current_string() {
        assert_eq!(
            session_cookie_header(&parse_raw_token("token"), false),
            "session=token; HttpOnly; SameSite=Lax; Path=/"
        );
    }

    #[test]
    fn clear_session_cookie_header_secure_matches_current_string() {
        assert_eq!(
            clear_session_cookie_header(true),
            "session=; HttpOnly; SameSite=Lax; Path=/; Secure; Max-Age=0"
        );
    }

    #[test]
    fn clear_session_cookie_header_insecure_matches_current_string() {
        assert_eq!(
            clear_session_cookie_header(false),
            "session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0"
        );
    }
}
