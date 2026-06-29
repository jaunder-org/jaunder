# Issue #138 — backup SSR storage-context panic — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the backup `#[server]` fns from panicking on `expect_context::<Arc<dyn UserStorage>>()` under authenticated SSR, and statically prevent the recurring post-await-reactive-read bug class.

**Architecture:** Reproduction-first. Build a deterministic red test that reproduces the panic; root-cause it; empirically classify candidate fixes (hoist / axum-Extension / helper-class); apply the chosen working fix; statically reject the non-working pattern(s); lock it with a regression test and an ADR addendum.

**Tech Stack:** Rust, Leptos (`reactive_graph` owner/context), axum, `leptos_axum`, nextest.

> **Outcome (2026-06-28) — pivoted to the helper fix (C3).** Reproduction confirmed
> the bug is *ancestor*-owner context loss across an SSR await. Rather than C1 (hoist
> the 5 backup reads) + a static lint (Tasks 4–5 as originally drafted), the fix lives
> in `server_boundary`: it holds the full owner ancestry strong via
> `owner_ancestry_strong()` (feasible because `Owner::parent()` upgrades the weak
> parent link). This *eliminates* the post-await-read class for every `#[server]` fn,
> so the hoist and the lint are dropped — no non-working pattern remains to reject.
> Tasks below are kept as the executed record; Task 4 = the helper fix, Task 5 = N/A
> (superseded by the structural fix), Task 6 = ADR + full validate.

## Global Constraints

- Backend parity, coverage policy, DI/ADR-0016, and the verify ladder per `CONTRIBUTING.md` apply to every task.
- Per-task gate: `cargo xtask check --no-test` (clippy + fmt). Final gate: `cargo xtask validate`.
- No `Co-Authored-By` trailers. No commits without the user's go (already given for this cycle's loop); the merge is a halt point.
- Raw `Resource::new`/`create_resource` is clippy-banned in `web/src`; use `crate::server_resource` (#124).
- Do not migrate the ~75 working **pre-await** reactive reads — they are correct.
- Reproduction is the gate: no fix is committed until a test is **red on `main`**.

---

### Task 1: Deterministically reproduce the panic (red test)

**Files:**
- Modify/Test: `web/src/error.rs` (the `#[cfg(test)] mod owner_lifetime` block, ~line 910)

**Interfaces:**
- Consumes: `crate::error::scoped_fetcher_future` (the `server_resource` fetcher wrapper), the module's existing `Marker`, `YieldOnce`, `step` helpers, `leptos::reactive::owner::Owner`, `leptos::prelude::{provide_context, use_context}`.
- Produces: a test named `post_await_read_loses_ancestor_context_when_parent_owner_dropped` that is **red on `main`** and documents the mechanism.

**Hypothesis under test:** storage context is provided in an **ancestor** owner (root `provide_app_state_contexts`), but `scoped_fetcher_future`/`server_boundary` only hold a strong ref to the **captured (child) owner**. A post-await `use_context` walks the ancestry; if the ancestor's strong ref is dropped during the SSR await, the walk returns `None`. Pre-await reads resolve before the drop, which is why the ~75 pre-await sites (e.g. `posts/listing.rs:60`) work.

- [ ] **Step 1: Write the failing test** — append to `mod owner_lifetime`:

```rust
/// #138: the storage contexts (`UserStorage`/`SiteConfigStorage`) are provided in
/// an *ancestor* owner (the root `provide_app_state_contexts`), while
/// `scoped_fetcher_future`/`server_boundary` hold a strong ref only to the
/// *captured child* owner (the resource's own owner). A post-await `use_context`
/// walks the ancestry; if the ancestor's strong ref is dropped during the SSR
/// await, the walk fails — reproducing the backup-fn panic. A pre-await read
/// resolves before the drop, which is why the ~75 pre-await sites do not panic.
#[test]
fn post_await_read_loses_ancestor_context_when_parent_owner_dropped() {
    let parent = Owner::new();
    parent.set();
    provide_context(Marker(7)); // provided in the ANCESTOR (like the root provide)

    let child = Owner::new(); // parent = current = `parent`
    child.set(); // resource's own owner is the captured one

    // Build the fetcher future exactly as `server_resource` does: it captures the
    // currently-set owner (`child`) via `Owner::current().unwrap_or_default()`.
    let mut fut = Box::pin(crate::error::scoped_fetcher_future(async {
        let pre = use_context::<Marker>();
        YieldOnce(false).await;
        let post = use_context::<Marker>();
        (pre, post)
    }));

    let mut cx = Context::from_waker(Waker::noop());
    assert!(step(fut.as_mut().poll(&mut cx)).is_none()); // first poll: reads `pre`, suspends
    drop(parent); // SSR drops the ancestor while the resource future is suspended
    drop(child); // only `ScopedFuture`'s captured strong ref keeps the child alive

    let (pre, post) =
        step(fut.as_mut().poll(&mut cx)).expect("future did not complete on second poll");
    assert_eq!(pre, Some(Marker(7)), "ancestor context resolvable before the drop");
    assert_eq!(
        post, None,
        "#138: post-await read loses ancestor context once the ancestor owner is dropped"
    );
}
```

- [ ] **Step 2: Run it and confirm it reproduces the mechanism**

Run: `cargo nextest run -p web owner_lifetime::post_await_read_loses_ancestor_context_when_parent_owner_dropped`
Expected: **PASS** if the hypothesis holds (the assert encodes the *buggy* behavior `post == None`). This is the deterministic capture of the bug.

- [ ] **Step 3: Decision gate — did the model capture the real bug?**
  - If `post == None` (test green as written): the mechanism is confirmed — **the captured-owner-only strong ref does not cover ancestor context**. Record this in the test doc-comment and proceed to Task 2.
  - If `post == Some(Marker(7))` (assert fails): parent links are strong / model insufficient. Investigate variants in order, re-running each: (a) capture the child while parent is *not* current (empty-owner capture, `error.rs:415`); (b) drop only `parent` (keep `child`); (c) provide in `parent`, set `child`, but build via the full `server_resource` + `server_boundary` stack rather than `scoped_fetcher_future` alone. If no unit arrangement reproduces, **escalate to Task 1b.**

- [ ] **Step 4: Commit**

```bash
git add web/src/error.rs
git commit -m "test(web): deterministically reproduce #138 backup SSR ancestor-context loss"
```

---

### Task 1b (conditional): SSR-level integration reproduction

Only if Task 1's unit harness cannot reproduce after the Step-3 variants.

**Files:**
- Test: `server/src/lib.rs` `#[cfg(test)] mod tests` (the existing axum/`tower` request harness, ~line 128) or a new `server/tests/backup_ssr.rs`.

**Interfaces:**
- Consumes: the existing server test harness that builds the router and issues a `Request` (see `server/src/lib.rs:128+`).
- Produces: a test that issues an **authenticated** GET `/` and asserts a 200 with the rendered operator widget (no panic / no 500 from the boundary).

- [ ] **Step 1: Write the failing integration test** — build the app router with a seeded operator session, issue an authenticated `GET /`, assert `StatusCode::OK` and that the body contains the backup-banner/operator marker. Expected on `main`: panic-induced 500 or missing marker.
- [ ] **Step 2: Run it** — `cargo nextest run -p server backup_ssr`. Expected: FAIL on `main`.
- [ ] **Step 3: Commit** — `git commit -m "test(server): reproduce #138 backup panic via authenticated SSR render"`.

---

### Task 2: Root-cause and document the mechanism

**Files:**
- Modify: the test doc-comment from Task 1/1b (no new code).

**Interfaces:**
- Consumes: the red/green test from Task 1.
- Produces: a one-paragraph mechanism statement (in the doc-comment) answering: where does the lost context live (ancestor vs captured owner), and what exactly drops it (ancestor strong-ref drop during SSR await). This drives Task 3 classification.

- [ ] **Step 1:** From the confirmed test, write the mechanism into the doc-comment (ancestor-owner context + captured-owner-only strong ref ⇒ post-await ancestry walk fails). No separate commit; folded into Task 3's first commit.

---

### Task 3: Empirically classify each candidate fix

For each candidate, apply it on top of the red repro and re-run the repro + the existing `owner_lifetime` suite. Record working / non-working **by the test**. Use a scratch branch or stash between candidates so each is measured in isolation; do **not** commit non-working candidates.

**Candidate experiments (run each, note the result):**

- [ ] **C1 — Hoist:** in `web/src/backup/mod.rs`, move the `expect_context::<Arc<dyn UserStorage>>()` / `SiteConfigStorage` reads **above** `require_auth().await` (mirror `posts/listing.rs:60`). In the unit harness, model this as reading `pre` before the await and using it post-await. Re-run repro. Working iff the pre-await value survives.
- [ ] **C2 — A (axum Extension):** add `.layer(axum::Extension(users_ext))` / `site_config_ext` in `server/src/lib.rs` (next to `sessions_ext` at `:124`), and have the backup fns read `Arc<dyn UserStorage>`/`Arc<dyn SiteConfigStorage>` from `Parts.extensions` (mirror `auth/server.rs:64-67`). Re-run repro. Owner-independent ⇒ expected working.
- [ ] **C3 — B (helper-class fix):** modify `server_boundary`/`scoped_fetcher_future` to hold a strong ref across the **full owner ancestry** (not just the captured owner), then re-run the repro and the entire `owner_lifetime` suite plus `cargo xtask check --no-test`. Working iff repro passes with no regression in the suite.

- [ ] **Step: Record the classification table** in the plan/spec working notes (working vs non-working, with the failing assertion for non-working ones).

---

### Task 4: Choose the fix, report all working options, apply it

**Files:**
- Modify: depends on choice — `web/src/backup/mod.rs` (C1), or `server/src/lib.rs` + `web/src/backup/mod.rs` + possibly `web/src/backup/server.rs` (C2), or `web/src/error.rs` (C3).

**Interfaces:**
- Consumes: Task 3 classification.
- Produces: the repro test passing with the *correct* (fixed) behavior, and a working-options matrix for plan/PR.

- [ ] **Step 1: Build the working-options matrix** (every option Task 3 proved working):

| Option | Boilerplate repetition | Substantial downsides |
|---|---|---|
| C1 hoist | per-fn: 0 new lines, just ordering | relies on the very read-ordering discipline that failed for months; guarded only by Task 5's static check |
| C2 Extension | per storage trait: 1 layer + per-fn `Parts.extensions` read | adds an axum layer per trait; two DI paths (context + extension) for storage |
| C3 helper fix | none per-fn | broad blast radius — touches the SSR path for all ~75 reactive reads; highest review risk |

Present to the user **at plan approval** (this plan) and again in the PR. Recommendation: prefer the most robust **working** option that does not rely on per-fn discipline (C2 or C3) **unless** C1 + the Task 5 static guard together give equivalent safety at far lower cost — decide from the real matrix.

- [ ] **Step 2: Update the repro test's expected post-await value to the fixed behavior** (`post == Some(Marker(7))` / widget renders), so it now asserts the fix.
- [ ] **Step 3: Apply the chosen fix** (code per the chosen candidate from Task 3).
- [ ] **Step 4: Run** `cargo nextest run -p web owner_lifetime` (and `-p server backup_ssr` if Task 1b). Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "fix(web): <chosen-fix> so backup #[server] fns survive authenticated SSR (#138)"`.

---

### Task 5: Statically reject the non-working pattern(s)

**Files:**
- Create: an ast-grep rule file under the repo's ast-grep rules dir (discover exact location, e.g. `sgconfig.yml` + `rules/`); or an xtask check module.
- Modify: `xtask/src/` static-check pass to run the rule; `CONTRIBUTING.md` if a new check is documented there.

**Interfaces:**
- Consumes: the non-working pattern(s) from Task 3 (at minimum: a post-await reactive-context **storage** read inside a `#[server]` fn).
- Produces: a static check wired into `cargo xtask check` that fails on the broken pattern with a file:line message, **with zero false positives** on the current tree.

Constraints (user direction): worth real effort (this class has recurred for months), but **no false positives**, and don't move heaven and earth. Cheapest-sound-first.

- [ ] **Step 1: Discover the static-check surface** — locate how `cargo xtask check`'s static pass runs (ast-grep config, existing rules, the clippy `disallowed-methods` ban from #124). Run: `rg -n "ast-grep|sgconfig|disallowed-methods|static" xtask/src CONTRIBUTING.md`.
- [ ] **Step 2: Write the rule (TDD) — failing fixture first.** Add a fixture (a doc-test or a rule test case) containing a `#[server]` fn that does `expect_context::<Arc<dyn FooStorage>>()` **after** an `.await`. Assert the rule flags it.
- [ ] **Step 3: Implement the ast-grep relational rule** — match `expect_context`/`use_context` whose type arg is an `Arc<dyn *Storage>` and that `follows` an `await` expression within the same `#[server]`-attributed `fn` body. If the relational form can't express the ordering soundly, fall back to a syn-based xtask walker over `web/src/**` `#[server]` fns.
- [ ] **Step 4: Verify zero false positives** — run the rule across `web/src`; confirm it flags only the (now-fixed) #138 sites in a deliberately-broken fixture and **nothing** in the real tree (the ~75 pre-await sites must not trip). Run: `cargo xtask check --no-test`.
- [ ] **Step 5: Fallback decision** — if a sound, false-positive-free check is infeasible, do **not** ship a flaky lint: document the tradeoff in the spec/PR and rely on the Task 1 repro test as the guard. Record the decision explicitly.
- [ ] **Step 6: Commit** — `git commit -m "test(xtask): statically reject post-await reactive storage reads in #[server] fns (#138)"`.

---

### Task 6: Regression guard, ADR addendum, full gate

**Files:**
- Modify: `docs/adr/0016-dependency-injection-and-appstate.md` (new addendum); `docs/README.md` (ADR table row if status/title changes).
- Verify: the repro test is the permanent guard.

**Interfaces:**
- Consumes: the chosen fix and static check.
- Produces: documented decision + green full gate.

- [ ] **Step 1: Write the ADR-0016 addendum** — record the pre-vs-post-await reactive-read rule, why ancestor-owned context is lost across SSR awaits, the chosen fix, and the static guard. Update the `docs/README.md` ADR row if needed.
- [ ] **Step 2: Run the full local gate** — `cargo xtask validate`. Expected: green (static + coverage + e2e zero-panic gate for backup paths passes). Watch for the coverage baseline; reanchor via `cargo xtask coverage reanchor` only if legitimately shifted.
- [ ] **Step 3: Commit** — `git commit -m "docs(adr): record SSR pre/post-await reactive-read rule and #138 fix"`.

---

## Self-Review

- **Spec coverage:** Phase 1 → Task 1/1b; Phase 2 → Task 2; Phase 3 → Task 3; Phase 4 (report all working options + matrix) → Task 4; Phase 5 (static rejection) → Task 5; Phase 6 (regression + ADR) → Task 6. Acceptance criteria: panic-free authenticated SSR + e2e gate → Task 6 Step 2; regression test → Task 1/4; static rejection → Task 5; `validate` green → Task 6.
- **Placeholders:** none — the only deferred content is the Task-4 fix code, which is intentionally gated on Task 3's empirical result; each candidate's code location and pattern is specified.
- **Type consistency:** helpers (`Marker`, `YieldOnce`, `step`) and `scoped_fetcher_future` match `web/src/error.rs`; `Parts.extensions` read mirrors `web/src/auth/server.rs:64-67`; Extension layering mirrors `server/src/lib.rs:124`.
