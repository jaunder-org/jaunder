//! The sole Jaunder-facing `WebAuthn` relying-party adapter.
//!
//! Fork and expert-core types stay here so storage and orchestration persist
//! stable values without acquiring a cryptographic-policy surface of their own.

use std::error::Error as StdError;

use common::tagged_url::BaseUrl;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use webauthn_rs::prelude::{
    AuthenticationResult, Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Url, Uuid, Webauthn, WebauthnBuilder,
};

/// A server-persisted discoverable credential. Its representation is deliberately
/// opaque outside this module so only this adapter selects verification policy.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
}

/// Server-only registration ceremony state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegistrationState(PasskeyRegistration);

/// Server-only authentication ceremony state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthenticationState(PasskeyAuthentication);

/// An opaque `WebAuthn` user handle. Registration currently requires the
/// fork-supported 16-byte representation; callers must not substitute usernames.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
        let mut origin = Url::parse(base_url.as_ref()).map_err(|_| PasskeyError::InvalidOrigin)?;
        let rp_id = origin
            .host_str()
            .ok_or(PasskeyError::MissingHostname)?
            .to_owned();
        origin.set_path("");
        origin.set_query(None);
        origin.set_fragment(None);
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
        let response: RegisterPublicKeyCredential =
            serde_json::from_value(response.0).map_err(PasskeyError::Json)?;
        self.webauthn
            .finish_passkey_registration(&response, &state.0)
            .map(Credential)
            .map_err(|error| PasskeyError::Protocol(Box::new(error)))
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
        let response: PublicKeyCredential =
            serde_json::from_value(response.0).map_err(PasskeyError::Json)?;
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
}
