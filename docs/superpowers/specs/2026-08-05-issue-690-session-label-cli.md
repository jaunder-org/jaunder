# Issue #690 — `SessionLabel` through the app-password CLI path

**Issue:** [#690](https://github.com/jaunder-org/jaunder/issues/690) — _types:
SessionLabel through the app-password CLI path (retire the #325 chokepoint)_
**Milestone:** #13 Domain-value type safety (newtypes) **Branch:**
`worktree-issue-690-session-label-cli`

## What is left, and what is not

#690 was rewritten during triage after most of it shipped as **#685**. Verified
against the tree at this fork point:

- `web/src/auth/api.rs:54` is already `label: Option<SessionLabel>`
  (`7b99aad5`).
- The hand-rolled 200-char User-Agent cap is gone; `SessionLabel::from_lossy`
  owns the bound and the default (`2e21b407`), which also fixed the
  bytes-vs-chars mismatch.
- The doc comment was cleaned so it cannot drift (`1070d4b6`).
- `web/src/audiences/api.rs:48`'s `SubscriberSummary.label` was **misfiled
  here** — it is a display name derived from `subscriber_ref`, not a session
  label. Moved to #750.

**Only the app-password CLI path remains.**

## The defect

A session label reaches storage through two untyped hops, with a validation step
that exists only because of them:

```rust
// server/src/cli.rs:317-318
#[arg(long, default_value = "app-password")]
label: String,

// server/src/commands.rs:238-253
pub async fn app_password_create(state: &AppState, username: &Username, label: &str) -> … {
    …
    // Validate the CLI-supplied label at the `SessionLabel` chokepoint (#325) —
    // the same non-empty/≤255 rule the web wire enforces.
    let label: SessionLabel = label.parse()
        .map_err(|e| anyhow::anyhow!("invalid label: {e}"))?;
```

Two things are wrong with it. The rule is enforced **one call away from the
boundary**, by a step a future edit can drop without anything noticing. And it
is enforced **after** the database is opened (`cmd_app_password_create:272-275`
opens storage, _then_ calls the validating function), so a typo'd label pays a
database connection before being rejected.

`username: Username` sits two lines above `label: String` in the same clap
struct and is already typed. The asymmetry is the whole issue.

## Decisions

### D1 — Type both hops, delete the chokepoint

`cli.rs`'s `label` becomes `SessionLabel`; `app_password_create` and
`cmd_app_password_create` take `&SessionLabel`. The `#325` parse is **deleted,
not relocated** — that is the point. Afterwards the invariant is carried by the
signature and there is no step to keep.

Clap parses it exactly as it already parses `username: Username`: `SessionLabel`
derives `Clone + Debug + PartialEq + Eq` plus `StrNewtype`'s
`FromStr`/`Display`, which is the trait set clap's derived value parser needs.

### D2 — Rejection moves to parse time

An invalid label now fails during clap parsing, before `open_existing_database`
runs. The error text changes from
`invalid label: session label must be non-empty and at most 255 characters` to
clap's own rendering of the same `InvalidSessionLabel` message. This is the same
boundary pattern #687 established for `jaunder site-config set`.

### D3 — The CLI default stays a clap string literal

`#[arg(long, default_value = "app-password")]` is kept as-is, and **not**
hoisted onto `SessionLabel`.

`SessionLabel::DEFAULT` (`common/src/session_label.rs:49`) is private and has no
`Default` impl, so there is nothing to depend on today — but the substantive
reason is that it answers a different question. Its doc: "the label used when a
lossy source yields nothing usable." It exists for _"we tried to derive a device
name and failed."_ An app password minted from the CLI is the opposite case —
the origin is known exactly. Wiring the CLI to it would put **"Unknown device"**
in the sessions UI for every app password created without `--label`: less
informative than today, and actively misleading.

`"app-password"` describes _how the token was minted_, which is CLI knowledge.
Moving it into `common` would push a CLI policy into a domain type for a
symmetry that is not real.

Clap parses the default through the same `FromStr` as user input, so it cannot
be invalid at runtime without a test catching it first (A5).

## Scope

### In

- `server/src/cli.rs:318` — `label: String` → `label: SessionLabel`.
- `server/src/commands.rs:238,241` — `app_password_create` takes
  `&SessionLabel`.
- `server/src/commands.rs:267,270` — `cmd_app_password_create` takes
  `&SessionLabel`.
- `server/src/commands.rs:249-253` — the chokepoint parse deleted.
- `server/src/commands.rs:80` — dispatch (passes `&label`; should still compile
  unchanged).
- `server/src/main.rs:299` — `label: "ert".to_string()` in the test constructing
  `Commands::AppPasswordCreate` (`:296` is the variant; `:299` is the line that
  changes). This module is also the **home for A6's new default-path test**,
  beside `run_app_password_create_mints_for_existing_user` (`:270`).
- `server/tests/misc/commands.rs:230,243` — two call sites passing `"ert"`.
- `server/src/cli.rs` — the new parse-rejection test (A4), beside the existing
  `Username` parse-time rejection test at `:804-806`, which is its direct
  analogue.

### Out — verified rejects

- **`web/src/auth/api.rs`** — already typed by #685; not touched.
- **`SubscriberSummary.label`** — not a session label; moved to #750.
- **`elisp/test/jaunder-integration-helper.el:129`** — passes `--label "ert"`, a
  valid label, through the real binary. It exercises the new parse path and
  needs **no edit**; confirming it still passes is coverage, not scope.
- **Making `--label` mandatory** — a breaking CLI change, and a separate
  argument about whether identical `app-password` labels are useful. Not this
  issue.

## Acceptance criteria

- **A1** `server/src/cli.rs`'s `AppPasswordCreate.label` is `SessionLabel`.
- **A2** `app_password_create` and `cmd_app_password_create` both take
  `&SessionLabel`. `rg -n 'label: &str|label: String' server/src` returns no hit
  on this path.
- **A3** The `#325` chokepoint parse in `commands.rs` **no longer exists** —
  verified by the absence of a `.parse()` on a label in that function, not
  merely by its having moved.
- **A4** An invalid label (empty, or 256+ chars) is rejected at **clap parse**,
  with an error naming the constraint, and **without opening the database**.
  Pinned by a `Cli::try_parse_from` test, which by construction cannot reach
  storage.
- **A5** A `Cli::try_parse_from` test omitting `--label` asserts the parsed
  value equals `"app-password"` — so a future typo in the literal fails a test
  rather than every runtime invocation. Pure parsing; no database.
- **A6** A dual-backend test drives the full path with `--label` omitted and
  reads the label back, asserting the session is recorded as `"app-password"`.
  The default is only applied by clap, so nothing below
  `cmd_app_password_create` can observe it — this test must therefore go through
  argv: `Cli::try_parse_from(...)` → the dispatch → `list_sessions`
  (`storage/src/sessions.rs:87`, `SessionRecord.label: SessionLabel` at `:23`).
  Its home is `server/src/main.rs`'s test module, beside the existing
  `run_app_password_create_mints_for_existing_user` (`:270`), which is the
  precedent for driving a parsed `Commands` through `run`.
- **A7** The existing dual-backend `cmd_app_password_create_*` tests still pass,
  adapted only in how they construct the label — use
  `common::test_support::parse_session_label` (the door already used by
  `storage/src/sessions.rs:261`), not an inline `.parse().unwrap()`.
- **A8** `devtool run -- cargo xtask validate --no-e2e` green.
- **A9** Full `devtool run -- cargo xtask validate` (with e2e) green before the
  PR opens — the `ert` elisp integration test drives the real binary through
  this argument, and no unit test covers that path.
