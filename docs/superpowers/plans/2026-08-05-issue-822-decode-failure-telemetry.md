# Arg-decode failure telemetry — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-08-05-issue-822-decode-failure-telemetry.md`](../specs/2026-08-05-issue-822-decode-failure-telemetry.md)
— the "what/why". This plan is the "how".

**Goal:** Make a failed `#[server]` argument decode emit the boundary telemetry
it currently skips, and correct the two ADR-0065 passages that describe the
pre-typed-arg world.

**Architecture:** One small server-gated helper in `web/src/error.rs`, called
from `FromServerFnError::from_server_fn_error` — the only place that sees the
typed `ServerFnErrorErr` at decode time. It reuses the existing `InternalError`
vocabulary and emits; it does **not** change the returned `WebError`, so the
wire shape is untouched.

**Tech Stack:** Rust, leptos `server_fn` 0.8.12, `tracing` +
`tracing-subscriber` test capture, `cargo nextest`, `cargo xtask check`.

## Review header

**Scope — in:** `web/src/error.rs` (the helper, its call site, and unit tests),
`server/tests/web/web_password_reset.rs` (wire-shape assertion),
`docs/adr/0065-client-side-domain-validation.md` (in-place amendment).

**Scope — out:** the enforcement gate for "wire-arg errors never echo input"
(Task 1 files it), any change to
`metrics::login`/`registration`/`password_reset`, restoring the deleted
`parse_password` spans, re-deriving fn identity at decode time.

**Tasks:**

1. File the D6 enforcement-gate follow-up.
2. Emit boundary telemetry on arg-decode failure, with unit tests.
3. Pin the decode-failure wire shape in an integration test.
4. Amend ADR-0065 in place.

**Key risks / decisions:**

- **The cfg gate is load-bearing.** `web` compiles to wasm and `host::metrics`
  is native-only, so the helper and its call must be
  `#[cfg(feature = "server")]`. A mistake here breaks the wasm build — caught by
  `cargo xtask check`'s wasm clippy step, not by a host build.
- **Use `validation_source`, not `validation`.** The latter sets `source: None`
  and would emit an empty `error.source`, silently defeating the change.
- **Telemetry only.** `from_server_fn_error` must still return
  `WebError::server_function(value.to_string())` unchanged.

## Global Constraints

- **Per-commit gate** — run `cargo xtask check` before each commit
  (**`jaunder-commit`**). **No `Co-Authored-By` trailer.**
- **Suppressing a lint needs user approval** — fix the code instead.
- **Crate names** — `cargo nextest run -p web …` for the unit tests; the
  integration tests live in package `jaunder`
  (`cargo nextest run -p jaunder …`). Bare nextest cannot run `case_2_postgres`
  locally; filter to sqlite and let the gate cover both.
- **ADR-0011 discipline** — no new metric name, no new bounded-enum variant, and
  nothing unbounded may reach a metric attribute.

---

### Task 1: File the enforcement-gate follow-up

The spec (D3) verifies by inspection that every wire-arg newtype's error message
is input-free except `BackupSchedule`. Making that a guarantee is a new
static-analysis gate with its own design — out of scope here (D6).

**Files:** none (tracker-only).

- [x] **Step 1: File the issue** via **`jaunder-issues`**.

Title: `xtask gate: a wire-arg newtype's error message must not echo its input`

Body must state: typed `#[server]` wire args now surface their `FromStr` error
text in `error.source` on the decode path (#822), which is exported to trace
backends, so ADR-0011's PII discipline applies to those messages. Today the
property holds by inspection — nearly every newtype error is a unit struct
interpolating only constants; `Email` wraps `email_address::Error` (17 unit
variants, const-only `Display`) and `Filename` interpolates a byte count. The
exception is `BackupSchedule`, which wraps `croner::CronError`, whose
`InvalidPattern`/`IllegalCharacters`/`ComponentError` variants carry `String`
payloads that can echo fragments of the submitted cron expression (not PII;
operator-only endpoint). Nothing prevents a future newtype — or a dependency
bump — from interpolating genuine user input. Propose a gate that, for each type
used as a `#[server]` wire arg, asserts its `FromStr::Err` `Display` contains no
runtime-formatted value other than a constant.

- [x] **Step 2: Record the number.** Filed as
      [#846](https://github.com/jaunder-org/jaunder/issues/846) — type `Task`,
      labels `tooling`/`type-safety`, milestone "Observability & diagnostics",
      P3.

No commit — the tracker is the deliverable.

---

### Task 2: Emit boundary telemetry on arg-decode failure

**Files:**

- Modify: `web/src/error.rs` — `from_server_fn_error` (`:67-73`) plus a new
  server-gated helper; unit tests in the existing `#[cfg(test)] mod tests`.

**Interfaces:**

- Consumes:
  `host::error::InternalError::{validation_source, with_context, emit_boundary_failure}`
  (all `pub`, re-exported through `web::error` under `feature = "server"` at
  `:11-12`).
- Produces: no public API change. `WebError` and `from_server_fn_error`'s return
  value are unchanged.

- [ ] **Step 1: Write the failing tests**

Add to `web/src/error.rs`'s test module. These need a field-capturing layer (the
existing `boundary_failure_event_carries_the_enclosing_instrument_span` at
`:496-553` captures span _scopes_; this captures event _fields_, so it is a new
helper, not a reuse):

```rust
    /// Records `(field, value)` pairs for every event, so a test can assert on the
    /// boundary event's structured fields rather than its rendered text.
    #[cfg(feature = "server")]
    struct FieldRecorder(std::sync::Arc<std::sync::Mutex<Vec<Vec<(String, String)>>>>);

    #[cfg(feature = "server")]
    impl<S: tracing::Subscriber> tracing_subscriber::layer::Layer<S> for FieldRecorder {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Visitor(Vec<(String, String)>);
            impl tracing::field::Visit for Visitor {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    self.0.push((field.name().to_string(), format!("{value:?}")));
                }
            }
            let mut visitor = Visitor(Vec::new());
            event.record(&mut visitor);
            self.0.lock().expect("field recorder mutex").push(visitor.0);
        }
    }

    #[cfg(feature = "server")]
    #[test]
    fn arg_decode_failure_emits_a_boundary_event() {
        use tracing_subscriber::prelude::*;

        let events = std::sync::Arc::default();
        let subscriber =
            tracing_subscriber::registry().with(FieldRecorder(std::sync::Arc::clone(&events)));
        let _guard = tracing::subscriber::set_default(subscriber);

        let error = WebError::from_server_fn_error(ServerFnErrorErr::Args(
            "invalid value for `password`: password must be at least 8 characters".into(),
        ));

        // The response is unchanged: telemetry is additive.
        assert!(matches!(error, WebError::ServerFunction { .. }));

        let recorded = events.lock().expect("field recorder mutex").clone();
        let fields: Vec<(String, String)> = recorded.into_iter().flatten().collect();
        let get = |name: &str| {
            fields
                .iter()
                .find(|(f, _)| f == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert!(
            get("error.kind").contains("Validation"),
            "expected a Validation-kind boundary event; got {fields:?}"
        );
        assert!(get("error.class").contains("Client"));
        // `stage = decode` is what distinguishes this from an in-body validation failure.
        assert!(get("error.context").contains("decode"), "fields: {fields:?}");
        // The deserializer's message reaches `error.source` — the diagnostic payload.
        assert!(get("error.source").contains("at least 8 characters"));
        // AC1 names the event itself and its public message; pin both so a future
        // refactor cannot quietly emit a *different* event and still pass.
        assert!(get("message").contains("server function failed"), "fields: {fields:?}");
        assert!(get("error.public").contains("invalid request arguments"));
    }

    #[cfg(feature = "server")]
    #[test]
    fn non_decode_server_fn_errors_emit_nothing() {
        use tracing_subscriber::prelude::*;

        let events: std::sync::Arc<std::sync::Mutex<Vec<Vec<(String, String)>>>> =
            std::sync::Arc::default();
        let subscriber =
            tracing_subscriber::registry().with(FieldRecorder(std::sync::Arc::clone(&events)));
        let _guard = tracing::subscriber::set_default(subscriber);

        // Transport-side: not an arg-decode failure, so it must stay silent.
        let _ = WebError::from_server_fn_error(ServerFnErrorErr::Request("connection reset".into()));

        assert!(
            events.lock().expect("field recorder mutex").is_empty(),
            "a non-decode variant must not emit a boundary event"
        );
    }
```

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd <worktree> -- cargo nextest run -p web --features server decode`

Expected: FAIL — the code **compiles** (the helper is not referenced by the
tests yet) and fails at runtime because nothing emits. Precisely: the first
failing assertion is `get("error.kind").contains("Validation")` against an empty
string, not an events-are-empty assertion.
(`non_decode_server_fn_errors_emit_nothing` passes trivially at this point; that
is fine — it is the guard against Task 2 over-firing.)

**The event is DEBUG level, and that is the one thing that could silently zero
this test.** `emit_boundary_failure` picks its level from `ErrorClass`
(`host/src/error.rs:68`), and `Client → Level::DEBUG` — unlike the existing
ERROR-level fixture at `:496-553`. A bare `tracing_subscriber::registry()`
applies no filter, so the event is captured; but if a filter is ever added to
this test, DEBUG is what it must admit.

- [ ] **Step 3: Implement against the tests**

In `web/src/error.rs`, add the helper above the `FromServerFnError` impl and
call it:

```rust
/// Emits the boundary telemetry for a failed `#[server]` **argument decode**.
///
/// Arg deserialization happens in leptos's `from_req`, *before* the generated
/// `__server_<ident>` fn runs — so neither the `web.<vertical>.<ident>` span nor
/// [`server_boundary`] is reached, and a malformed request would otherwise leave no
/// trace at all (#822). This restores the standard boundary event and error metric for
/// that path, reusing the existing vocabulary: `Validation`/`Client`, plus a
/// `stage = decode` context entry that distinguishes it from an in-body validation
/// failure.
///
/// Only arg-decode variants emit. `ServerError`/`MiddlewareError`/`Response` are
/// server-side too, but they arise *downstream* of decode and are already covered by
/// the in-body boundary; the predicate here is "this request's arguments were
/// malformed".
///
/// The failing fn is identified by the enclosing request span's `uri`
/// (`server/src/observability.rs`), not by a span name — see the spec's D4.
#[cfg(feature = "server")]
fn emit_arg_decode_failure(value: &ServerFnErrorErr) {
    if !matches!(
        value,
        ServerFnErrorErr::Args(_)
            | ServerFnErrorErr::MissingArg(_)
            | ServerFnErrorErr::Deserialization(_)
    ) {
        return;
    }
    // `validation_source` (not `validation`) — the latter carries no source and would
    // emit an empty `error.source`, which is the diagnostic payload. `ServerFnErrorErr`
    // is `Clone` + `thiserror::Error`, so it satisfies the source bound directly.
    InternalError::validation_source("invalid request arguments", value.clone())
        .with_context("stage", "decode")
        .emit_boundary_failure();
}
```

and in the impl:

```rust
    fn from_server_fn_error(value: ServerFnErrorErr) -> Self {
        #[cfg(feature = "server")]
        emit_arg_decode_failure(&value);
        Self::server_function(value.to_string())
    }
```

Note the return value is untouched — the emit is additive.

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd <worktree> -- cargo nextest run -p web --features server`

Expected: PASS, including the existing
`boundary_failure_event_carries_the_enclosing_instrument_span`.

- [ ] **Step 5: Commit**

Run `cargo xtask check` first (**`jaunder-commit`**) — its **wasm clippy** step
is what proves the cfg gate is right; a host-only build will not.

```bash
git add web/src/error.rs
git commit -m "feat(web): emit boundary telemetry when a server-fn arg fails to decode (#822)"
```

---

### Task 3: Pin the decode-failure wire shape

**Files:**

- Modify: `server/tests/web/web_password_reset.rs` — the too-short-password case
  (comment at `:305`, assertion at `:333`).

**Interfaces:**

- Consumes: Task 2's behaviour (indirectly — this pins the response, which Task
  2 must not change).
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Strengthen the assertion**

`confirm_password_reset_with_short_password_returns_error` (`:310`) currently
asserts only `assert_ne!(status, StatusCode::OK)` at `:333`, which cannot tell a
decode rejection from an in-body failure — both are HTTP 500.

**Mind two traps.** The response body is discarded today, and the name `body` is
already taken by the _request_ form (`:323-331`):

```rust
    let body = format!("token={raw_token}&new_password=short");
    let (status, _body) = post_form_with_mailer(
        &state, &mailer, <web::password_reset::Confirm as ServerFn>::PATH, body, None,
    )
    .await;
```

Asserting on `body` would therefore be a use-after-move **and** would inspect
the request. Bind the response under a distinct name:

```rust
    let (status, response_body) = post_form_with_mailer(
        &state, &mailer, <web::password_reset::Confirm as ServerFn>::PATH, body, None,
    )
    .await;

    // A decode rejection is HTTP 500 with a body tagged `server_function` — distinct
    // from an in-body failure, which projects to `validation`/`unauthorized`/etc.
    // (`WebError` is externally tagged, snake_case.) This is the wire contract the
    // decode-telemetry path in `web::error` sits behind (#822).
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response_body.contains("server_function"),
        "expected a server-fn decode rejection; body: {response_body}"
    );
```

`post_form_with_mailer` already returns `(StatusCode, String)`
(`server/tests/helpers/mod.rs:506-528`), so no helper change is needed.

- [ ] **Step 2: Run it**

Run:
`devtool run --cwd <worktree> -- cargo nextest run -p jaunder -E 'test(password_reset) and test(sqlite)'`

Expected: PASS. If the body is not tagged `server_function`, stop — that
falsifies the spec's D1 claim about which path produces the response, and the
spec needs revisiting rather than the assertion being loosened.

- [ ] **Step 3: Commit**

```bash
git add server/tests/web/web_password_reset.rs
git commit -m "test(web): pin the wire shape of a server-fn arg-decode rejection (#822)"
```

---

### Task 4: Amend ADR-0065 in place

**Files:**

- Modify: `docs/adr/0065-client-side-domain-validation.md` — header note
  (`:6-8`), the secret exception (`:76-78`), the Consequences sentence
  (`:112-113`).

- [ ] **Step 1: Rewrite the two passages**

Follow #568's markup — it is the only prior amendment that used it in full.

**Header** — extend the existing `- Note:` block with a second amendment line
naming #822 and summarising both changes.

**Secret exception (`:76-78`)** — replace. The premise is true of the _domain_
type and false of the _inbound twin_; the real distinction is the twin split:

```markdown
- **Secret exception.** _Amended by #822._ A secret's **domain** type
  (`Password`) has no serde bridge (ADR-0063), so it cannot itself be a wire arg
  — but its **inbound twin** (`ProfferedPassword`, ADR-0063's `Proffered`,
  generalized by ADR-0084) does have one and carries the wire role. So a secret
  **is** a typed wire arg like any other, validated at decode by the shared
  shape rule and client-pre-validated through the same `FromStr`; what stays
  special is the twin split, plus `skip`-ping the value out of tracing. (This
  bullet previously said a secret's arg "stays `String`" — untrue since #315
  typed all three password-taking fns.)
```

**Consequences (`:112-113`)** — the sentence "Args that stay `String` (secrets
like `password`) still parse in the body, so their rejection telemetry is
unchanged" is false in both halves. Replace with a statement that secrets decode
like every other typed arg, and that a decode failure is now observable (#822)
rather than silent — which also updates the preceding sentence's "skipping the
body's error-boundary telemetry and rejection metrics", since that is no longer
the whole story.

- [ ] **Step 2: Verify**

Run: `devtool run --cwd <worktree> -- cargo xtask check` Expected: PASS —
`adr-format`, `adr-readme-parity`, `doc-links` green. (`adr-format` constrains
only the heading and the bare `- Status:` token, so extra header lines are
fine.)

Then confirm AC8 — the pattern contains backticks, so run it from a fenced block
rather than an inline span:

```bash
rg 'stays?\s+`String`' docs/adr/0065-*.md
```

It must return nothing, matching both offending phrasings ("its arg **stays**
`String`" at `:77` and "Args that **stay** `String`" at `:112`).

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0065-client-side-domain-validation.md
git commit -m "docs(adr): amend ADR-0065 — secrets are typed wire args via the inbound twin (#822)"
```

---

## Verification

- [ ] `devtool run --cwd <worktree> -- cargo xtask validate` green (static +
      clippy + coverage + all four `{sqlite,postgres}×{chromium,firefox}` e2e
      combos).
- [x] AC8 — **the AC's literal pattern was too blunt; corrected during execution.**
      `rg 'stays?\s+`String`'` still matches twice, but both hits are the amendment
      _quoting_ the old wording in order to mark it wrong — the same style #568 used
      ("this bullet previously cited ADR-0056"). What AC8 means is that the ADR no
      longer **asserts** it, so the check is for the original claims:

      ```bash
      rg 'its arg stays `String`|Args that stay `String`' docs/adr/0065-*.md
      ```

      That returns nothing. Keeping the quotations is deliberate: an amendment that
      silently rewrites text leaves a future reader unable to tell what changed.

- [ ] `rg 'InternalError::validation\(' web/src/error.rs` returns nothing — the
      decode path must use `validation_source` (the major finding from the spec
      review).
