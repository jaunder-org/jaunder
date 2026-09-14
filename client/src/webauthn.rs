//! Browser `WebAuthn` ceremony bridge.
//!
//! The server already speaks the `WebAuthn` JSON representation: binary fields are
//! base64url strings. The `WebAuthn` JSON conversion methods turn those strings
//! into the `ArrayBuffer` values required by `navigator.credentials` and back
//! again, without exposing browser types to the `web` crate.

use js_sys::{JSON, Promise, Reflect};
use serde_json::Value;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    CredentialCreationOptions, CredentialRequestOptions, CredentialsContainer, PublicKeyCredential,
    PublicKeyCredentialCreationOptions, PublicKeyCredentialRequestOptions,
};

/// The result of an explicit browser `WebAuthn` ceremony.
#[derive(Debug, PartialEq)]
pub enum CeremonyOutcome<T> {
    /// The browser returned a `WebAuthn` credential response.
    Success(T),
    /// The user or authenticator declined the ceremony.
    Cancelled,
    /// The browser does not expose the `WebAuthn` credentials API.
    Unsupported,
    /// An option conversion, browser invocation, or response conversion failed.
    Failed,
}

#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function jaunderWebauthnSupported(navigator) {
    return typeof PublicKeyCredential !== "undefined"
        && navigator != null
        && navigator.credentials != null
        && typeof navigator.credentials.create === "function"
        && typeof navigator.credentials.get === "function"
        && typeof PublicKeyCredential.parseCreationOptionsFromJSON === "function"
        && typeof PublicKeyCredential.parseRequestOptionsFromJSON === "function"
        && typeof PublicKeyCredential.prototype.toJSON === "function";
}

// web-sys exposes these WebAuthn JSON methods only behind unstable bindings.
// Keep the stable, standards-defined calls here rather than hand-decoding any
// base64url fields, which would drift when the credential dictionaries evolve.
export function jaunderCreationOptionsFromJson(options) {
    return PublicKeyCredential.parseCreationOptionsFromJSON(options.publicKey ?? options);
}

export function jaunderRequestOptionsFromJson(options) {
    return PublicKeyCredential.parseRequestOptionsFromJSON(options.publicKey ?? options);
}

export function jaunderCredentialToJson(credential) {
    return credential.toJSON();
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = jaunderWebauthnSupported)]
    fn browser_supported(navigator: &web_sys::Navigator) -> bool;

    #[wasm_bindgen::prelude::wasm_bindgen(catch, js_name = jaunderCreationOptionsFromJson)]
    fn creation_options_from_json(
        options: &JsValue,
    ) -> Result<PublicKeyCredentialCreationOptions, JsValue>;

    #[wasm_bindgen::prelude::wasm_bindgen(catch, js_name = jaunderRequestOptionsFromJson)]
    fn request_options_from_json(
        options: &JsValue,
    ) -> Result<PublicKeyCredentialRequestOptions, JsValue>;

    #[wasm_bindgen::prelude::wasm_bindgen(catch, js_name = jaunderCredentialToJson)]
    fn credential_to_json(credential: &PublicKeyCredential) -> Result<JsValue, JsValue>;
}

/// Whether this browser exposes the explicit `WebAuthn` credentials API.
#[must_use]
pub fn is_supported() -> bool {
    #[cfg(test)]
    if let Some(seam) = test_seam() {
        return matches!(seam.capability, TestCapability::Complete);
    }

    web_sys::window().is_some_and(|window| browser_supported(&window.navigator()))
}

/// Explicitly create a credential from a server-provided `WebAuthn` JSON request.
pub async fn create(options: &Value) -> CeremonyOutcome<Value> {
    let Some(credentials) = credentials() else {
        return CeremonyOutcome::Unsupported;
    };
    let Ok(options) = json_value(options) else {
        return CeremonyOutcome::Failed;
    };
    let Ok(public_key) = creation_options_from_json(&options) else {
        return CeremonyOutcome::Failed;
    };
    let request = CredentialCreationOptions::new();
    request.set_public_key(&public_key);

    complete(invoke_create(&credentials, &request)).await
}

/// Explicitly request a credential assertion from a server-provided `WebAuthn` JSON request.
pub async fn get(options: &Value) -> CeremonyOutcome<Value> {
    let Some(credentials) = credentials() else {
        return CeremonyOutcome::Unsupported;
    };
    let Ok(options) = json_value(options) else {
        return CeremonyOutcome::Failed;
    };
    let Ok(public_key) = request_options_from_json(&options) else {
        return CeremonyOutcome::Failed;
    };
    let request = CredentialRequestOptions::new();
    request.set_public_key(&public_key);

    complete(invoke_get(&credentials, &request)).await
}

fn credentials() -> Option<CredentialsContainer> {
    is_supported().then(|| web_sys::window().map(|window| window.navigator().credentials()))?
}

fn json_value(value: &Value) -> Result<JsValue, ()> {
    serde_json::to_string(value)
        .ok()
        .and_then(|json| JSON::parse(&json).ok())
        .ok_or(())
}

async fn complete(pending: Result<Promise, JsValue>) -> CeremonyOutcome<Value> {
    let pending = match pending {
        Ok(pending) => pending,
        Err(error) => return error_outcome(&error),
    };
    let credential = match JsFuture::from(pending).await {
        Ok(credential) => credential,
        Err(error) => return error_outcome(&error),
    };
    let Ok(credential) = credential.dyn_into::<PublicKeyCredential>() else {
        return CeremonyOutcome::Failed;
    };
    let Ok(response) = credential_to_json(&credential) else {
        return CeremonyOutcome::Failed;
    };
    JSON::stringify(&response)
        .ok()
        .and_then(|json| json.as_string())
        .and_then(|json| serde_json::from_str(&json).ok())
        .map_or(CeremonyOutcome::Failed, CeremonyOutcome::Success)
}

fn invoke_create(
    credentials: &CredentialsContainer,
    options: &CredentialCreationOptions,
) -> Result<Promise, JsValue> {
    #[cfg(test)]
    if let Some(error_name) = test_seam().and_then(|seam| seam.rejection) {
        return Ok(rejected_promise(error_name));
    }

    credentials.create_with_options(options)
}

fn invoke_get(
    credentials: &CredentialsContainer,
    options: &CredentialRequestOptions,
) -> Result<Promise, JsValue> {
    #[cfg(test)]
    if let Some(error_name) = test_seam().and_then(|seam| seam.rejection) {
        return Ok(rejected_promise(error_name));
    }

    credentials.get_with_options(options)
}

fn error_outcome<T>(error: &JsValue) -> CeremonyOutcome<T> {
    if Reflect::get(error, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .is_some_and(|name| name == "NotAllowedError")
    {
        CeremonyOutcome::Cancelled
    } else {
        CeremonyOutcome::Failed
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum TestCapability {
    Complete,
    MissingPublicKeyCredential,
    MissingCredentialsCreate,
    MissingCredentialsGet,
    MissingCreationOptionsJson,
    MissingRequestOptionsJson,
    MissingCredentialToJson,
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct TestSeam {
    capability: TestCapability,
    rejection: Option<&'static str>,
}

#[cfg(test)]
std::thread_local! {
    static TEST_SEAM: std::cell::Cell<Option<TestSeam>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn test_seam() -> Option<TestSeam> {
    TEST_SEAM.with(std::cell::Cell::get)
}

#[cfg(test)]
fn set_test_seam(seam: Option<TestSeam>) {
    TEST_SEAM.with(|slot| slot.set(seam));
}

#[cfg(test)]
fn rejected_promise(name: &str) -> Promise {
    let error = js_sys::Object::new();
    Reflect::set(&error, &JsValue::from_str("name"), &JsValue::from_str(name))
        .expect("test error name is writable");
    Promise::reject(&error)
}

#[cfg(test)]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function jaunderTestCredential() {
    return {
        toJSON() {
            return {
                id: "AQI",
                rawId: "AQI",
                type: "public-key",
                response: {
                    clientDataJSON: "AwQ",
                    attestationObject: "BQY"
                },
                clientExtensionResults: {}
            };
        }
    };
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = jaunderTestCredential)]
    fn test_credential() -> PublicKeyCredential;
}

#[cfg(test)]
mod tests {
    use super::{
        CeremonyOutcome, TestCapability, TestSeam, create, creation_options_from_json,
        credential_to_json, get, is_supported, json_value, set_test_seam, test_credential,
    };
    use js_sys::{Reflect, Uint8Array};
    use serde_json::json;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_test::wasm_bindgen_test;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    fn creation_request() -> serde_json::Value {
        json!({
            "publicKey": {
                "challenge": "AAE",
                "rp": { "name": "Jaunder", "id": "example.test" },
                "user": { "id": "AgM", "name": "user", "displayName": "User" },
                "pubKeyCredParams": [{ "type": "public-key", "alg": -7 }]
            }
        })
    }

    #[wasm_bindgen_test]
    fn protocol_conversion_preserves_binary_base64url_fields() {
        let options = json_value(&creation_request()).expect("request converts to JS");
        let browser_options =
            creation_options_from_json(&options).expect("browser parses WebAuthn JSON options");
        let challenge = Reflect::get(browser_options.as_ref(), &JsValue::from_str("challenge"))
            .expect("challenge field");
        let user =
            Reflect::get(browser_options.as_ref(), &JsValue::from_str("user")).expect("user field");
        let user_id = Reflect::get(&user, &JsValue::from_str("id")).expect("user id field");

        assert_eq!(Uint8Array::new(&challenge).to_vec(), [0, 1]);
        assert_eq!(Uint8Array::new(&user_id).to_vec(), [2, 3]);

        let response =
            credential_to_json(&test_credential()).expect("credential serializes to JSON");
        let encoded = js_sys::JSON::stringify(&response)
            .expect("JSON stringify")
            .as_string()
            .expect("JSON string");
        let response: serde_json::Value = serde_json::from_str(&encoded).expect("response JSON");
        assert_eq!(response["rawId"], "AQI");
        assert_eq!(response["response"]["clientDataJSON"], "AwQ");
        assert_eq!(response["response"]["attestationObject"], "BQY");
    }

    #[wasm_bindgen_test(async)]
    async fn capability_seam_distinguishes_supported_and_unsupported() {
        for capability in [
            TestCapability::MissingPublicKeyCredential,
            TestCapability::MissingCredentialsCreate,
            TestCapability::MissingCredentialsGet,
            TestCapability::MissingCreationOptionsJson,
            TestCapability::MissingRequestOptionsJson,
            TestCapability::MissingCredentialToJson,
        ] {
            set_test_seam(Some(TestSeam {
                capability,
                rejection: None,
            }));
            assert!(!is_supported());
            assert!(matches!(
                create(&creation_request()).await,
                CeremonyOutcome::Unsupported
            ));
            assert!(matches!(
                get(&json!({ "publicKey": { "challenge": "AAE", "rpId": "example.test" } })).await,
                CeremonyOutcome::Unsupported
            ));
        }

        set_test_seam(Some(TestSeam {
            capability: TestCapability::Complete,
            rejection: None,
        }));
        assert!(is_supported());
        set_test_seam(None);
    }

    #[wasm_bindgen_test(async)]
    async fn cancellation_is_not_a_failure() {
        set_test_seam(Some(TestSeam {
            capability: TestCapability::Complete,
            rejection: Some("NotAllowedError"),
        }));
        let outcome = create(&creation_request()).await;
        set_test_seam(None);
        assert!(matches!(outcome, CeremonyOutcome::Cancelled));
    }

    #[wasm_bindgen_test(async)]
    async fn thrown_failure_has_no_exception_payload() {
        set_test_seam(Some(TestSeam {
            capability: TestCapability::Complete,
            rejection: Some("InvalidStateError"),
        }));
        let outcome =
            get(&json!({ "publicKey": { "challenge": "AAE", "rpId": "example.test" } })).await;
        set_test_seam(None);
        assert!(matches!(outcome, CeremonyOutcome::Failed));
    }
}
