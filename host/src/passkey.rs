//! The sole Jaunder-facing `WebAuthn` relying-party adapter.
//!
//! Fork and expert-core types stay here so storage and orchestration persist
//! stable values without acquiring a cryptographic-policy surface of their own.

use std::error::Error as StdError;
use std::fmt;

use common::tagged_url::BaseUrl;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Host;
use webauthn_rs::prelude::{
    AuthenticationResult, Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Url, Uuid, Webauthn, WebauthnBuilder,
};

/// A server-persisted discoverable credential. Its representation is deliberately
/// opaque outside this module so only this adapter selects verification policy.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Credential(Passkey);

impl Credential {
    /// Stable opaque identity used for storage ownership checks.
    #[must_use]
    pub fn credential_id(&self) -> &[u8] {
        self.0.cred_id().as_slice()
    }

    /// The persisted signature-counter high-water mark.
    #[must_use]
    pub fn counter(&self) -> u32 {
        self.0.counter()
    }

    /// Rehydrates a known-valid serialized credential for storage tests.
    ///
    /// This test-support-only seam does not expose the fork type or admit a
    /// production construction path.
    ///
    /// # Errors
    ///
    /// Returns the serialization error when `value` is not a valid credential.
    #[cfg(feature = "test-support")]
    pub fn deserialize_for_test(value: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(value)
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Credential([REDACTED])")
    }
}

/// Server-only registration ceremony state.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegistrationState(PasskeyRegistration);

/// Server-only authentication ceremony state.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthenticationState(PasskeyAuthentication);

impl fmt::Debug for RegistrationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RegistrationState([REDACTED])")
    }
}

impl fmt::Debug for AuthenticationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthenticationState([REDACTED])")
    }
}

/// An opaque `WebAuthn` user handle. Registration currently requires the
/// fork-supported 16-byte representation; callers must not substitute usernames.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserHandle([u8; 16]);

impl UserHandle {
    #[must_use]
    pub fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for UserHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("UserHandle([REDACTED])")
    }
}

/// JSON sent to `navigator.credentials.create`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegistrationRequest(serde_json::Value);

impl RegistrationRequest {
    #[must_use]
    pub fn json(&self) -> &serde_json::Value {
        &self.0
    }
}

/// JSON sent to `navigator.credentials.get`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthenticationRequest(serde_json::Value);

impl AuthenticationRequest {
    #[must_use]
    pub fn json(&self) -> &serde_json::Value {
        &self.0
    }
}

/// A browser registration result that remains untrusted until finished here.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegistrationResponse(serde_json::Value);

/// A browser assertion that remains untrusted until finished here.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthenticationResponse(serde_json::Value);

impl RegistrationResponse {
    /// Reconstitutes the untrusted browser JSON at the server boundary.
    #[must_use]
    pub fn from_json(value: serde_json::Value) -> Self {
        Self(value)
    }
}

impl AuthenticationResponse {
    /// Reconstitutes the untrusted browser JSON at the server boundary.
    #[must_use]
    pub fn from_json(value: serde_json::Value) -> Self {
        Self(value)
    }
}

/// The opaque identifiers supplied by a discoverable assertion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredCredential {
    pub user_handle: UserHandle,
    pub credential_id: Vec<u8>,
}

/// How durable storage should fold an already-verified signature counter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CounterOutcome {
    Advance(u32),
    RetainHighWater { returned: u32 },
}

/// The durable changes from a fully verified assertion.
#[derive(Clone, Debug)]
pub struct VerifiedAuthentication {
    pub credential: Credential,
    pub counter: CounterOutcome,
}

/// Classify a verified assertion without ever lowering a nonzero stored counter.
#[must_use]
pub fn classify_counter(stored: u32, returned: u32) -> CounterOutcome {
    if returned > stored {
        CounterOutcome::Advance(returned)
    } else {
        CounterOutcome::RetainHighWater { returned }
    }
}

/// Returns the exact trusted `WebAuthn` origin and RP hostname for a configured
/// base URL. HTTP is limited to browser-trustworthy `localhost` domains. IP
/// addresses are not valid RP IDs at the safe wrapper boundary.
///
/// # Errors
///
/// Returns an error when the URL has no domain hostname or is neither HTTPS nor
/// an accepted localhost HTTP origin.
pub fn webauthn_identity(base_url: &BaseUrl) -> Result<(String, String), PasskeyError> {
    let mut origin = Url::parse(base_url.as_ref()).map_err(|_| PasskeyError::InvalidOrigin)?;
    let host = origin.host().ok_or(PasskeyError::MissingHostname)?;
    let (rp_id, http_loopback) = match host {
        Host::Domain(domain) => {
            let rp_id = domain.to_owned();
            (
                rp_id.clone(),
                rp_id == "localhost" || rp_id.ends_with(".localhost"),
            )
        }
        Host::Ipv4(_) | Host::Ipv6(_) => return Err(PasskeyError::InvalidOrigin),
    };
    if origin.scheme() != "https" && !(origin.scheme() == "http" && http_loopback) {
        return Err(PasskeyError::InvalidOrigin);
    }
    origin.set_path("");
    origin.set_query(None);
    origin.set_fragment(None);
    Ok((origin.to_string().trim_end_matches('/').to_owned(), rp_id))
}
/// Failures constructing the exact relying-party identity or translating trusted
/// `WebAuthn` protocol values.
#[derive(Debug, Error)]
pub enum PasskeyError {
    #[error("site.base_url is not a WebAuthn origin")]
    InvalidOrigin,
    #[error("site.base_url has no hostname for the WebAuthn RP ID")]
    MissingHostname,
    #[error("WebAuthn protocol failure")]
    Protocol(#[source] Box<dyn StdError + Send + Sync>),
    #[error("WebAuthn JSON does not match the expected ceremony value")]
    Json(#[source] serde_json::Error),
    #[error("verified WebAuthn credential did not match persisted credential")]
    CredentialMismatch,
    #[error("WebAuthn assertion supplied an invalid opaque user handle")]
    InvalidUserHandle,
}

/// The exact-origin relying-party adapter.
pub struct RelyingParty {
    webauthn: Webauthn,
}

impl RelyingParty {
    /// Build one RP from `site.base_url` only. Request headers and permissive
    /// origin knobs are intentionally unavailable at this seam.
    ///
    /// # Errors
    ///
    /// Returns an error when `site.base_url` has no valid hostname/origin or the
    /// safe `WebAuthn` wrapper rejects the resulting relying-party configuration.
    pub fn from_base_url(base_url: &BaseUrl) -> Result<Self, PasskeyError> {
        let (exact_origin, rp_id) = webauthn_identity(base_url)?;
        let origin = Url::parse(&exact_origin).map_err(|_| PasskeyError::InvalidOrigin)?;
        let webauthn = WebauthnBuilder::new(&rp_id, &origin)
            .map_err(|error| PasskeyError::Protocol(Box::new(error)))?
            .build()
            .map_err(|error| PasskeyError::Protocol(Box::new(error)))?;
        Ok(Self { webauthn })
    }

    /// Start resident-required, UV-required, no-attestation registration.
    ///
    /// # Errors
    ///
    /// Returns an error when the opaque handle cannot form a UUID or the safe
    /// `WebAuthn` wrapper cannot issue the resident registration ceremony.
    pub fn start_registration(
        &self,
        user_handle: &UserHandle,
        user_name: &str,
        user_display_name: &str,
        existing: &[Credential],
    ) -> Result<(RegistrationRequest, RegistrationState), PasskeyError> {
        let user = Uuid::from_bytes(*user_handle.as_bytes());
        let excluded = existing
            .iter()
            .map(|credential| credential.0.cred_id().clone())
            .collect();
        let (request, state) = self
            .webauthn
            .start_resident_key_passkey_registration(
                user,
                user_name,
                user_display_name,
                Some(excluded),
            )
            .map_err(|error| PasskeyError::Protocol(Box::new(error)))?;
        Ok((
            RegistrationRequest(serde_json::to_value(request).map_err(PasskeyError::Json)?),
            RegistrationState(state),
        ))
    }

    /// Complete a resident registration ceremony.
    ///
    /// # Errors
    ///
    /// Returns an error when the browser response is malformed or `WebAuthn`
    /// verification rejects the registration ceremony.
    pub fn finish_registration(
        &self,
        response: RegistrationResponse,
        state: &RegistrationState,
    ) -> Result<Credential, PasskeyError> {
        // cov:ignore-start: Browser credential deserialization is exercised by malformed-response unit tests and successful Chromium virtual-authenticator E2E; source coverage cannot combine those target-specific paths.
        let response: RegisterPublicKeyCredential =
            serde_json::from_value(response.0).map_err(PasskeyError::Json)?;
        // Successful attestation verification is proved by Chromium virtual-authenticator E2E; host fixtures cannot forge a valid attestation.
        self.webauthn
            .finish_passkey_registration(&response, &state.0)
            .map(Credential)
            .map_err(|error| PasskeyError::Protocol(Box::new(error)))
        // cov:ignore-stop
    }

    /// Start a user-invoked discoverable assertion with empty allowCredentials.
    ///
    /// # Errors
    ///
    /// Returns an error when the safe `WebAuthn` wrapper cannot issue the
    /// discoverable authentication ceremony.
    pub fn start_authentication(
        &self,
    ) -> Result<(AuthenticationRequest, AuthenticationState), PasskeyError> {
        let (request, state) = self
            .webauthn
            .start_discoverable_passkey_authentication()
            .map_err(|error| PasskeyError::Protocol(Box::new(error)))?;
        Ok((
            AuthenticationRequest(serde_json::to_value(request).map_err(PasskeyError::Json)?),
            AuthenticationState(state),
        ))
    }

    /// Identify the opaque user-handle and credential ID in an assertion.
    ///
    /// # Errors
    ///
    /// Returns an error when the browser response is malformed, lacks a
    /// 16-byte user handle, or the safe `WebAuthn` wrapper rejects identification.
    pub fn identify(
        &self,
        response: &AuthenticationResponse,
    ) -> Result<DiscoveredCredential, PasskeyError> {
        let response: PublicKeyCredential =
            serde_json::from_value(response.0.clone()).map_err(PasskeyError::Json)?;
        let (user_handle, credential_id) = self
            .webauthn
            .identify_discoverable_passkey_authentication(&response)
            .map_err(|error| PasskeyError::Protocol(Box::new(error)))?;
        let user_handle = user_handle
            .try_into()
            .map_err(|_| PasskeyError::InvalidUserHandle)?;
        Ok(DiscoveredCredential {
            user_handle: UserHandle::new(user_handle),
            credential_id: credential_id.to_vec(),
        })
    }

    /// Complete verification after storage has checked the discovered user-handle
    /// and credential pairing. Counter anomalies are returned as verified results.
    ///
    /// # Errors
    ///
    /// Returns an error when the browser response is malformed, verification
    /// fails, or its credential does not match the supplied persisted credential.
    pub fn finish_authentication(
        &self,
        response: AuthenticationResponse,
        state: AuthenticationState,
        credential: &Credential,
    ) -> Result<VerifiedAuthentication, PasskeyError> {
        // cov:ignore-start: Browser assertion deserialization and verification are exercised across malformed-response unit tests and successful Chromium virtual-authenticator E2E; source coverage cannot combine those target-specific paths.
        let response: PublicKeyCredential =
            serde_json::from_value(response.0).map_err(PasskeyError::Json)?;
        // Chromium virtual-authenticator E2E proves successful assertion cryptography; host fixtures cannot forge one, and the singleton credential invariant makes mismatch construction unreachable.
        let result: AuthenticationResult = self
            .webauthn
            .finish_discoverable_passkey_authentication(
                &response,
                state.0,
                std::slice::from_ref(&credential.0),
            )
            .map_err(|error| PasskeyError::Protocol(Box::new(error)))?;
        let counter = classify_counter(credential.counter(), result.counter());
        let mut credential = credential.clone();
        // The safe wrapper folds backup flags and only advances counters, so the
        // returned opaque value is the one storage must persist after verification.
        credential
            .0
            .update_credential(&result)
            .ok_or(PasskeyError::CredentialMismatch)?;
        Ok(VerifiedAuthentication {
            credential,
            counter,
        })
        // cov:ignore-stop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_classification_preserves_the_high_water_mark() {
        assert_eq!(classify_counter(4, 5), CounterOutcome::Advance(5));
        assert_eq!(
            classify_counter(4, 4),
            CounterOutcome::RetainHighWater { returned: 4 }
        );
        assert_eq!(
            classify_counter(4, 0),
            CounterOutcome::RetainHighWater { returned: 0 }
        );
    }

    #[test]
    fn registration_uses_exact_base_url_rp_and_resident_policy() {
        let base_url: BaseUrl = "https://passkeys.example.test:8443/".parse().unwrap();
        let relying_party = RelyingParty::from_base_url(&base_url).unwrap();
        let (request, state) = relying_party
            .start_registration(&UserHandle::new([7; 16]), "account", "Account", &[])
            .unwrap();
        assert_eq!(format!("{state:?}"), "RegistrationState([REDACTED])");
        assert_eq!(
            format!("{:?}", UserHandle::new([7; 16])),
            "UserHandle([REDACTED])"
        );

        assert_eq!(
            request.json()["publicKey"]["rp"]["id"],
            "passkeys.example.test"
        );
        assert_eq!(request.json()["publicKey"]["attestation"], "none");
        assert_eq!(
            request.json()["publicKey"]["authenticatorSelection"]["residentKey"],
            "required"
        );
        assert_eq!(
            request.json()["publicKey"]["authenticatorSelection"]["userVerification"],
            "required"
        );
        let encoded = serde_json::to_value(&state).unwrap();
        let decoded: RegistrationState = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
    }

    #[test]
    fn authentication_omits_mediation_and_serializes_server_state() {
        let base_url: BaseUrl = "https://passkeys.example.test:8443/".parse().unwrap();
        let relying_party = RelyingParty::from_base_url(&base_url).unwrap();
        let (request, state) = relying_party.start_authentication().unwrap();
        assert_eq!(format!("{state:?}"), "AuthenticationState([REDACTED])");

        assert_eq!(request.json()["publicKey"]["rpId"], "passkeys.example.test");
        assert_eq!(
            request.json()["publicKey"]["allowCredentials"],
            serde_json::json!([])
        );
        assert!(request.json().get("mediation").is_none());
        let encoded = serde_json::to_value(&state).unwrap();
        let decoded: AuthenticationState = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
    }

    #[test]
    fn browser_response_constructors_keep_untrusted_json_at_the_adapter_boundary() {
        let value = serde_json::json!({"id": "browser-supplied"});

        assert_eq!(RegistrationResponse::from_json(value.clone()).0, value);
        assert_eq!(AuthenticationResponse::from_json(value.clone()).0, value);
    }

    #[test]
    fn registration_finish_rejects_malformed_browser_json() {
        let base_url: BaseUrl = "https://passkeys.example.test:8443/".parse().unwrap();
        let relying_party = RelyingParty::from_base_url(&base_url).unwrap();
        let (_, state) = relying_party
            .start_registration(&UserHandle::new([7; 16]), "account", "Account", &[])
            .unwrap();

        let result = relying_party.finish_registration(
            RegistrationResponse::from_json(serde_json::json!({})),
            &state,
        );

        assert!(matches!(result, Err(PasskeyError::Json(_))));
    }

    #[test]
    fn webauthn_identity_matches_safe_wrapper_origin_boundaries() {
        for value in [
            "https://example.test/",
            "http://localhost/",
            "http://dev.localhost:8080/",
        ] {
            let base_url = value.parse().unwrap();
            assert!(webauthn_identity(&base_url).is_ok(), "{value}");
            assert!(RelyingParty::from_base_url(&base_url).is_ok(), "{value}");
        }
        for value in ["http://example.test/", "http://127.0.0.1/", "http://[::1]/"] {
            assert!(
                webauthn_identity(&value.parse().unwrap()).is_err(),
                "{value}"
            );
        }
    }
}
