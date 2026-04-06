---
root: false
targets:
  - '*'
---
- EVERY http endpoint must have an integration test.
- EVERY http endpoint must have an end to end test.
- NEVER commit a change without accompanying tests unless the user explicitly waives testing for that change.
- NEVER remove functionality to pass tests.
- ALWAYS place unit tests in the same file as the code being tested.
- ALWAYS place integration tests in `tests/`, mirroring the source path.
- ALWAYS place e2e tests in `e2e/` using `playwright`.
- **In-Memory Database:** For tests requiring a database, use `sqlite::memory:` and run migrations using `sqlx::migrate!("./migrations").run(&pool).await?` before creating the `AppState`.
- **Unwrap in Tests:** While `.unwrap()` and `.expect()` are forbidden in production code, they are **permitted and encouraged** in test functions and test helpers to signal immediate failure on setup errors.
- **E2E Hydration:** In Playwright tests, call `waitForHydration(page)` (defined in `end2end/tests/auth.spec.ts`) before filling any form fields. Leptos `prop:value` bindings reset input values during WASM hydration; filling before hydration completes sends empty fields to the server.
