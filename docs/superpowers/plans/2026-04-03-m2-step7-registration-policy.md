# M2 Step 7: RegistrationPolicy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `RegistrationPolicy` enum (`Open`, `InviteOnly`, `Closed`) with `FromStr`/`Display`, and a `load_registration_policy` async helper that reads the policy from the site config store.

**Architecture:** Everything lives in `server/src/auth.rs` alongside the existing `AuthUser` and `generate_token` code. `load_registration_policy` takes `&dyn SiteConfigStorage` so it is testable with any store implementation and usable from server functions that hold `Arc<AppState>`. Unit tests sit in the same file.

**Tech Stack:** Rust, `thiserror`, `sqlx` (in-memory SQLite for async tests), `tokio::test`

---

### Task 1: RegistrationPolicy enum, load_registration_policy, and tests

**Files:**
- Modify: `server/src/auth.rs`

- [ ] **Step 1: Write all tests first (they will fail to compile)**

Add a `#[cfg(test)]` module at the bottom of `server/src/auth.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteSiteConfigStorage;

    // --- FromStr / Display ---

    #[test]
    fn open_parses() {
        assert_eq!(
            "open".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::Open
        );
    }

    #[test]
    fn invite_only_parses() {
        assert_eq!(
            "invite_only".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::InviteOnly
        );
    }

    #[test]
    fn closed_parses() {
        assert_eq!(
            "closed".parse::<RegistrationPolicy>().unwrap(),
            RegistrationPolicy::Closed
        );
    }

    #[test]
    fn unknown_string_returns_error() {
        assert!("unknown".parse::<RegistrationPolicy>().is_err());
    }

    #[test]
    fn display_round_trips() {
        for policy in [
            RegistrationPolicy::Open,
            RegistrationPolicy::InviteOnly,
            RegistrationPolicy::Closed,
        ] {
            assert_eq!(
                policy.to_string().parse::<RegistrationPolicy>().unwrap(),
                policy
            );
        }
    }

    // --- load_registration_policy ---

    async fn in_memory_store() -> SqliteSiteConfigStorage {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        SqliteSiteConfigStorage::new(pool)
    }

    #[tokio::test]
    async fn absent_key_returns_closed() {
        let store = in_memory_store().await;
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::Closed
        );
    }

    #[tokio::test]
    async fn key_set_to_open_returns_open() {
        let store = in_memory_store().await;
        store
            .set("site.registration_policy", "open")
            .await
            .unwrap();
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::Open
        );
    }

    #[tokio::test]
    async fn key_set_to_invite_only_returns_invite_only() {
        let store = in_memory_store().await;
        store
            .set("site.registration_policy", "invite_only")
            .await
            .unwrap();
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::InviteOnly
        );
    }

    #[tokio::test]
    async fn invalid_value_in_db_returns_closed() {
        let store = in_memory_store().await;
        store
            .set("site.registration_policy", "garbage")
            .await
            .unwrap();
        assert_eq!(
            load_registration_policy(&store).await,
            RegistrationPolicy::Closed
        );
    }
}
```

- [ ] **Step 2: Run tests to verify they fail to compile**

```bash
cargo nextest run -E 'test(registration_policy)'
```

Expected: compile error — `RegistrationPolicy` not found, `load_registration_policy` not found.

- [ ] **Step 3: Add imports and implement RegistrationPolicy and load_registration_policy**

Update the imports at the top of `server/src/auth.rs` (add to existing `use std::sync::Arc;` block and add new `use` lines):

```rust
use std::{fmt, str::FromStr, sync::Arc};

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use leptos::prelude::ServerFnError;
use rand::RngCore;
use thiserror::Error;

use crate::storage::{AppState, SiteConfigStorage};
use crate::username::Username;
```

Then add the following after the `generate_token` function (before `AuthUser`):

```rust
/// The site's user-registration access policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationPolicy {
    /// Anyone may register without a code.
    Open,
    /// New accounts require a valid, unused invite code.
    InviteOnly,
    /// Registration is disabled; no new accounts can be created.
    Closed,
}

impl fmt::Display for RegistrationPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistrationPolicy::Open => write!(f, "open"),
            RegistrationPolicy::InviteOnly => write!(f, "invite_only"),
            RegistrationPolicy::Closed => write!(f, "closed"),
        }
    }
}

/// Error returned when a string does not name a valid [`RegistrationPolicy`].
#[derive(Debug, Error)]
#[error("invalid registration policy: {0:?}")]
pub struct InvalidRegistrationPolicy(String);

impl FromStr for RegistrationPolicy {
    type Err = InvalidRegistrationPolicy;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(RegistrationPolicy::Open),
            "invite_only" => Ok(RegistrationPolicy::InviteOnly),
            "closed" => Ok(RegistrationPolicy::Closed),
            other => Err(InvalidRegistrationPolicy(other.to_owned())),
        }
    }
}

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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -E 'test(registration_policy) | test(open_parses) | test(invite_only_parses) | test(closed_parses) | test(unknown_string) | test(absent_key) | test(key_set_to) | test(invalid_value) | test(display_round)'
```

Expected: all new tests PASS. Then run the full suite:

```bash
cargo nextest run
```

Expected: all tests pass (was 55, now 60).

- [ ] **Step 5: Check formatting and lint**

```bash
cargo fmt --check && cargo clippy -- -D warnings
```

Expected: no output (all clean).

- [ ] **Step 6: Commit**

```bash
git add server/src/auth.rs docs/milestones/M2.md
git commit -m "M2.7.1-M2.7.3: RegistrationPolicy enum, FromStr/Display, and load_registration_policy"
```

Then check off items M2.7.1–M2.7.3 in `docs/milestones/M2.md` and commit that separately:

```bash
git add docs/milestones/M2.md
git commit -m "M2.7.1-M2.7.3: Check off completed milestone items"
```
