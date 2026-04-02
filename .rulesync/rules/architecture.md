---
root: false
targets:
  - '*'
---
- `common` contains code shared between other packages
- `server` contains the back-end
- `web` contains leptos components
- `hydrate` contains the front-end driver
- `end2end` contains the end-to-end tests
- Use Rust's type system to make invalid states impossible using infallible types.
- At the boundary (`#[server]` functions, DB calls): parse data into infallible types, reject invalid data, handle `Result`/`Option` conversion.
- Leptos components only render data; business logic belongs in server functions or pure transformation functions.
- Data transformations should be pure functions, keeping them easy to test and reason about.
- **Storage Traits:** Define all storage traits (e.g., `UserStorage`, `SessionStorage`) and record types in `common/src/storage.rs`. This allows both `web` and `server` crates to use them without circular dependencies.
- **AppState:** Use the `AppState` struct from `common::storage` to bundle storage handles. In `web` (server functions), retrieve it via `expect_context::<Arc<AppState>>()`.
- **Implementations:** Keep concrete SQLite/Postgres implementations in the `server` crate (e.g., `server/src/storage/sqlite.rs`). Re-export these in `server/src/storage/mod.rs` for use by the CLI and server runner.
