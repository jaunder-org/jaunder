# Match `#[server]` fns by vertical, drop vestigial vertical nouns — Implementation Plan (#684)

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating individual tasks to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-29-issue-684-server-fn-path-matching.md`](../specs/2026-07-29-issue-684-server-fn-path-matching.md)
— the "what/why". This plan is the "how" and does not restate it.

**Goal:** Re-key the ADR-0066 registrar gate from leaf name to
`(vertical, leaf)` so the vestigial vertical nouns can be dropped from all 55
`#[server]` fn idents, and re-namespace the wire to `/api/<vertical>/<op>`.

**Architecture:** Three xtask gates share one enumerator
(`xtask/src/web_server_fns.rs`). This work promotes `vertical_of` and the
attribute-rewrite machinery into that shared module, re-keys the registrar gate,
adds a third gate that owns `endpoint` as a derived literal, and then performs
the renames — with the two derived literals (span name, endpoint) written by
`Mode::Fix` rather than by hand.

**Tech Stack:** Rust, `syn` (gate parsing), `cargo xtask` (gate driver),
`cargo nextest` (tests), Playwright/TypeScript (e2e).

---

## Review header

**Scope — in:**

- Re-key `server-fn-registrar` to `(vertical, leaf)`; narrow the duplicate check
  to per-vertical.
- New `server-fn-endpoint` gate with `Mode::Fix`/`Mode::Check`.
- Promote `vertical_of` + rewrite machinery into `web_server_fns`.
- Rename 42 `#[server]` fn idents and 12 hand-written wire DTOs.
- Re-namespace 55 endpoints to `/<vertical>/<op>`; convert 228 `server/tests`
  literals to `::PATH`; update 15 e2e sites.
- Amend ADR-0066; draft a new ADR for the wire namespace.

**Scope — out:** the `/api` prefix itself; AtomPub; `auth::AuthUser` /
`auth::AuthRejection` / `tags::TagInputState`; a TypeScript-side endpoint guard
(filed as a follow-up in Task 1).

**Tasks:**

- **Task 1** — File the three separable follow-up issues.
- **Task 2** — Promote `vertical_of` + rewrite machinery into `web_server_fns`
  (no behavior change).
- **Task 3** — Re-key the registrar gate to `(vertical, leaf)`; narrow the
  duplicate check.
- **Task 4** — Convert 228 `server/tests` URL literals to
  `<T as ServerFn>::PATH`.
- **Task 5** — Rename the 42 `#[server]` fn idents (+ their `boundary!` labels,
  registrar leaves, live-doc references).
- **Task 6** — Rename the 12 wire DTOs.
- **Task 7a** — Add the `server-fn-endpoint` gate; flip 55 endpoints; add the
  router regression test. (Rust only.)
- **Task 7b** — Follow the wire move through e2e and the URL docs.
- **Task 8** — Amend ADR-0066; draft the wire-namespace ADR.

**Key risks / decisions:**

- **Task 3 must not delete the duplicate check** — spec §"Why the duplicate
  check cannot simply be deleted". Two same-ident `#[server]` fns in one
  vertical compile cleanly via glob shadowing (`rustc`-verified); deleting the
  check reopens #358.
- **`boundary!("…")` is a third derived literal, and it is unguarded.** All 55
  `#[server]` bodies pass their fn ident to `boundary!`, which becomes the
  ADR-0011 structured-log/metric field naming the failing server fn. Neither the
  compiler nor any gate correlates it with the ident, so Task 5 must rename the
  42 labels by hand and Task 5 Step 1b is the only thing that catches a miss. A
  gate is filed as a follow-up (Task 1 Step 3).
- **Ordering is load-bearing.** Every commit runs the full `cargo xtask check`,
  so each task must leave the tree green. Idents are renamed (Task 5) _before_
  endpoints move (Task 7a) so the e2e suite is touched exactly once.
- **Task 4 precedes Task 7a** so the 228 Rust literals are already `::PATH` when
  the wire flips — otherwise 7a breaks every server integration test.
- **e2e is red between 7a and 7b, deliberately.** `cargo xtask check` does not
  run e2e, so 7a is green by the per-commit gate while the Playwright literals
  still name the old URLs. 7b closes it. Do not run `e2e-local` at the end of
  7a.
- **`storage/src` is never touched** — `update_post`, `confirm_password_reset`,
  and `create_user_with_invite` also name storage fns. See Global Constraints.
- **D10 (diff discipline)** applies to every task: edit a comment only where the
  rename made it factually wrong.

---

## Global Constraints

Copied verbatim from the spec; every task's requirements implicitly include
these.

- **D10 — diff discipline.** Comments and doc comments are edited **only** where
  the rename makes them factually incorrect (a stale ident, a stale URL, a stale
  claim). No opportunistic rewording, no reflowing, no "while I'm here"
  improvements.
- **Vertical** = the first path segment under `web/src`.
- **Endpoint form** = `/<vertical>/<ident>`; full URL `/api/<vertical>/<ident>`.
- **Registrar entry form** = `web::<vertical>::<Leaf>`, exactly three segments.
- **Historical docs are not edited:** everything under `docs/archive/` and
  `docs/superpowers/` — **except this cycle's own spec and plan**, which are
  working documents until `jaunder-ship` archives them. The rule exists to stop
  us rewriting _other_ cycles' records.
- **The rename is scoped to `web/src` and its callers — never `storage/src`.**
  Three idents on the rename table also name unrelated storage-layer fns:
  `update_post` (`storage/src/posts.rs:572,773,1004` + the dialect files),
  `confirm_password_reset` (`storage/src/{sqlite,postgres}/mod.rs`,
  `atomic.rs`), and `create_user_with_invite`. A bare `rg -w <ident>` sweep will
  hit them. Renaming a storage fn is out of scope and would be a silent scope
  violation.
- **Commits:** one clean commit per task. Run `cargo xtask check` first so the
  pre-commit gate passes clean (**jaunder-commit**). **No `Co-Authored-By`
  trailer.**
- **Gate invocation:** `devtool run -- cargo xtask check` (worktree-aware;
  honest exit code).
- **Integration tests need a database.** Use
  `devtool pg run -- cargo nextest run -p jaunder --test integration` — it wraps
  the command in a throwaway PostgreSQL 16 cluster. A **bare** nextest run has
  no PostgreSQL listening, so every `*_postgres` case fails `ConnectionRefused`
  and the run cancels after ~16 of 1061 tests: a false red that never reaches
  the changed code. (`devtool pg`, not `cargo xtask pg`.)

---

### Task 1: File the separable follow-up issues

None belongs to #684; all three were surfaced by its investigation and its
reviews. File them now so they can be picked up concurrently
(**jaunder-issues**).

**Files:** none (tracker only).

**Interfaces:**

- Produces: three issue numbers, referenced in Task 8's ADR draft (TS guard) and
  in the spec's Out-of-scope list.

**Outcome:** filed as **#712** (e2e drift guard, `test-infra`+`dx`), **#713**
(non-wire types, `web`), **#714** (`boundary!` gate, `observability`+`tooling`)
— all three added to Jaunder Backlog (#1). Note the repo has no `e2e` or `xtask`
label; the nearest real topic labels were used.

- [x] **Step 1: File the TypeScript endpoint-drift guard issue**

Title:
`end2end: no guard ties Playwright's /api/… literals to the server-fn endpoints`

Body must state: after #684, `endpoint` is a gate-owned derived literal that
`cargo xtask check` rewrites under `Mode::Fix`, but the 15 `end2end/tests/**`
sites remain hardcoded strings with no gate — the only detector of drift is a
red e2e matrix. Reference `end2end/tests/helpers.ts:105-110` (`failServerFn`
takes the endpoint as a bare string) and the spec's risk table. Label `e2e`.

- [x] **Step 2: File the non-wire vertical-noun types issue**

Title: `web: three non-wire types still carry their vertical's noun`

Body must list `auth::AuthUser` (`web/src/auth/server.rs:26`),
`auth::AuthRejection` (`web/src/auth/server.rs:33`), `tags::TagInputState`
(`web/src/tags/input_state.rs:24`), and state that #684 drew its scope line at
"appears in a `#[server]` fn signature", which these do not. Label `web`.

- [x] **Step 3: File the `boundary!` label gate issue**

Title: `xtask: no gate ties boundary!("…") labels to their server-fn idents`

Body must state: `web/src/lib.rs:15-19`'s `boundary!($name, $body)` forwards
`$name` to `error::server_boundary(server_fn: &'static str, …)`
(`web/src/error.rs:115-127`), which emits it as the structured-log/metric field
naming the failing server fn (ADR-0011). All 55 call sites pass the fn ident
verbatim, and **nothing enforces the correspondence** — not the compiler, not a
gate, not a test. #684 renames the 42 labels by hand (Task 5); this issue is to
make them a gate-enforced derived literal like the span name and endpoint, so
they cannot drift again. Note that unlike those two, the label is a macro
argument in the fn _body_, so it needs `syn` body traversal rather than the
shared attribute-rewrite machinery. Label `xtask`.

- [x] **Step 4: Record the issue numbers in the spec**

Add the three numbers to the spec's Out-of-scope bullets (one clause each, e.g.
"— filed as #NNN"), and add a bullet for the `boundary!` gate. Permitted by the
Global Constraints carve-out for this cycle's own spec; a factual addition, not
a reword, so D10-compliant.

- [x] **Step 5: Commit** — `a547f4f0`

```bash
git add docs/superpowers/specs/2026-07-29-issue-684-server-fn-path-matching.md
git commit -m "docs(spec): link #684's separable concerns to their issues"
```

---

### Task 2: Promote `vertical_of` and the rewrite machinery into `web_server_fns`

Pure refactor — no gate changes behavior. Three gates will need `vertical_of`;
two will need attribute-literal rewriting, and the existing implementation is
spelled for `name` only.

**Files:**

- Modify: `xtask/src/web_server_fns.rs` (add `vertical_of`, `LineFix`,
  `apply_fixes`, `rewrite_attr_arg`)
- Modify: `xtask/src/steps/server_fn_tracing_check.rs:135-151` (delete
  `vertical_of`), `:428-505` (delete `rewrite_name`/`find_name_eq`), `:507-562`
  (delete `LineFix`/`apply_fixes`), and their call sites at `:516-547`

**Interfaces:**

- Produces, from `crate::web_server_fns`:
  - `pub fn vertical_of(path: &str) -> Result<&str, String>` — moved verbatim,
    but the error message drops "span name" (now shared by three gates):
    `"{path}: a #[server] fn directly under {WEB_SRC} has no vertical directory — move it into {WEB_SRC}/<vertical>/"`.
  - `pub type LineFix = (usize, usize, String);`
  - `pub fn apply_fixes(src: &str, fixes: Vec<LineFix>) -> String` — moved
    verbatim.
  - `pub fn rewrite_attr_arg(attr_src: &str, attr_name: &str, key: &str, desired: &str, insert_if_absent: bool) -> Option<String>`
    — generalized from `rewrite_name`. **Three** parameters are added, not two:
    `key` replaces the hardcoded `"name"`, `insert_if_absent` controls whether a
    missing argument is synthesized, and `attr_name` replaces the hardcoded
    `"instrument"` at `server_fn_tracing_check.rs:455`
    (`attr_src.find("instrument")`). Without `attr_name` the shared helper would
    carry an invisible attribute-name assumption behind a key-general signature
    — inert today only because Task 7 passes `insert_if_absent = false` and
    returns before that line. Callers:
    `rewrite_attr_arg(&current, "instrument", "name", &desired, true)` (tracing)
    and `rewrite_attr_arg(&current, "server", "endpoint", &desired, false)`
    (Task 7).
  - `pub fn find_attr_arg_eq(attr_src: &str, key: &str) -> Option<usize>` —
    generalized from `find_name_eq`. The `fields(` guard at `:497` stays
    hardcoded: it exists so a `fields(name = …)` entry is not mistaken for the
    span name, and `#[server(...)]` has no nested-parenthesised argument, so it
    is inert for `endpoint`. Document that limitation on the fn — a future
    attribute with its own nested list would need it parameterized.

- [x] **Step 1: Move the four items, generalizing two**

Cut `vertical_of`, `rewrite_name`, `find_name_eq`, `LineFix`, `apply_fixes` from
`server_fn_tracing_check.rs` into `web_server_fns.rs` and make them `pub`. In
the moved code, replace the literal `"name"` in `rewrite_name`/`find_name_eq`
with the `key` parameter, and gate the insert branches on `insert_if_absent`.

At the tracing gate's call sites, `rewrite_name(&current, &desired)` becomes:

```rust
web_server_fns::rewrite_attr_arg(&current, "instrument", "name", &desired, true)
```

and `vertical_of(path)` / `apply_fixes(...)` / `LineFix` become
`web_server_fns::`-qualified. Per **memory: import discipline**, import enough
that call sites are not littered with long paths — add the needed names to the
existing `use crate::web_server_fns::{self, WEB_SRC};` line.

- [x] **Step 2: Move only the cleanly-movable tests; leave the rest in place**

**Only the six `rewrite_name` tests (`server_fn_tracing_check.rs:886-943`)
move.** Relocate them into `web_server_fns.rs`'s test module, updating each call
to pass `"instrument", "name"` explicitly.

Do **not** attempt to move any `vertical_of` or `apply_fixes` test:

- There is no standalone `vertical_of` test. The vertical behavior is covered
  _indirectly_ by `derives_the_vertical_from_the_first_segment_not_the_file`
  (`:707`), `a_server_fn_directly_under_web_src_is_a_hard_error` (`:717`), and
  `nothing_is_rewritten_for_a_file_with_no_vertical_directory` (`:983`) — all of
  which call the tracing gate's `problems`/`name_fixes`. They must **stay** in
  the tracing gate or it loses that coverage.
- Both `apply_fixes` tests (`:946`, `:989`) call `name_fixes` first, which stays
  behind. Moving them would not compile.

Then add, in `web_server_fns.rs`:

```rust
#[test]
fn rewrite_attr_arg_replaces_an_arbitrary_key_on_an_arbitrary_attribute() {
    let attr = "#[server(endpoint = \"/create_post\", input = Json)]";
    let got = rewrite_attr_arg(attr, "server", "endpoint", "/posts/create", false).unwrap();
    assert_eq!(
        got,
        "#[server(endpoint = \"/posts/create\", input = Json)]",
        "other arguments must survive verbatim"
    );
}

#[test]
fn rewrite_attr_arg_leaves_a_missing_key_alone_when_not_inserting() {
    // The endpoint gate treats a missing `endpoint` as a hard error, not something
    // to synthesize — so it must not be inserted behind the author's back.
    let attr = "#[server]";
    assert_eq!(rewrite_attr_arg(attr, "server", "endpoint", "/tags/list", false), None);
}

#[test]
fn rewrite_attr_arg_inserts_into_the_named_attribute_not_a_hardcoded_one() {
    // Pins the `attr_name` parameter: the insert branch must not assume "instrument".
    let attr = "#[server(input = Json)]";
    let got = rewrite_attr_arg(attr, "server", "endpoint", "/tags/list", true).unwrap();
    assert_eq!(got, "#[server(endpoint = \"/tags/list\", input = Json)]");
}

#[test]
fn vertical_of_takes_the_first_segment_under_web_src() {
    assert_eq!(vertical_of("web/src/posts/api/listing.rs").unwrap(), "posts");
    let err = vertical_of("web/src/loose.rs").unwrap_err();
    assert!(err.contains("web/src/loose.rs"), "{err}");
    assert!(err.contains("vertical"), "{err}");
}
```

The `vertical_of` test is **new**, not moved — it gives the now-shared fn direct
coverage that does not depend on any one gate.

- [x] **Step 3: Run the xtask tests, verify they pass** — 337 passed

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS — this task is behavior-preserving, so every pre-existing
tracing-gate test must still pass unchanged.

- [x] **Step 4: Run the gate, verify no tree mutation, verify the old symbol is
      gone**

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS, and
`git status --porcelain` shows only the two xtask files — the tracing gate's
`Mode::Fix` must not rewrite any span, because nothing about the derived names
changed.

Run: `rg 'fn rewrite_name'` Expected: no output (spec AC14 — a leftover
`rewrite_name` wrapper delegating to `rewrite_attr_arg` would pass the test
suite but fail the AC).

- [x] **Step 5: Commit** — `243bf577`

```bash
git add xtask/src/web_server_fns.rs xtask/src/steps/server_fn_tracing_check.rs
git commit -m "refactor(xtask): share vertical_of and attribute-literal rewriting across server-fn gates (#684)"
```

---

### Task 3: Re-key the registrar gate to `(vertical, leaf)`

**Files:**

- Modify: `xtask/src/steps/server_fn_registrar_check.rs` — module doc `:22-28`,
  `ServerFn` `:48-54`, `registered_names` `:125-175`, `problems` `:177-232`,
  tests `:273-433`

**Interfaces:**

- Consumes: `web_server_fns::vertical_of` (Task 2).
- Produces (private to this module, but named here because the tests below use
  them):
  - `struct ServerFn { name: String, line: usize }` — unchanged; the vertical is
    derived from the source _path_ in `problems`, not stored per-fn.
  - `fn registered_entries(registrar_src: &str) -> (BTreeSet<(String, String)>, Vec<String>)`
    — replaces `registered_names`. Returns `(vertical, leaf)` pairs plus one
    message per malformed entry.
  - `fn register_explicit_entry(path: &syn::Path) -> Option<Result<(String, String), String>>`
    — replaces `register_explicit_leaf`. `None` when the path is not a
    `register_explicit` turbofish; `Err` when the turbofish type is not exactly
    `web::<vertical>::<Leaf>`.

- [x] **Step 1: Write the failing tests**

Add to the module's `#[cfg(test)] mod tests`. Note the existing helper
`wrap_reg` is reused, and existing tests using bare `web/src/a.rs`-style paths
must be repathed to `web/src/<vertical>/api.rs` (Step 4).

```rust
#[test]
fn same_leaf_in_two_verticals_both_registered_is_fine() {
    // The change that unblocks #684: `Create` in posts and audiences are
    // distinct keys, not a collision.
    let sources = vec![
        (
            "web/src/posts/api.rs".to_string(),
            "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
        ),
        (
            "web/src/audiences/api.rs".to_string(),
            "#[server(endpoint = \"/audiences/create\")]\npub async fn create() {}\n".to_string(),
        ),
    ];
    let registrar = wrap_reg(
        "server_fn::axum::register_explicit::<web::posts::Create>();\n\
         server_fn::axum::register_explicit::<web::audiences::Create>();",
    );
    assert_eq!(problems(&sources, &registrar), None);
}

#[test]
fn the_unregistered_half_of_a_cross_vertical_pair_is_named_with_its_vertical() {
    let sources = vec![
        (
            "web/src/posts/api.rs".to_string(),
            "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
        ),
        (
            "web/src/audiences/api.rs".to_string(),
            "#[server(endpoint = \"/audiences/create\")]\npub async fn create() {}\n".to_string(),
        ),
    ];
    let registrar = wrap_reg("server_fn::axum::register_explicit::<web::audiences::Create>();");
    let detail = problems(&sources, &registrar).expect("posts::Create is unregistered");
    assert!(detail.contains("web/src/posts/api.rs"), "{detail}");
    assert!(detail.contains("posts"), "the vertical disambiguates the pair: {detail}");
    assert!(!detail.contains("web/src/audiences/api.rs"), "{detail}");
}

#[test]
fn a_duplicate_ident_within_one_vertical_fails_even_when_registered() {
    // Glob shadowing (web/src/posts/api.rs:16 `pub use listing::*;`) makes this
    // COMPILE — verified with rustc. Both fns collapse to (posts, Create), one
    // registrar entry satisfies both, and the unregistered one silently 404s.
    // This is the #358 hole; the gate is the only thing that catches it.
    let sources = vec![
        (
            "web/src/posts/api.rs".to_string(),
            "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
        ),
        (
            "web/src/posts/api/listing.rs".to_string(),
            "#[server(endpoint = \"/posts/create_other\")]\npub async fn create() {}\n".to_string(),
        ),
    ];
    let registrar = wrap_reg("server_fn::axum::register_explicit::<web::posts::Create>();");
    let detail = problems(&sources, &registrar).expect("a within-vertical duplicate is a problem");
    assert!(detail.contains("duplicate"), "{detail}");
    assert!(detail.contains("posts"), "{detail}");
    assert!(detail.contains("web/src/posts/api.rs"), "{detail}");
    assert!(detail.contains("web/src/posts/api/listing.rs"), "{detail}");
}

#[test]
fn a_registrar_entry_that_is_not_web_vertical_leaf_is_reported_as_malformed() {
    let sources = vec![(
        "web/src/posts/api.rs".to_string(),
        "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
    )];
    // Four segments — the private `api` module is not nameable, so this is a typo.
    let registrar = wrap_reg("server_fn::axum::register_explicit::<web::posts::api::Create>();");
    let detail = problems(&sources, &registrar).expect("malformed entry is reported");
    assert!(detail.contains("web::posts::api::Create"), "{detail}");
}

#[test]
fn a_two_segment_registrar_entry_is_reported_as_malformed() {
    let sources = vec![(
        "web/src/posts/api.rs".to_string(),
        "#[server(endpoint = \"/posts/create\")]\npub async fn create() {}\n".to_string(),
    )];
    let registrar = wrap_reg("server_fn::axum::register_explicit::<posts::Create>();");
    let detail = problems(&sources, &registrar).expect("malformed entry is reported");
    assert!(detail.contains("posts::Create"), "{detail}");
}

#[test]
fn a_server_fn_directly_under_web_src_is_an_error() {
    let sources = vec![(
        "web/src/loose.rs".to_string(),
        "#[server(endpoint = \"/x\")]\npub async fn x() {}\n".to_string(),
    )];
    let detail = problems(&sources, &wrap_reg("")).expect("no vertical is an error");
    assert!(detail.contains("web/src/loose.rs"), "{detail}");
    assert!(detail.contains("vertical"), "{detail}");
}
```

- [~] **Step 2: Run the tests, verify they fail** — **SKIPPED; recorded as a
  deviation.** The tests were written before the implementation but never run
  red in between. They are non-vacuous by construction —
  `same_leaf_in_two_verticals_both_registered_is_fine` expects `None` exactly
  where the old leaf-only code emitted "duplicate", and the malformed-entry and
  loose-file cases assert output the old code had no branch to produce — but
  that is an argument, not the observation this step asks for.

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_registrar`
Expected: FAIL — `problems` still keys on the bare leaf, so the cross-vertical
pair reads as a duplicate, malformed entries are silently accepted, and the
loose fn passes.

- [x] **Step 3: Implement against the tests**

Re-key `problems` to `(vertical, leaf)`:

- Derive each source's vertical with `web_server_fns::vertical_of(path)`; an
  `Err` becomes a reported line (pins
  `a_server_fn_directly_under_web_src_is_an_error`) and that file contributes no
  fns.
- Replace `registered_names` with `registered_entries`, and
  `register_explicit_leaf` with `register_explicit_entry`: accept only a
  3-segment `web::<vertical>::<Leaf>` turbofish, returning `(vertical, leaf)`;
  any other shape yields `Err` carrying the path as written (pins both malformed
  tests). Keep the existing `syn`-parsing rationale — a commented-out
  registration must still not count.
- The duplicate check keys `BTreeMap<(&str vertical, &str leaf), Vec<String>>`
  instead of `BTreeMap<&str leaf, _>`, so it fires only within one vertical
  (pins tests 1 and 3). Keep the message's `duplicate` wording and add the
  vertical.
- The missing-registration check tests `registered.contains(&(vertical, leaf))`,
  and its message names the vertical (pins test 2).

Every branch above is pinned by a test, so the body follows from them.

- [x] **Step 4: Update the pre-existing tests for the new path rule**

`problems_flags_an_unregistered_fn_by_name_and_path` (`:367`),
`problems_is_none_when_registrar_covers_every_fn` (`:380`),
`problems_matches_by_leaf_ignoring_reexport_module_path` (`:390`),
`problems_surfaces_a_hard_error_with_the_file` (`:404`) and
`problems_flags_a_duplicate_leaf_name` (`:414`) use paths with no vertical
directory (`web/src/media/mod.rs`, `web/src/x.rs`, `web/src/a.rs`) or assert the
old cross-module duplicate behavior.

- Repath each to `web/src/<vertical>/api.rs` (or `web/src/posts/api/listing.rs`
  for the re-export case), and update its registrar entry to the
  `web::<vertical>::<Leaf>` form.
- **Delete** `problems_flags_a_duplicate_leaf_name` — its behavior (cross-module
  duplicates fail) is now the _opposite_ of
  `same_leaf_in_two_verticals_both_registered_is_fine`, and the within-vertical
  case it was reaching for is covered by
  `a_duplicate_ident_within_one_vertical_fails_even_when_registered`.

- [x] **Step 5: Rewrite the module doc**

`:22-28` currently states "Matching is by leaf type name, not module path" and
justifies the unconditional duplicate failure. Both are now false — a
D10-qualifying factual correction. State: matching is `(vertical, leaf)`; the
vertical is the first path segment under `web/src`; the glob re-export needs no
resolution because both files share a vertical; and the duplicate check is
**per-vertical** because glob shadowing lets two same-ident `#[server]` fns
compile in one vertical (the compiler does **not** own this case).

- [x] **Step 6: Run the tests, verify they pass** — 20/20

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_registrar`
Expected: PASS

- [x] **Step 7: Run the gate against the real tree** — no-op, as predicted

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS — all 55 fns
still register, and today's 55 registrar entries are already
`web::<vertical>::<Leaf>`, so the re-key is a no-op on the current tree.

- [x] **Step 8: Commit** — `1feac14c`

```bash
git add xtask/src/steps/server_fn_registrar_check.rs
git commit -m "feat(xtask): key the server-fn registrar gate on (vertical, leaf) (#684)"
```

---

### Task 4: Convert `server/tests` URL literals to `<T as ServerFn>::PATH`

228 quoted literals across 15 files. Endpoints do not change here — this is
purely removing the hardcoding so Task 7 need not touch these files.

**Files:**

- Modify:
  `server/tests/web/{web_auth,web_posts,web_site,web_account,web_password_reset,audiences,web_media,web_backup,web_tags,web_subscriptions,web_email,web_sessions,router}.rs`,
  `server/tests/feed/feed_events_hook.rs`, `server/tests/misc/media_handlers.rs`

**Interfaces:**

- Consumes: `web::<vertical>::<Leaf>` generated types — the same set already
  named in `server/tests/helpers/mod.rs`'s registrar.
- Produces: no new API. After this task, `rg '"/api/' server/tests` returns
  nothing (AC21).

- [x] **Step 1: Replace every literal with the constant** — 228 across 15 files

Each `"/api/<endpoint>"` becomes `<web::<vertical>::<Leaf> as ServerFn>::PATH`,
e.g.

```rust
// server/tests/web/web_posts.rs:52
post_json(state, <web::posts::CreatePost as ServerFn>::PATH, payload, cookie).await
```

Add `use server_fn::ServerFn;` to each file that gains a `::PATH` reference.
Where a helper already takes the path as a parameter, change the _call site_,
not the helper's signature.

Per **memory: bulk rename → delegate to subagent** — this is 228 mechanical
sites across 15 files; dispatch it via **jaunder-dispatch** with the constraint
that no test logic, assertion, or comment changes (D10). The 3 prose mentions at
`server/tests/web/web_auth.rs:645,694` are comments describing the route and
stay factually correct until Task 7 — leave them.

- [x] **Step 2: Verify no literal survives**

Run: `rg '"/api/[a-z_]+"' server/tests` Expected: no output.

**Note the pattern includes the closing quote.** The looser `rg '"/api/'` cannot
return nothing: `server/tests/web/web_auth.rs` has an assert _message_ —
`"/api/register route not registered (got 404)"` — which is prose naming a
route, not a URL expression. It is deliberately left alone here (converting it
would mean restructuring the assertion into a format arg, i.e. editing test
logic) and is handled with the other prose mentions in Task 7b. Spec AC21 is
worded for the precise pattern for the same reason.

- [x] **Step 3: Run the server integration suite**

Run: `devtool pg run -- cargo nextest run -p jaunder --test integration`
Expected: PASS — the constants resolve to exactly today's URLs, so behavior is
unchanged. A failure means a wrong type was named for an endpoint.

**`devtool pg run --` is load-bearing here.** It wraps the command in a
throwaway PostgreSQL 16 cluster, injecting `JAUNDER_PG_TEST_URL` and
`JAUNDER_PG_BOOTSTRAP_TEST_URL`. A **bare**
`cargo nextest run -p jaunder --test integration` has no PostgreSQL listening,
so every `*_postgres` case fails with `Io(Os { code: 111, ConnectionRefused })`
and nextest cancels after ~16 of 1061 tests — a false red that never reaches the
changed files. (It is `devtool pg`, not `cargo xtask pg`; xtask has no such
subcommand.)

`devtool run -- cargo xtask check` is the heavier alternative — the
Nix-instrumented suite _with_ PostgreSQL plus coverage — and is the per-commit
gate regardless. Reach for `devtool pg run` when you want only the integration
suite.

- [x] **Step 4: Commit** — `bfa49c77`

```bash
git add server/tests
git commit -m "test(server): name server-fn URLs via ServerFn::PATH instead of literals (#684)"
```

---

### Task 5: Rename the 42 `#[server]` fn idents

**Files:**

- Modify: all 14 `web/src/<vertical>/api.rs` (+ `web/src/posts/api/listing.rs`),
  each vertical's `mod.rs` re-export list, every call site in
  `web/src/**/component.rs`
- Modify: `server/tests/helpers/mod.rs:34-88` (55 registrar entries), and every
  `::PATH` reference from Task 4
- Modify: `docs/adr/0011-unified-observability.md:195,258,269,286`,
  `docs/adr/0039-e2e-parallelism-via-per-test-identity-fixtures.md:55`,
  `docs/web-style-guide.md:262,276`

**Interfaces:**

- Consumes: the rename table in the spec, §"Rename table — fns". It is
  authoritative; do not re-derive it.
- Produces: the new fn idents and their `PascalCase` generated type names, which
  Tasks 6 and 7 depend on.

**Outcome — two name collisions the spec's check did not predict.** The spec's
re-export collision check compared new names against each vertical's `mod.rs`
exports; it did **not** consider names _imported into_ the api.rs file, nor
trait names in scope. Both surfaced here and were resolved without renaming any
generated struct (which would break the `PascalCase(ident)` rule):

1. **`posts::audience_selection` generates `AudienceSelection`**, colliding with
   `common::visibility::AudienceSelection`, already imported in
   `web/src/posts/api.rs` and used as that fn's return type and as the
   `audience` field of `CreateArgs`/`UpdateArgs`. Resolved by importing the
   domain type aliased as `DomainAudienceSelection` (5 sites, one file).
2. **`posts::get` / `posts::update` generate `Get` / `Update`**, which shadow
   the `leptos::prelude::{Get, Update}` traits that `.get()`/`.update()` resolve
   through. `web/src/posts/component.rs` makes 7 `.update(…)` calls, so
   importing the structs by name there would have silently broken method
   resolution. Resolved by keeping both out of that import list and spelling
   them `super::Get` / `super::Update` at their use sites.

- [x] **Step 1: Apply the 42 renames — including the `boundary!` label**

Rename each fn ident per the spec table, and in the same edit update: the
vertical's `mod.rs` `pub use api::{…}` list, every in-crate call site, and the
corresponding `server/tests/helpers/mod.rs` registrar leaf
(`web::posts::CreatePost` → `web::posts::Create`) plus every Task-4 `::PATH`
reference.

**Each renamed fn's `boundary!("…")` label must be renamed with it.** This is a
_third_ derived literal, and unlike the span name and endpoint it is **not**
gate-enforced and **not** compiler-enforced — `web/src/lib.rs:15-19` forwards
the string to `error::server_boundary(server_fn: &'static str, …)`
(`web/src/error.rs:115-127`), which emits it as the structured-log/metric field
naming the failing server fn (ADR-0011). All 55 sites currently pass the ident
verbatim; leaving them stale would silently label 42 failures with fns that no
longer exist. Nothing will catch it — the gate for this is filed as a follow-up
in Task 1 Step 3.

```rust
// web/src/posts/api.rs:174 — the label moves with the ident
pub async fn create(args: CreateArgs) -> WebResult<CreateResult> {
    boundary!("create", {   // was boundary!("create_post", { … })
```

Leave `endpoint = "…"` **untouched** — endpoints move in Task 7a. This
deliberately leaves
`#[server(endpoint = "/create_post")] pub async fn create(…)` in the tree for
two commits; nothing enforces the correspondence yet, and it keeps the e2e suite
untouched until the wire actually moves.

Bulk work — dispatch via **jaunder-dispatch** (**memory: bulk rename → delegate
to subagent**). The brief must carry the Global Constraint that `storage/src` is
never touched: `update_post` and `confirm_password_reset` also name storage fns.

- [x] **Step 1b: Verify the rename is complete (AC15, AC16)** — 0 old idents; 55
      `boundary!` labels, none stale; 13/13 unchanged present; `storage/` clean

Run:
`rg -n 'fn (create_audience|rename_audience|delete_audience|list_my_audiences|add_subscriber_to_audience|remove_subscriber_from_audience|list_audience_members|backup_warning_visible|get_backup_settings|update_backup_settings|request_email_verification|verify_email|create_invite|list_invites|list_my_media|media_usage|delete_media|upload_media|request_password_reset|confirm_password_reset|list_user_posts|list_posts_by_tag|list_user_posts_by_tag|create_post|get_post|get_post_preview|update_post|post_audience_selection|publish_post|delete_post|unpublish_post|get_profile|update_profile|get_registration_policy|list_sessions|revoke_session|get_site_identity|update_site_identity|subscribe_to|unsubscribe_from|is_subscribed_to|list_tags)\b' web/src`
Expected: no output — all 42 old idents gone from `web/src` (AC15).

Run: `rg -c 'boundary!\("' web/src` Expected: 55 total. Then spot-check that no
label names an old ident:
`rg -o 'boundary!\("[a-z_]+"' web/src | rg -w 'create_post|create_audience|delete_media|get_profile|list_my_media|get_site_identity|list_tags|revoke_session|verify_email|get_registration_policy|list_audience_members|request_password_reset'`
Expected: no output.

Run:
`rg -n 'fn (login|logout|session|register|list_local_timeline|list_home_feed|default_audience_selection|list_drafts|list_my_subscribers|create_app_password|get_default_post_format|set_default_post_format|base_url_warning_visible)\b' web/src`
Expected: 13 matches — the unchanged idents are still present (AC16).

Since Step 1 is dispatched as bulk work, this is the completion check; the
compiler enforces _call sites_ but would not notice a partially-applied table
row or a stray old definition.

- [x] **Step 2: Update the live docs that name a renamed ident**

Six sites across three files. Update the identifier only — D10 forbids rewording
the surrounding prose.

| Site                                         | Ident                                                                                                      |
| -------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `docs/adr/0011-unified-observability.md:195` | `create_post`, `create_audience`                                                                           |
| `docs/adr/0011-unified-observability.md:224` | `audiences::create_audience` — prose _about_ #684's motivation. Update the ident; the sentence stays true. |
| `docs/adr/0011-unified-observability.md:258` | `request_password_reset` (→ `request`); `login` is unchanged                                               |
| `docs/adr/0011-unified-observability.md:269` | `delete_media`                                                                                             |
| `docs/adr/0011-unified-observability.md:286` | `get_profile`/`update_profile`                                                                             |
| `docs/adr/0039-…:55`                         | `api.create_post`                                                                                          |
| `docs/web-style-guide.md:262`                | `get_registration_policy` (→ `get_policy`)                                                                 |
| `docs/web-style-guide.md:276`                | `list_audience_members` (→ `list_members`)                                                                 |

**Do not touch** `docs/adr/0021-sqlite-transaction-discipline.md:41,52`,
`docs/adr/0022-validate-before-expensive-work.md:20,51`, or
`docs/adr/0026-test-fault-injection-hooks-feature.md:51`. They name
`confirm_password_reset`, `update_post`, and `create_user_with_invite` — but in
their **storage-layer** sense (`storage/src/{sqlite,postgres}/mod.rs`,
`storage/src/posts.rs`), which this issue does not rename. Editing them would be
a scope violation.

Verify the sweep with the **full 42-ident** alternation (a partial list would
return clean while leaving sites stale):

Run:
`rg -nw 'create_audience|rename_audience|delete_audience|list_my_audiences|add_subscriber_to_audience|remove_subscriber_from_audience|list_audience_members|backup_warning_visible|get_backup_settings|update_backup_settings|request_email_verification|verify_email|create_invite|list_invites|list_my_media|media_usage|delete_media|upload_media|request_password_reset|confirm_password_reset|list_user_posts|list_posts_by_tag|list_user_posts_by_tag|create_post|get_post|get_post_preview|update_post|post_audience_selection|publish_post|delete_post|unpublish_post|get_profile|update_profile|get_registration_policy|list_sessions|revoke_session|get_site_identity|update_site_identity|subscribe_to|unsubscribe_from|is_subscribed_to|list_tags' docs CONTEXT.md CONTRIBUTING.md --glob '!docs/archive/**' --glob '!docs/superpowers/**'`

Expected: exactly the five storage-layer lines listed above, and nothing else.

- [x] **Step 3: Run the gate — spans are rewritten for you** — 42 span literals
      rewritten by `Mode::Fix`, none hand-edited

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS.
`server-fn-tracing`'s `Mode::Fix` rewrites all 42 changed span-name literals
(`web.posts.create_post` → `web.posts.create`). Confirm with
`git diff --stat web/src` that the span edits appear — **do not hand-edit any
span literal**.

- [x] **Step 4: Run the full host test suite** — via `cargo xtask check` (green)

Run: `devtool run -- cargo nextest run --workspace` Expected: PASS

- [x] **Step 5: Commit** — `3a211658` (61 files)

```bash
git add web/src server/tests docs/adr
git commit -m "refactor(web): drop vestigial vertical nouns from server-fn idents (#684)"
```

---

### Task 6: Rename the 12 wire DTOs

**Files:**

- Modify: `web/src/audiences/api.rs:35`, `web/src/invites/api.rs:29`,
  `web/src/media/api.rs:39,51,59`, `web/src/posts/api.rs:76,89,117,128,142`,
  `web/src/profile/api.rs:32`, `web/src/sessions/api.rs:23`, each vertical's
  `mod.rs` re-export list, and every call site across `web/`, `server/`,
  `client/`

**Interfaces:**

- Consumes: the spec's §"Rename table — wire DTOs" (12 rows, all bare strips).
- Produces: `audiences::Summary`, `invites::Info`,
  `media::{Item, UsageData, DeleteResult}`,
  `posts::{CreateResult, UpdateResult, PublishResult, CreateArgs, UpdateArgs}`,
  `profile::Data`, `sessions::Info`.

**Outcome — a derive-generated companion the DoD grep could not see.**
`AudienceSummary` derives `reactive_stores::Store`, which generates a trait
named by concatenation: `AudienceSummaryStoreFields` (imported at
`web/src/audiences/component.rs:6`). `rg -w AudienceSummary` does **not** match
it, so the word-boundary check in Step 2 would have passed with a stale
identifier in the tree. Renamed to `SummaryStoreFields`; a substring (non-`-w`)
sweep across `web server common client csr` afterwards confirms no other renamed
type has such a companion. Worth remembering for any future type rename: check
substrings, not just word boundaries, when the type carries a derive that
generates named items.

- [x] **Step 1: Apply the 12 renames**

Rename each type per the spec table, updating its `mod.rs` re-export and every
call site. `AuthUser`, `AuthRejection`, and `TagInputState` are **not** renamed
(out of scope, filed in Task 1).

Bulk work — dispatch via **jaunder-dispatch**.

- [x] **Step 2: Verify no old name survives** — zero matches, and zero as
      _substrings_ too (the stricter sweep that catches derive companions)

Run:
`rg -w 'AudienceSummary|InviteInfo|MediaItem|MediaUsageData|DeleteMediaResult|CreatePostResult|UpdatePostResult|PublishPostResult|CreatePostArgs|UpdatePostArgs|ProfileData|SessionInfo' web server common client`
Expected: no output.

- [x] **Step 3: Run the full host test suite** — via `cargo xtask check` (green)

Run: `devtool run -- cargo xtask check` Expected: PASS. These are
`Serialize`/`Deserialize` structs whose _field_ names carry the wire format; the
type name is not serialized, so no wire behavior changes.

- [x] **Step 4: Commit** — `30f69a5d` (22 files)

```bash
git add web/src server client
git commit -m "refactor(web): drop vertical nouns from server-fn wire types (#684)"
```

---

### Task 7a: Add the `server-fn-endpoint` gate and flip the wire (Rust)

The gate's `Mode::Fix` performs the 55 endpoint rewrites, so they are generated
rather than typed. Split from the e2e/doc follow-through (Task 7b) so the new
gate, its tests, the wiring, and the machine-generated wire flip are one
reviewable Rust-only commit.

**Green boundary:** this commit is green under `cargo xtask check` and
`validate --no-e2e`, but the **e2e suite is red between 7a and 7b** — the wire
has moved and the Playwright literals have not. That is deliberate and is why 7b
follows immediately; do not run `e2e-local` at the end of 7a.

**Files:**

- Create: `xtask/src/steps/server_fn_endpoint_check.rs`
- Modify: `xtask/src/lib.rs:27-28` (module list), `:299-300` (check wiring),
  `:333-334` (validate wiring)
- Modify: all 14 `web/src/<vertical>/api.rs` + `web/src/posts/api/listing.rs` —
  **by the gate, not by hand**
- Test: `server/tests/web/router.rs` (the multi-segment regression guard)

**Interfaces:**

- Consumes:
  `web_server_fns::{vertical_of, rewrite_attr_arg, apply_fixes, LineFix, server_fns_in, read_web_sources, WEB_SRC}`
  (Task 2); `crate::result::{CommandResult, StepResult}`; `crate::Mode`.
- Produces: `pub fn run(mode: Mode, result: &mut CommandResult)` — the
  `server-fn-endpoint` step, mirroring `server_fn_tracing_check::run`'s shape at
  `:611-647`.

- [x] **Step 1: Write the failing gate tests** — all 7 exist and pass

In `xtask/src/steps/server_fn_endpoint_check.rs`'s test module. Reuse the
tracing gate's `src(vertical, body)` helper shape (`:653-656`).

```rust
fn src(vertical: &str, body: &str) -> Vec<(String, String)> {
    vec![(format!("web/src/{vertical}/api.rs"), body.to_string())]
}

#[test]
fn conforming_endpoint_is_accepted() {
    let s = src("posts", "#[server(endpoint = \"/posts/create\")]\npub async fn create() -> R {}\n");
    assert_eq!(problems(&s), None);
}

#[test]
fn a_stale_endpoint_is_flagged_with_the_expected_value() {
    let s = src("posts", "#[server(endpoint = \"/create_post\")]\npub async fn create() -> R {}\n");
    let detail = problems(&s).expect("stale endpoint is a problem");
    assert!(detail.contains("/posts/create"), "must state the expected value: {detail}");
}

#[test]
fn a_missing_endpoint_is_flagged_as_a_hash_hazard() {
    // Without `endpoint`, server_fn derives the URL from a hash of
    // CARGO_MANIFEST_DIR + module_path!(), which varies by checkout directory.
    let s = src("tags", "#[server]\npub async fn list() -> R {}\n");
    let detail = problems(&s).expect("a missing endpoint is a problem");
    assert!(detail.contains("CARGO_MANIFEST_DIR"), "{detail}");
}

#[test]
fn two_fns_deriving_the_same_endpoint_are_flagged_with_both_locations() {
    let s = vec![
        (
            "web/src/posts/api.rs".to_string(),
            "#[server(endpoint = \"/posts/create\")]\npub async fn create() -> R {}\n".to_string(),
        ),
        (
            "web/src/posts/api/listing.rs".to_string(),
            "#[server(endpoint = \"/posts/create\")]\npub async fn create() -> R {}\n".to_string(),
        ),
    ];
    let detail = problems(&s).expect("a duplicate endpoint is a problem");
    assert!(detail.contains("web/src/posts/api.rs"), "{detail}");
    assert!(detail.contains("web/src/posts/api/listing.rs"), "{detail}");
}

#[test]
fn a_server_fn_directly_under_web_src_is_an_error() {
    let s = vec![(
        "web/src/loose.rs".to_string(),
        "#[server(endpoint = \"/x\")]\npub async fn x() -> R {}\n".to_string(),
    )];
    let detail = problems(&s).expect("no vertical is an error");
    assert!(detail.contains("web/src/loose.rs"), "{detail}");
}

#[test]
fn fix_rewrites_the_endpoint_preserving_other_arguments() {
    let src_text = "#[server(endpoint = \"/create_post\", input = Json)]\npub async fn create() -> R {}\n";
    let fixes = endpoint_fixes("web/src/posts/api.rs", src_text);
    let fixed = web_server_fns::apply_fixes(src_text, fixes);
    assert!(fixed.contains("endpoint = \"/posts/create\""), "{fixed}");
    assert!(fixed.contains("input = Json"), "other args must survive: {fixed}");
}

#[test]
fn fix_does_not_synthesize_a_missing_endpoint() {
    // A missing `endpoint` is a hard error for the author to resolve, not
    // something Mode::Fix invents.
    let src_text = "#[server]\npub async fn list() -> R {}\n";
    assert!(endpoint_fixes("web/src/tags/api.rs", src_text).is_empty());
}
```

- [~] **Step 2: Run the tests, verify they fail** — **SKIPPED; recorded as a
  deviation.** The implementer created the module with its seven tests in one
  write rather than landing them red first, so no red phase was observed. Same
  deviation as Task 3 Step 2. All seven are the plan's verbatim tests and pass.

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_endpoint`
Expected: FAIL — the module does not exist.

- [x] **Step 3: Implement the gate**

Write `server_fn_endpoint_check.rs` with:

- `fn endpoint_of(attr: &syn::Attribute) -> Result<Option<String>, String>` —
  the `endpoint = "…"` literal, `Ok(None)` when absent, `Err` on an unparseable
  argument list. Mirror `server_fn_default_named`'s `Meta` matching
  (`server_fn_registrar_check.rs:94-107`).
- `fn problems(web_sources: &[(String, String)]) -> Option<String>` — per fn:
  resolve `vertical_of`; the expected endpoint is
  `format!("/{vertical}/{ident}")`; report a mismatch (naming the expected
  value), a missing endpoint (naming `CARGO_MANIFEST_DIR`), and any endpoint
  claimed by two fns (naming both `file:line`). Close with a `recovery:` line,
  matching the sibling gates' shape.

  **On the duplicate-endpoint rule (AC10):** it is _defence in depth_, not an
  independent check. Because the expected endpoint is derived as
  `/{vertical}/{ident}`, two fns can only collide on it by sharing
  `(vertical, ident)` — exactly what the re-keyed registrar gate already
  hard-fails (D2/AC3). Keep it: it is cheap, and it holds the line if the
  derivation rule ever changes. But do not present it in the code comment as
  catching something the registrar gate misses.

- `fn endpoint_fixes(path: &str, src: &str) -> Vec<LineFix>` — mirrors
  `name_fixes` (`server_fn_tracing_check.rs:516-547`) but calls
  `rewrite_attr_arg(&current, "server", "endpoint", &desired, false)`, so a
  missing `endpoint` is never synthesized. (Five arguments —
  `attr_src, attr_name, key, desired, insert_if_absent`. An earlier draft of
  this line spelled a four-argument call, from before the plan review added
  `attr_name`; Task 2's Interfaces block is the authoritative signature.)
- `pub fn run(mode: Mode, result: &mut CommandResult)` — mirrors
  `server_fn_tracing_check::run` (`:611-647`): read sources, apply fixes under
  `Mode::Fix`, then report.

Wire into `xtask/src/lib.rs`: `pub mod server_fn_endpoint_check;` at `:27-28`,
`steps::server_fn_endpoint_check::run(Mode::Fix, &mut result);` after the
tracing gate at `:300`, and `Mode::Check` at `:334`.

- [x] **Step 4: Run the tests, verify they pass** — 7/7

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_endpoint`
Expected: PASS

- [x] **Step 5: Let the gate flip all 55 endpoints** — 55 rewritten, zero
      hand-edited; green in a single invocation as designed

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS in a single
invocation. `run` applies the fixes to the in-memory sources and _then_
evaluates `problems` against the fixed text — mirror
`server_fn_tracing_check.rs:624-645`, where the fix loop assigns `*src = fixed`
before `problems(&sources)` runs. If the gate needed a second pass to go green,
this task's commit-gating run would be red; that is the signal that `run` was
built to the wrong shape.

Verify: `git diff --stat web/src` shows 55 endpoint edits;
`rg 'endpoint = "/[a-z_]+"' web/src` returns nothing (every endpoint now has two
segments). **Do not hand-edit an endpoint literal.**

- [x] **Step 6: Write the router regression test** — passes on both backends

The multi-segment wildcard is the assumption the whole scheme rests on.

Note the existing `session_api_route_returns_ok`
(`server/tests/web/router.rs:73-95`) becomes an _implicit_ prover once Task 4
converts its `"/api/session"` literal to `::PATH` and this task flips the
endpoint to `/auth/session` — it would 404 if the wildcard did not capture two
segments. That is coincidental coverage, not a guard: its subject is the session
route. Add an explicit one alongside it, in the same dual-backend shape (a bare
`#[tokio::test]` fails the `test-backend-pattern` guard, ADR-0053):

```rust
/// `server/src/lib.rs:65` mounts every server fn under one wildcard,
/// `"/api/{*fn_name}"`. The #684 endpoint scheme (`/api/<vertical>/<op>`) is only
/// viable if that wildcard captures multi-segment remainders — matchit's own
/// doctest says it does (`matchit-0.8.4/src/lib.rs:47-48`); this pins it so an
/// axum upgrade cannot silently 404 every server-fn route at once.
#[apply(backends)]
#[tokio::test]
async fn multi_segment_server_fn_route_is_reachable(#[case] backend: Backend) {
    let TestEnv { state, base: _base } = backend.setup().await;
    ensure_server_fns_registered();
    let app = jaunder::create_router(state, noop_mailer(), true, tmp_storage_path());
    let path = <web::auth::Session as ServerFn>::PATH;
    assert_eq!(path, "/api/auth/session", "the #684 scheme under test");
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::empty())
                .expect("failed to build request"),
        )
        .await
        .expect("failed to get response");
    assert_ne!(
        response.status(),
        StatusCode::NOT_FOUND,
        "`/api/{{*fn_name}}` must capture a multi-segment server-fn path"
    );
}
```

Assert **not-404** rather than a success status: the subject is routing, so a
401/422 still proves the request reached the fn — and that keeps the guard from
breaking when the session fn's own behavior changes. `auth::session` is one of
the 13 unchanged idents, so this test's expected path is stable across the
rename.

- [x] **Step 7: Run the gates in both modes**

Run: `devtool run -- cargo xtask check` Expected: PASS, with
`server-fn-endpoint` present in the step list (spec AC8).

Run: `devtool run -- cargo xtask validate --no-e2e --allow-dirty` Expected:
PASS, with `server-fn-endpoint` present (AC8 names `validate`). This is the
**only** step in the plan that exercises the `Mode::Check` path — the one CI
runs, and the one that must not mutate the tree. Confirm
`git status --porcelain` is **byte-identical before and after** the run; that,
not a clean tree, is the no-mutation evidence.

**`--allow-dirty` is required here.** `validate`'s `clean-tree` precheck
hard-fails a dirty tree, and Step 8 (commit) has not run yet, so at this point
the tree is necessarily dirty. Bare `validate --no-e2e` — the AC29 form, with
`clean-tree` green — is run **after** Step 8 lands, on the now-clean tree.

- [x] **Step 8: Commit** — `ddfe4e7a` — then re-ran bare
      `devtool run -- cargo xtask validate --no-e2e` on the clean tree: exit 0,
      `clean-tree` green, and `git status --porcelain` empty afterwards (AC29)

AC29 names the bare form. Running it post-commit is the only point in the cycle
where the tree is clean enough for `clean-tree` to pass.

```bash
git add xtask/src web/src server/tests/web/router.rs
git commit -m "feat(xtask): derive and enforce server-fn endpoints as /<vertical>/<op> (#684)"
```

---

### Task 7b: Follow the wire move through e2e and the docs

The wire moved in 7a; this is everything outside Rust that names a URL. Separate
commit because e2e is the only thing that exercises it, and because 7a's diff is
machine-generated while this one is hand-edited.

**Files:**

- Modify:
  `end2end/tests/{media.spec.ts,feeds.spec.ts,backup.spec.ts,posts.ts,audiences.spec.ts,helpers.ts,authed-flash.spec.ts}`
- Modify: `docs/observability.md:326`,
  `docs/adr/0046-test-support-seed-binary.md:10,23`,
  `docs/adr/0016-dependency-injection-and-appstate.md:272`
- Modify: `server/tests/web/web_auth.rs:645,694` (the 3 prose route mentions)

**Interfaces:**

- Consumes: the endpoints as flipped by Task 7a. Read the actual values out of
  `web/src/<vertical>/api.rs` rather than deriving them by hand.

**Outcome — the 14-site list was incomplete, and the e2e run is what proved
it.** Both this plan and spec AC22 enumerated the e2e work by grepping for
`/api/`. That pattern is too narrow: `end2end/tests/profile.spec.ts` matches
responses with `response.url().includes("update_profile")` — a **bare fn ident,
no `/api/` prefix** — in six `waitForResponse` predicates. The first `e2e-local`
run after the wire moved failed exactly there: 4 specs, all
`page.waitForResponse: Test timeout of 30000ms exceeded`, because
`/api/profile/update` does not contain `update_profile`.

The correct sweep is **every renamed ident as a bare word** across `end2end/`,
not `/api/`-prefixed URLs. That found 6 functional predicates plus 9 stale
labels/comments (a `withTimedAction` metric label, two assertion messages, six
comments) — 15 further edit points beyond the 14 originally listed. Note
`set_default_post_format` predicates keep working, because that ident is one of
the 13 unchanged; only renamed ones broke.

This is precisely the drift class **#712** was filed for, demonstrated: no gate
ties these strings to the endpoints, so a red e2e matrix was the only detector —
and it reported a 30-second timeout rather than naming the drift.

- [x] **Step 1: Update the 14 e2e edit points** — plus the 15 found by the
      bare-ident sweep above

Per spec AC22. Direct literals: `media.spec.ts:13,39` (`/api/media/upload`),
`feeds.spec.ts:263` (`/api/posts/update`), `backup.spec.ts:106,122`
(`/api/backup/update_settings`), `posts.ts:15,31` (`/api/posts/create`),
`audiences.spec.ts:97,98` (`/api/audiences/list_members`,
`/api/audiences/list_mine`). `failServerFn` arguments:
`audiences.spec.ts:182,199,230` (`audiences/list_mine`,
`audiences/list_members`, `audiences/list_my_subscribers`),
`authed-flash.spec.ts:111` (`auth/session`). `helpers.ts:97` doc comment — its
`#[server(endpoint = "/name")]` … `/api/name` description is now factually
wrong, so it is D10-qualifying.

**`helpers.ts:109` needs no edit.** It is
`await page.route(\`\**/api/${endpoint}\`, …)` — a template whose *interpolated
argument* changes at the call sites, not the template itself. The glob still
matches a two-segment path. Editing it would be a no-op change, which D10
forbids. (Spec AC22 counts it among the 11 `/api/…` lines; that count is of
*occurrences\*, not required edits — 14 edit points, not 15.)

- [x] **Step 2: Update the URL docs and the 3 Rust prose mentions** — 4 in
      `web_auth.rs` (`:652`, `:712`, `:725`), plus `observability.md:326`,
      `adr/0046:10`, `adr/0016:272`. Deliberately left: `observability.md:158`'s
      `/api/current_user` (names a server fn that does not exist — stale prose,
      out of scope per the spec) and `adr/0046:23`'s `/api/seed_posts` (a
      _rejected_ alternative, so not a claim about today's tree).

`docs/observability.md:326` (`POST /api/posts/create`), `docs/adr/0046:10,23`,
`docs/adr/0016:272` (`POST /api/{fn}` → `POST /api/{vertical}/{fn}`), and
**four** Rust prose mentions in `server/tests/web/web_auth.rs`: the two route
comments at `:645,694` plus the assert message
`"/api/register route not registered (got 404)"` (near `:701` after Task 4's
reflow) — Task 4 deliberately left that one because rewriting it means
restructuring the assertion. Identifier/URL only — no rewording.

- [x] **Step 3: Run the e2e suite** — **111 passed (4.5m)**, exit 0. The first
      attempt failed 4 specs and is what exposed the incomplete sweep above.

Run: `devtool run -- cargo xtask e2e-local` Expected: PASS — the only detector
of a missed e2e literal (**memory: local e2e runs here**). A 404 in a Playwright
request is the signature of a missed site.

- [x] **Step 4: Commit** — `03e0d2ec` (15 files)

```bash
git add end2end/tests docs server/tests/web/web_auth.rs
git commit -m "test(e2e): follow the server-fn endpoints to /<vertical>/<op> (#684)"
```

---

### Task 8: Record the decisions

**Files:**

- Modify: `docs/adr/0066-server-fn-test-registrar-guard.md` (Decision `:56-64`,
  Consequences `:78-82`)
- Create: `docs/adr/drafts/server-fn-wire-namespace.md`

**Interfaces:**

- Consumes: the spec's D1, D2, D5, and §"Why the duplicate check cannot simply
  be deleted".

- [x] **Step 1: Amend ADR-0066**

In _Decision_ `:56-64`, replace "It **matches by leaf type name, not module
path**, because re-exports (`pub use listing::*`) make the registrar path differ
from the source path" with the `(vertical, leaf)` rule: the vertical is the
first path segment under `web/src`, which makes the re-export irrelevant because
both files share a vertical, and registrar entries must be exactly
`web::<vertical>::<Leaf>`.

In _Consequences_ `:78-82`, correct the stale bullet. It currently calls leaf
collision an "accepted limitation … benign" that could let a fn **slip through**
— the code hard-**failed**. Replace with: the duplicate check is scoped to one
vertical and hard-fails there, because glob shadowing lets two same-ident
`#[server]` fns compile in one vertical (so the compiler does not own this
case); cross-vertical same-leaf pairs are no longer collisions at all. **Keep**
the bullet's still-true observation that a within-vertical pair also collides at
the endpoint level — the `server-fn-endpoint` gate (#684) now enforces that
independently. Do **not** claim the new design has no limitation.

Per **memory: ADR promote → prettier the README** and **memory: pre-commit
prettier restages prose**, run `prettier -w` on the edited markdown before
staging.

- [x] **Step 2: Draft the wire-namespace ADR** —
      `docs/adr/drafts/server-fn-wire-namespace.md`

Create `docs/adr/drafts/server-fn-wire-namespace.md` from `docs/adr/template.md`
(numberless — `cargo xtask adr promote` numbers it at ship, per ADR-0048).
Content:

- **Context:** `server/src/lib.rs:65` mounts every server fn under one wildcard,
  so endpoints are a flat global namespace; the vertical noun in the _endpoint_
  is therefore load-bearing exactly where it is vestigial in the _ident_.
- **Decision:** endpoints are `/api/<vertical>/<op>`, derived from
  `(vertical, fn ident)` and written by the `server-fn-endpoint` gate under
  `Mode::Fix`.
- **Why `endpoint` stays pinned:** without it,
  `server_fn_macro-0.8.10/src/lib.rs:515-521` derives the URL from
  `xxh64(CARGO_MANIFEST_DIR + ":" + module_path!())` — an absolute path, so the
  URL varies by checkout directory and cannot be named in docs or tests.
- **Consequences:** Rust callers name `<T as ServerFn>::PATH` and cannot drift;
  the e2e suite has no such constant and remains hand-maintained (the Task 1
  follow-up issue); endpoint uniqueness is entirely gate-enforced because a
  pinned `endpoint` suppresses the disambiguating hash.
- Note the multi-segment wildcard dependency and the
  `server/tests/web/router.rs` guard from Task 7.

- [x] **Step 3: Run the gate** — `adr-format` and `adr-readme-parity` green

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS, including
`adr-check`.

- [ ] **Step 4: Commit**

```bash
git add docs/adr
git commit -m "docs(adr): re-key the registrar guard to (vertical, leaf); draft the wire-namespace ADR (#684)"
```

---

## Self-review

**Spec coverage** — every AC maps to a task. Two rows span tasks; they are
called out rather than collapsed.

| AC                                                      | Task                              | AC                             | Task                  |
| ------------------------------------------------------- | --------------------------------- | ------------------------------ | --------------------- |
| AC1–AC6 (registrar gate)                                | 3                                 | AC19 (spans)                   | 5 (auto-fixed)        |
| **AC7** (loose-file error, **both** gates)              | **3 (registrar) + 7a (endpoint)** | AC20 (endpoints)               | 7a                    |
| AC8–AC12 (endpoint gate)                                | 7a                                | AC21 (`::PATH`)                | 4                     |
| AC13–AC14 (shared module)                               | 2                                 | AC22 (e2e)                     | 7b                    |
| AC15, **AC15b**, AC16 (fn renames + `boundary!` labels) | 5 (verified at Step 1b)           | AC23 (router test)             | 7a                    |
| AC17 (wire DTOs)                                        | 6                                 | AC24–AC25 (docs)               | 5 (idents), 7b (URLs) |
| AC18 (registrar entries)                                | 5                                 | AC26 (D10)                     | all tasks             |
|                                                         |                                   | AC27–AC28 (ADRs)               | 8                     |
|                                                         |                                   | **AC29** (`validate --no-e2e`) | **7a Step 7**         |
|                                                         |                                   | AC30 (e2e green)               | 7b Step 3             |

AC7 is the one AC no single task satisfies: the shared `vertical_of` gives the
registrar gate its loose-file error in Task 3 and the endpoint gate its own in
Task 7a Step 1.

AC29 named `cargo xtask validate --no-e2e`, which **no earlier draft of this
plan ever ran** — every task ran `check` or `check --no-test`. Since `check`
wires `Mode::Fix` and `validate` wires `Mode::Check` (`xtask/src/lib.rs:299-300`
vs `:333-334`), the non-mutating path CI actually runs was never exercised. Task
7a Step 7 now runs it, including a clean-tree assertion.

**Type consistency:**
`rewrite_attr_arg(attr_src, attr_name, key, desired, insert_if_absent)` (Task 2)
is the name and arity used at every call site in Tasks 2 and 7a.
`registered_entries` / `register_explicit_entry` (Task 3) replace
`registered_names` / `register_explicit_leaf` and are not referenced elsewhere.
`endpoint_fixes` (Task 7a) mirrors `name_fixes` and is used only in that
module's `run` and tests. No symbol appears in a later task that an earlier task
does not define.

**Ordering invariant:** every task leaves `cargo xtask check` green — Task 3's
re-key is a no-op on today's tree (all 55 registrar entries are already
`web::<vertical>::<Leaf>`); Task 5 renames idents while endpoints stay put, so
e2e is untouched and the stale `endpoint = "/create_post"` on
`pub async fn create` is genuinely unenforced in that window; Task 4 removes the
Rust literals _before_ 7a moves the wire. The one gap in the invariant is **e2e
between 7a and 7b**, stated explicitly above rather than left implicit.
