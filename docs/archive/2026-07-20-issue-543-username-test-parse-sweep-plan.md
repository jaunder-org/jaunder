# Plan — #543: sweep inline `Username` test-parses to `parse_username`

- Spec:
  [2026-07-20-issue-543-username-test-parse-sweep.md](../specs/2026-07-20-issue-543-username-test-parse-sweep.md)
- Issue: [#543](https://github.com/jaunder-org/jaunder/issues/543)
- For agentic workers: drive with **jaunder-iterate**; the mechanical sweep is
  **delegated to a subagent** (jaunder-dispatch) to keep file bulk out of the
  driver's context.

## Review header

**Goal.** Mechanically route the ~28 test-fixture `Username` parses (enumerated
in the spec) through `common::test_support::parse_username`; leave runtime
parses and `Username`'s own tests alone. No behavior change.

**Scope.** In: the 11 files in the spec's sweep table. Out: the spec's "Do NOT
touch" list (runtime parses + `common/src/username.rs`).

**Tasks.**

1. Delegate the sweep (below) to a subagent; verify its result (gate + diff).

**Key risks / decisions.**

- **Runtime vs. fixture is the whole game.** The sweep must not touch the "Do
  NOT touch" sites — those parse untrusted input and must stay fallible. The
  subagent gets the exact keep-list.
- **Import hygiene.** Each swept module adds
  `use common::test_support::parse_username;` and drops a now-unused
  `use …::Username` (clippy `unused_imports` would fail the gate otherwise).
- **`.ok()` form.** `web/src/forms/field.rs:212` is
  `"alice".parse::<Username>().ok()` (an `Option<Username>` for comparison) →
  `Some(parse_username("alice"))`, not a bare `parse_username`.
- **`var.parse()` form.** `posts/listing.rs:365`, `subscriptions/server.rs:47`
  parse a `&str` variable → `parse_username(var)`; if the variable is not
  `&str`, adapt (`parse_username(var.as_ref())` or leave if it's already typed).
- **No new deps.** `parse_username` (from #542, on main) is reachable —
  `storage` and `web` already carry `common`'s `test-support` dev-dep.

## Global constraints

- Rust; structured Edits (no shell text-munging). No `Co-Authored-By`. Gate:
  `cargo xtask check` clean before commit. Review base `wt-base-issue-543`.

---

## Task 1 — the sweep (delegated)

**Subagent brief:** in each file in the spec's sweep table, replace the inline
`Username` test-fixture parse with `parse_username(…)` per the transforms in the
spec's Decision; add the `parse_username` import and remove any now-unused
`Username` import; touch **nothing** in the spec's "Do NOT touch" list. Return a
per-file summary of sites changed.

**Verify (driver):**

- `cargo xtask check` — PASS (clippy incl. no-unused-imports; coverage clean).
- `git diff wt-base-issue-543 -- common/src/username.rs storage/src/helpers.rs server/ web/src/pages/posts.rs common/src/feed/feed_path.rs`
  is **empty** (keep-list untouched).
- Grep: no `parse::<Username>().unwrap()` /
  `let _: Username = "…".parse().unwrap()` fixture form remains in the swept
  files.

**Commit** after gate green (**jaunder-commit**).

## Self-review checklist

- [ ] All 11 sweep files use `parse_username`; keep-list byte-unchanged.
- [ ] No unused `Username` imports; clippy clean.
- [ ] `cargo xtask check` green; `git status` clean.
