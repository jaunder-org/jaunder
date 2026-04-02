---
paths:
  - '*.rs'
---
- **NEVER** use .unwrap().
- **NEVER** use .expect() in production code.
- **Storage Errors:** Use specialized error enums in `common::storage` (e.g., `UserAuthError`, `CreateUserError`) for storage trait methods. Use `thiserror` for these enums.
- **Server Function Errors:** Convert storage errors to `leptos::prelude::ServerFnError` using `.map_err(|e| ServerFnError::new(e.to_string()))`.
- **Input Validation:** Enforce lowercase for usernames at the boundary (CLI or Server Function) before passing them to storage methods.
