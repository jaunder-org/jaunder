# Plan — Retire `server_boundary`'s SSR-vestigial owner-pinning (#594)

Spec:
[`2026-07-22-issue-594-retire-server-boundary-owner-pinning.md`](../specs/2026-07-22-issue-594-retire-server-boundary-owner-pinning.md).
The spec is the "what/why" (the vestigiality determination + acceptance
criteria); this plan is the "how." Don't re-derive the analysis — see the spec.

## Review header

- **Goal:** Remove the dead owner-pinning from `web/src/error.rs`'s
  `server_boundary` (the #89/#124/#138 SSR mechanism), keep the error-projection
  half, and record the retirement in ADR-0016.
- **Scope (in):** `web/src/error.rs` (simplify `server_boundary`; delete
  `owner_ancestry_strong`; delete the `owner_lifetime` test module; fix stale
  doc comments) and `docs/adr/0016-dependency-injection-and-appstate.md`
  (retirement addendum).
- **Scope (out):** the `boundary!` macro and every server-fn call site
  (signature unchanged); `emit_boundary_failure`/`project`; `host`; #520 endgame
  work; no new regression test (spec Non-goals).
- **Tasks:**
  1. Atomically remove the owner-pinning from `web/src/error.rs`: simplify
     `server_boundary`, delete `owner_ancestry_strong`, delete the
     `owner_lifetime` module, fix stale doc comments. (AC1–AC5)
  2. Add the ADR-0016 retirement addendum. (AC6)
  3. Run the full gate and confirm a clean tree. (AC7)
- **Key risks/decisions:**
  - **Atomicity (task 1):** the two `server_boundary_*` tests assert context
    survives an owner drop; with pinning removed they would fail, so the code
    change and the `owner_lifetime` deletion are one commit — never split.
  - No separable concerns surfaced — no first "file issues" task needed.
  - `server_boundary` is `#[cfg(feature = "server")]`; verification must
    exercise the server-gated path (the default `cargo xtask check` covers
    server-gated web code via its instrumented test run; the pinning removal is
    host-side, not wasm-only).

- **For agentic workers:** execute with **`jaunder-iterate`** (delegate a task
  to a subagent via **`jaunder-dispatch`** if useful). This is a small,
  single-file code change plus a docs edit — inline execution is appropriate.

## Global Constraints

- Follow `CONTRIBUTING.md` (coverage policy, import discipline). No
  `Co-Authored-By` trailer on commits.
- Commit only after `cargo xtask check` is green (**`jaunder-commit`**); the
  pre-commit hook runs the full check.
- Review base is the fork point: `git diff wt-base-issue-594..HEAD` (or
  `main...HEAD`).

---

## Task 1 — Remove the owner-pinning from `web/src/error.rs` (AC1–AC5)

**Files:**

- `web/src/error.rs` — edit `server_boundary`; delete `owner_ancestry_strong`;
  delete the `owner_lifetime` module; fix stale doc comments.

**Step 1a — simplify `server_boundary`.** Replace the current body (the
`if let Some(owner) = Owner::current()` / `ScopedFuture` block and its ~30-line
#89/#138 comment) so it awaits the future once and projects the error. Also fix
the doc comment, which currently mislabels the error type as `ServerFnError`:

```rust
/// Awaits the given future, converting any `InternalError` to its public
/// `WebError` form. This is a thin error-projection boundary: it owns no leptos
/// reactive-owner lifetime concerns. (Owner-pinning against context loss across an
/// `.await` was removed in #594 — see the ADR-0016 retirement addendum; the sole
/// server-fn invocation path, leptos_axum's `/api` handler, holds the owner strong
/// for the whole future itself.)
///
/// # Errors
///
/// Returns `Err(WebError)` if the wrapped future returns an `InternalError`.
#[cfg(feature = "server")]
pub async fn server_boundary<T>(
    server_fn: &'static str,
    future: impl std::future::Future<Output = InternalResult<T>>,
) -> WebResult<T> {
    match future.await {
        Ok(value) => Ok(value),
        Err(error) => {
            // The carrier owns its own observability (structured log + metric);
            // `web` only performs the wire projection.
            error.emit_boundary_failure(server_fn);
            Err(project(error.kind(), error.public_message()))
        }
    }
}
```

**Step 1a′ — fix the file-header comment.** The module-header comment
(`error.rs:8-11`) says `web` "keeps only the wire type, the `kind → WebError`
projection, and the leptos owner-pinning boundary." Drop the "and the leptos
owner-pinning boundary" clause so the header no longer advertises a boundary
that no longer exists (AC5).

**Step 1b — delete `owner_ancestry_strong`.** Remove the entire
`#[cfg(feature = "server")] fn owner_ancestry_strong(…) -> Vec<…Owner> { … }`
and its doc comment (currently just above `server_boundary`). After 1a it has no
callers.

**Step 1c — delete the `owner_lifetime` test module.** Remove the whole
`#[cfg(test)] mod owner_lifetime { … }` block at the bottom of the file (module
doc + all six tests, incl. the four leptos-primitive tests — spec AC3). Leave
the earlier `#[cfg(test)] mod tests { … }` (projection/emit tests) untouched.

**Step 1d — confirm no dangling references.** `server_boundary` and the deleted
items were the only users of `leptos::reactive::owner::Owner` / `ScopedFuture`
in this file's non-test code; verify nothing else references them.

**Verify:**

```
rg -n 'ScopedFuture|owner_ancestry_strong|Owner::current|mod owner_lifetime' web/src/error.rs
```

→ expected: **no matches** (AC1, AC2, AC3).

```
cargo nextest run -p web --features server error::tests
```

→ expected: **PASS** — every retained projection/emit test green (AC4). (If the
crate's default features don't include `server`, run with `--all-features`; the
projection tests are `#[cfg(feature = "server")]`.)

```
cargo clippy -p web --all-features --all-targets -- -D warnings
```

→ expected: **PASS** — no dead-import / unused warnings from the removal.

**Commit** (after `cargo xtask check` is green — **jaunder-commit**):
`refactor(web): retire SSR-vestigial owner-pinning in server_boundary (#594)`

---

## Task 2 — ADR-0016 retirement addendum (AC6)

**Files:**

- `docs/adr/0016-dependency-injection-and-appstate.md` — append a new dated
  addendum after the existing #138 addendum.

Addenda to an existing ADR are edited in place (the draft/promote flow is for
_new_ ADRs only). Append a section in the same style as the #89/#124/#138
addenda:

```markdown
## Addendum (2026-07-22): owner-pinning retired — CSR-only makes it vestigial (#594)

The #89 → #124 → #138 addenda hardened `server_boundary` against a leptos
reactive owner being dropped while a `#[server]`-fn future was suspended at an
`.await`. Every load-bearing scenario was SSR-specific: #89's base concern is
_already_ covered on the `/api` path (leptos_axum establishes and holds the
owner), while #124 (SSR `Resource` fetcher) and #138 (page-render SSR root) only
arose under component SSR.

With no component SSR (#487) and `server_resource` removed (#515), the **sole**
server-fn invocation path is a browser `POST /api/{fn}` dispatched by
`leptos_axum::handle_server_fns_with_context`. There, leptos_axum creates one
parentless root `Owner`, runs `additional_context`
(`provide_app_state_contexts`) and the server-fn body inside it, and holds that
owner strong for the entire `.await` (a stack local plus leptos_axum's own
`ScopedFuture`). So `server_boundary`'s `owner_ancestry_strong` walked a root to
an **empty** ancestry and its inner `ScopedFuture` merely duplicated
leptos_axum's — dead weight.

**Resolution.** `server_boundary` no longer touches reactive-owner lifetime: it
awaits the body and projects `InternalError → WebError` (the error-projection
half is unrelated to SSR and is retained). `owner_ancestry_strong` and the
deterministic `owner_lifetime` tests are removed. **The #89, #124, and #138
addenda above are superseded and retained for history only** — their described
owner-pinning no longer exists, and their test citations (including #138's
`post_await_read_loses_ancestor_context_when_parent_owner_dropped`, already
stale) should not be treated as live.
```

**Verify:**

- `prettier --check docs/adr/0016-dependency-injection-and-appstate.md` (the
  pre-commit hook runs prettier; run `prettier -w` on it before staging so
  formatting matches — memory: precommit prettier restages prose).
- No ADR README table change is needed (adding an addendum doesn't change the
  ADR's title/status row).

**Commit:**
`docs(adr): record #594 retirement of server_boundary owner-pinning (ADR-0016)`

---

## Task 3 — Full gate + clean-tree check (AC7)

**Verify:**

```
cargo xtask validate --no-e2e
```

→ expected: **PASS** (static + clippy + coverage). Confirms no dead imports, no
clippy fallout, and no coverage regression from the removed `owner_lifetime`
tests (the remaining `server_boundary`/`project` tests still cover the surviving
branches; the simplified `server_boundary` has fewer branches than before, so no
new uncovered regions).

Then confirm the tree is clean (fmt auto-fixes may have restaged — memory):

```
git status --porcelain
```

→ expected: empty (both commits landed, nothing uncommitted).

No further tasks — the work is two commits (code, docs) behind the fork-point
tag `wt-base-issue-594`. Ship via **jaunder-ship**.

## Self-review

- Every spec AC maps to a task: AC1/AC5 → 1a; AC2 → 1b; AC3 → 1c; AC4 → 1's
  nextest; AC6 → 2; AC7 → 3. AC "no new regression test" is honored (no test
  task).
- Tasks are independently verifiable (each has an explicit expected result) and
  small.
- No task smuggles out-of-scope work; no placeholders.
