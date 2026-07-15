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

/// A session token resolved from request headers, plus — for HTTP Basic auth —
/// the username the authenticated session must belong to.
pub struct Credential {
    /// The raw session token to authenticate.
    pub token: RawToken,
    /// For Basic auth, the validated username supplied alongside the token, which
    /// must match the authenticated session's user. `None` for cookie/Bearer auth.
    pub expected_username: Option<common::username::Username>,
}

/// Resolves the session credential from request headers.
///
/// Precedence: the `session=` cookie, then `Authorization: Bearer <token>`,
/// then `Authorization: Basic <base64(user:token)>` (app passwords). Returns
/// `None` when no recognized credential is present.
#[must_use]
pub fn resolve_credential(headers: &http::HeaderMap) -> Option<Credential> {
    let from_cookie = headers
        .get(http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .map(str::trim)
                .find_map(|c| c.strip_prefix("session="))
                // Parse the value as a `RawToken` and contribute a credential only
                // on success: an empty or malformed `session=` value is skipped so
                // it no longer short-circuits past a valid header (#344 item 2).
                .and_then(|token| RawToken::from_str(token).ok())
                .map(|token| Credential {
                    token,
                    expected_username: None,
                })
        });
    if from_cookie.is_some() {
        return from_cookie;
    }

    let header = headers
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())?;
    if let Some(token) = header.strip_prefix("Bearer ") {
        RawToken::from_str(token).ok().map(|token| Credential {
            token,
            expected_username: None,
        })
    } else {
        let (username, password) = common::auth::parse_basic_auth(header)?;
        RawToken::from_str(&password).ok().map(|token| Credential {
            token,
            expected_username: Some(username),
        })
    }
}

/// Builds the `Set-Cookie` header value that stores the session token. `secure`
/// appends the `; Secure` attribute (production/HTTPS deployments).
#[must_use]
pub fn session_cookie_header(token: &RawToken, secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    // `token` is a `RawToken`, so its value is base64url by construction: the
    // charset cannot contain the `;`/newline separators a cookie header uses, so
    // interpolating it here cannot inject extra attributes or headers (#344 item 3).
    let token = token.as_ref();
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

    fn headers_with(name: http::HeaderName, value: &str) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert(name, http::HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn resolve_credential_prefers_cookie_over_authorization() {
        let mut headers = headers_with(http::header::COOKIE, "session=cookie-token");
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("Bearer bearer-token"),
        );
        let credential = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token.as_ref(), "cookie-token");
        assert_eq!(credential.expected_username, None);
    }

    #[test]
    fn resolve_credential_reads_bearer_token() {
        let headers = headers_with(http::header::AUTHORIZATION, "Bearer bearer-token");
        let credential = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token.as_ref(), "bearer-token");
        assert_eq!(credential.expected_username, None);
    }

    #[test]
    fn resolve_credential_reads_basic_app_password() {
        // base64("alice:tok123") == "YWxpY2U6dG9rMTIz"
        let headers = headers_with(http::header::AUTHORIZATION, "Basic YWxpY2U6dG9rMTIz");
        let credential = resolve_credential(&headers).expect("credential");
        assert_eq!(credential.token.as_ref(), "tok123");
        assert_eq!(credential.expected_username.as_deref(), Some("alice"));
    }

    #[test]
    fn resolve_credential_returns_none_without_recognized_header() {
        assert!(resolve_credential(&http::HeaderMap::new()).is_none());
        let headers = headers_with(http::header::AUTHORIZATION, "Negotiate xyz");
        assert!(resolve_credential(&headers).is_none());
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
        let credential = resolve_credential(&headers).expect("credential from header");
        assert_eq!(credential.token.as_ref(), "abcABC012-_");
    }

    #[test]
    fn resolve_credential_rejects_unparseable_bearer() {
        // A Bearer value that is not a valid RawToken yields no credential from that source.
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            "Bearer has space".parse().unwrap(),
        );
        assert!(resolve_credential(&headers).is_none());
    }

    #[test]
    fn session_cookie_header_secure_matches_current_string() {
        assert_eq!(
            session_cookie_header(&RawToken::try_from("token".to_string()).unwrap(), true),
            "session=token; HttpOnly; SameSite=Lax; Path=/; Secure"
        );
    }

    #[test]
    fn session_cookie_header_insecure_matches_current_string() {
        assert_eq!(
            session_cookie_header(&RawToken::try_from("token".to_string()).unwrap(), false),
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
