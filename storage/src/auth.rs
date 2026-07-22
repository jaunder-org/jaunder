use crate::SiteConfigStorage;

// `RegistrationPolicy` now lives in `common` (importable ungated from the wasm
// client, per ADR-0065); storage re-exports it so `storage::RegistrationPolicy`
// call sites keep resolving. `load_registration_policy` stays here — it needs
// `SiteConfigStorage`.
pub use common::registration::RegistrationPolicy;

// ---------------------------------------------------------------------------
// load_registration_policy
// ---------------------------------------------------------------------------

/// Reads `site.registration_policy` from the config store and parses it.
///
/// Returns [`RegistrationPolicy::Closed`] when the key is absent or its
/// value cannot be parsed — a safe default that prevents unintended open
/// registration on a freshly initialised instance.
pub async fn load_registration_policy(store: &dyn SiteConfigStorage) -> RegistrationPolicy {
    store
        .get("site.registration_policy")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RegistrationPolicy::Closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{backends, Backend};
    use rstest::*;
    use rstest_reuse::*;

    // --- load_registration_policy ---
    // (type-behavior tests — FromStr / Display / serde — live with the type in
    // `common::registration`.)

    #[apply(backends)]
    #[tokio::test]
    async fn absent_key_returns_closed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        assert_eq!(
            load_registration_policy(store).await,
            RegistrationPolicy::Closed
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn key_set_to_open_returns_open(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store.set("site.registration_policy", "open").await.unwrap();
        assert_eq!(
            load_registration_policy(store).await,
            RegistrationPolicy::Open
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn key_set_to_invite_only_returns_invite_only(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set("site.registration_policy", "invite_only")
            .await
            .unwrap();
        assert_eq!(
            load_registration_policy(store).await,
            RegistrationPolicy::InviteOnly
        );
    }

    #[apply(backends)]
    #[tokio::test]
    async fn invalid_value_in_db_returns_closed(#[case] backend: Backend) {
        let env = backend.setup().await;
        let store = &*env.state.site_config;
        store
            .set("site.registration_policy", "garbage")
            .await
            .unwrap();
        assert_eq!(
            load_registration_policy(store).await,
            RegistrationPolicy::Closed
        );
    }
}
