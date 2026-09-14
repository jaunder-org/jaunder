//! Passkey ceremony and credential-management API surface.

mod api;
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(any(target_arch = "wasm32", test))]
mod page_state;

pub use api::{
    AuthenticationStart, Availability, BrowserCredentialId, BrowserPasskeyLabel, CeremonyHandle,
    CredentialInfo, Delete, FinishAuthentication, FinishRegistration, List, RegistrationStart,
    StartAuthentication, StartRegistration, availability, delete, finish_authentication,
    finish_registration, list, start_authentication, start_registration,
};
#[cfg(target_arch = "wasm32")]
pub use component::PasskeysPage;
