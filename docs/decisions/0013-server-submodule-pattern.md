# ADR-0013: Server Submodule Pattern for Web-Layer Modules

* Status: accepted
* Deciders: mdorman, Claude Sonnet
* Date: 2026-05-23

## Context and Problem Statement

Each feature module in `web/src/` (e.g. `auth.rs`, `posts.rs`, `profile.rs`) mixes three distinct concerns in a single flat file:

1. **The Seam** — `#[server]` functions and shared serialization types (DTOs) that must compile on both the WASM client and the SSR server.
2. **The Server Implementation** — DB queries, Axum context extractions, cryptography.
3. **The Web Dependencies** — server-only imports (`axum`, `storage`, `chrono`, etc.) that cannot compile under WASM.

The result is a proliferation of `#[cfg(feature = "ssr")]` annotations threaded through every file. In the largest modules (`posts.rs` at 1,200+ lines, `auth.rs` at 571 lines) this makes the code difficult to read and reason about. The compiler also provides no structural guarantee that server-only code is actually isolated from client compilation.

## Decision Drivers

* Readability: `#[server]` function bodies should read as ordinary Rust, without per-import cfg guards.
* Structural isolation: the compiler should enforce the WASM/SSR boundary via module gating, not scattered annotations.
* Testability: server-only helpers should live in a module that can grow its own `#[cfg(test)]` block without noise.

## Decision Outcome

Chosen option: **directory modules with a conditionally-compiled `server` submodule**, because it moves the WASM/SSR boundary from annotations scattered throughout function bodies to a single `#[cfg(feature = "ssr")] mod server;` declaration at the module root, and leverages the fact that `#[server]` bodies are already cfg-gated to SSR by Leptos's proc-macro.

### Canonical Structure

```text
web/src/feature/
├── mod.rs     # The Seam: shared DTOs + `#[server]` functions with real bodies
└── server.rs  # Module-private helpers, transactions, and unit tests
               # (omitted for small features that need no shared helpers)
```

### Canonical `mod.rs` shape

```rust
#[cfg(feature = "ssr")]
mod server;
#[cfg(feature = "ssr")]
use server::*;          // brings all server-only helpers into scope

// Shared DTOs — no cfg needed
#[derive(Serialize, Deserialize)]
pub struct FeatureData { ... }

// Real implementation directly in the #[server] body
#[server]
pub async fn do_thing(input: String) -> WebResult<FeatureData> {
    boundary!("do_thing", {
        let user = require_auth().await?;
        // ... full implementation here
        Ok(result)
    })
}
```

Key properties:
- `server.rs` is only compiled in SSR mode. The WASM build never sees it.
- `#[server]` bodies are already SSR-only (Leptos replaces the body with HTTP-call boilerplate on WASM), so they can freely reference items from `server.rs` without additional cfg guards.
- `server.rs` exists only when there is genuine module-private logic worth naming — complex helpers, multi-step transactions, tests. Small modules may not need one.

### The `boundary!` Macro

The `web_server_fn!` macro previously handled cfg-gating (now redundant) and `server_boundary` wrapping (still needed for tracing and error context). It is replaced by a slim `boundary!` macro:

```rust
macro_rules! boundary {
    ($name:expr, $body:block) => {
        $crate::error::server_boundary($name, async move $body).await
    };
}
```

Every `#[server]` body is wrapped with `boundary!("function_name", { ... })`.

### Auth Re-export Surface

`require_auth`, `AuthUser`, `set_session_cookie`, `clear_session_cookie`, `CookieSettings`, and `AuthRejection` are consumed by every other server module. They live in `auth/server.rs` and are re-exported from `auth/mod.rs` under `#[cfg(feature = "ssr")] pub use server::{...};` so that other modules' `use crate::auth::*;` continues to resolve correctly.

## Consequences

* Good: `#[server]` function bodies are free of cfg noise and read as ordinary, sequential Rust.
* Good: The compiler enforces the WASM/SSR split structurally rather than relying on correctly placed annotations.
* Good: `server.rs` gains a focused scope for unit tests, which no longer need to sit alongside unrelated view-logic types.
* Bad: Directory modules are slightly more work to create than appending to a flat file. The pattern is self-consistent once established, but new contributors must know to reach for `mod.rs` + optional `server.rs` rather than a flat file.
* Neutral: Small features that need no `server.rs` remain flat inside their `mod.rs`; the directory structure is not a mandate to split for its own sake.
