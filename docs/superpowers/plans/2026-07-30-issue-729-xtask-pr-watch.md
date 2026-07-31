# `cargo xtask pr watch` / `pr land` Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **`jaunder-iterate`** (delegating individual tasks to a subagent via
> **`jaunder-dispatch`** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-30-issue-729-xtask-pr-watch.md`](../specs/2026-07-30-issue-729-xtask-pr-watch.md)
— referenced by decision (D1–D13) and acceptance criterion (A1–A16). **The plan
is "how"; the spec is "what/why." Do not re-derive spec rationale here — read it
there.**

**Goal:** One `cargo xtask pr watch <N>` / `pr land <N>` pair that owns the
whole green→queue→merged sequence and reports a single unambiguous outcome,
replacing the prose protocol every agent currently re-derives.

**Architecture:** A new `xtask/src/pr/` module layered boundary → pure → loop
(D3). `gh.rs` is the only file that runs a subprocess; everything above it sees
typed `PrSnapshot` / `ApiError`. The state machine in `decide.rs` is a pure
function, so all six failure rules are host-testable with hand-built values and
no network.

**Tech Stack:** Rust (xtask's own workspace, `xtask/Cargo.toml`), `clap` derive,
`serde`/`serde_json`, `std::process::Command` for the subprocess, `gh` CLI as
the GitHub transport. **No new crate dependencies.**

## Review header

**Scope — in:** the `pr` module and its two subcommands; the `pr` field on
`CommandResult`; three new readers in `xtask/src/git.rs`; `pkgs.gh` in the
devShell; one ADR; four tracked doc updates; one out-of-tree skill update at
ship.

**Scope — out:** re-running jobs, rebasing, re-enqueueing, merging by any path
other than `pr land`; CI/Nix usage; multi-PR watching; any test touching the
network (spec Non-goals).

| #   | Task                                           | Deliverable                                                                   |
| --- | ---------------------------------------------- | ----------------------------------------------------------------------------- |
| 1   | Outcome + report types, envelope plumbing      | `PrReport` serializes; `ok`/`exit_code` agree with the outcome (A16, A3)      |
| 2   | `gh.rs` — transport and failure classification | `classify()` turns real `gh` specimens into typed `ApiError` (D2, F5)         |
| 3   | `snapshot.rs` — types, fixtures, parsing       | Real captured GitHub JSON → `PrSnapshot` / `RequiredChecks` (D6, F11)         |
| 4   | Shared test fixtures + `decide.rs`             | The pure state machine; every outcome and the six traps (A6, A7, A7b, A8)     |
| 5   | `watch.rs` — the poll loop                     | Virtual clock; watcher-error, rate-limit, fingerprint, heartbeat (A4, A5, A9) |
| 6   | `land.rs` — divergence guard + arming          | Honest arm predicate, re-arm once (A10, A11)                                  |
| 7   | git readers, subject resolution, CLI wiring    | `cargo xtask pr watch` runs end-to-end (A1, A2, A11, A12, A14, A15)           |
| 8   | ADR draft                                      | The four decisions recorded (D12)                                             |
| 9   | Tracked docs + `flake.nix`                     | `pkgs.gh`, CONTRIBUTING, CLAUDE.md, the ci-merge-queue correction (A13)       |
| 10  | Ship-time: manual smoke + skill update         | **At ship, last** — A1 proven against a real PR; A13b out of tree             |

**Key risks / decisions:**

- **Test helpers are shared through a real module, not copied.** A
  `#[cfg(test)] mod tests` is private to its own file, so fakes cannot cross
  files. Task 4 creates `xtask/src/pr/test_support.rs` (declared
  `#[cfg(test)] pub(crate) mod test_support;`) and Tasks 5–6 extend and reuse
  it. Duplicating builders per file is a plan failure.
- **Task 3 needs live GitHub calls to capture fixtures.** Everything after runs
  offline. Capture once, commit, never call again.
- **The `ok` invariant is load-bearing and easy to break.** `push()` recomputes
  `ok`, so the `pr` commands must push **exactly one** `StepResult` (D5). Task 1
  pins it (A16); a second `push()` anywhere silently produces
  `xtask-done: ok=true exit=1`.
- **A3 is enforced by types, not by a test.** `watch()` and `land()` return
  `PrReport`, not `Result<PrReport, _>` — the `Err`-on-a-terminal-outcome path
  the criterion exists to forbid is _unrepresentable_. Only subject resolution
  can bail (D13), and that is a pure, table-tested mapping in Task 7.
- **Task 10 cannot be verified from the PR diff** — `.claude/` is untracked
  (F10). It is deliberately last, after flag names are final, and it is also
  where A1 gets its only real-PR exercise (D11 forbids networked tests, so the
  smoke run is manual).
- **No separable concerns surfaced** during the design interview, so there is no
  issue-filing first task. The one wart noticed (`write_sidecar` hardcodes a
  relative path, `result.rs:114`) is worked around by scoping A3 to the returned
  value, not refactored here.

## Global Constraints

Copied verbatim from the spec; every task's requirements implicitly include
these.

- **No new crate dependencies.** `gh` is the transport (D2). Adding
  `octocrab`/`reqwest` is out of scope.
- **`gh.rs` is the only file that runs a subprocess or knows `gh` exists** (D3).
  `snapshot.rs` parses `serde_json::Value` handed to it; nothing above
  `snapshot.rs` sees JSON at all.
- **`decide.rs` performs no IO and reads no clock** (D3).
- **`watch()` and `land()` return `PrReport`, never `Result`** — every terminal
  outcome is a report (D5/A3). `gh` missing, unauthenticated, or rate-limited
  are `watcher-error` _reports_.
- **The `pr` commands push exactly one `StepResult`**, `ok` iff the outcome is
  `merged` (D5/A16).
- **No check name is hardcoded** in `decide.rs`; required contexts, the strict
  flag, and queue-presence come from `/rules/branches/main` per run (D6/A8).
- **No test reaches the network** (D11). Fixtures are the contract.
- **Exit codes:** 0 iff `merged`; 1 for every other terminal outcome and
  `pending`; 2 only for failures preceding any report (D5/D13/A2).
- **Timing defaults:** interval 30s (`--interval <SECONDS>`, range `5..`),
  budget 90 min (`--timeout <MINUTES>`, range `1..`), 5 consecutive transient
  failures → `watcher-error` (D7).
- Test command for every task:
  `cargo test --manifest-path xtask/Cargo.toml <filter>` (matching
  `xtask/src/steps/host_tests.rs:12-17`).
- **Test fn names must be `snake_case`** — `static_checks.rs:119-131` runs
  `cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings`,
  and `--all-targets` lints `#[cfg(test)]` code, so a `_NOT_` in a test name
  fails the gate.
- Per-commit gate: run `cargo xtask check` clean before committing
  (**`jaunder-commit`**). **No `Co-Authored-By` trailer.**

---

### Task 1: Outcome and report types; envelope plumbing

**Files:**

- Create: `xtask/src/pr/mod.rs` (all Task 1 tests live here)
- Modify: `xtask/src/result.rs` (add the `pr` field; add the `PrReport` render
  arm in `print_human()`)
- Modify: `xtask/src/lib.rs:5-21` (add `pub mod pr;` to the flat module list —
  **not** the `mod steps { … }` block at `:22-43`)
- Test: in-file `#[cfg(test)]` in `xtask/src/pr/mod.rs`

**Interfaces:**

- Consumes: `CommandResult`, `StepResult` from `xtask/src/result.rs`.
- Produces — every later task depends on these exact names:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrNumber(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject { pub owner: String, pub repo: String, pub number: PrNumber }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Merged, ChecksFailed, Ejected, Conflicted, ClosedUnmerged,
    Stale, TimedOut, WatcherError, Pending,
}

impl Outcome { pub fn is_merged(self) -> bool }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind { Phase, Check, Queue, Heartbeat, PollError, Warning, Terminal }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Event { pub at: String, pub kind: EventKind, pub detail: String }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PrReport {
    pub outcome: Outcome,
    pub pr: u64,
    pub head_sha: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub pointer: Option<String>,
    pub events: Vec<Event>,
}

/// Build the single-step `CommandResult` (D5). The ONLY constructor the `pr`
/// commands use — this is what keeps `ok` in sync with the outcome.
pub fn into_result(command: &str, report: PrReport) -> CommandResult;
```

- [x] **Step 1: Write the failing tests**

All six live in `xtask/src/pr/mod.rs` so `cargo test … pr::` runs them.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn report(outcome: Outcome) -> PrReport {
        PrReport {
            outcome, pr: 731, head_sha: "abc123".into(),
            phase: None, detail: None, pointer: None,
            events: vec![Event { at: "2026-07-30T14:02:11Z".into(),
                                 kind: EventKind::Phase, detail: "awaiting-checks".into() }],
        }
    }

    #[test]
    fn merged_result_is_ok_and_exits_zero() {
        let r = into_result("pr-watch", report(Outcome::Merged));
        assert!(r.ok);
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn non_merged_result_is_not_ok_and_exits_one() {
        for outcome in [Outcome::ChecksFailed, Outcome::Ejected, Outcome::Conflicted,
                        Outcome::ClosedUnmerged, Outcome::Stale, Outcome::TimedOut,
                        Outcome::WatcherError, Outcome::Pending] {
            let r = into_result("pr-watch", report(outcome));
            assert!(!r.ok, "{outcome:?} must not be ok");
            assert_eq!(r.exit_code(), 1, "{outcome:?} must exit 1");
        }
    }

    #[test]
    fn exactly_one_step_is_pushed() {
        // Load-bearing: `push()` recomputes `ok` from the step vector (result.rs:90-93),
        // so a second step would decouple `ok` from the outcome.
        let r = into_result("pr-watch", report(Outcome::ChecksFailed));
        assert_eq!(r.steps.len(), 1);
        assert_eq!(r.steps[0].name, "pr-watch");
    }

    #[test]
    fn outcomes_serialize_kebab_case() {
        assert_eq!(serde_json::to_value(Outcome::WatcherError).unwrap(), "watcher-error");
        assert_eq!(serde_json::to_value(Outcome::ClosedUnmerged).unwrap(), "closed-unmerged");
    }

    #[test]
    fn report_rides_the_envelope_json() {
        let r = into_result("pr-watch", report(Outcome::Ejected));
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["pr"]["outcome"], "ejected");
        assert_eq!(v["pr"]["pr"], 731);
        assert_eq!(v["pr"]["events"][0]["kind"], "phase");
        assert_eq!(v["ok"], false);
    }

    #[test]
    fn absent_report_is_omitted_from_json() {
        let r = crate::result::CommandResult::new("check");
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("pr").is_none(), "no `pr` key when the command has no report");
    }
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::` Expected: FAIL — module
`pr` does not exist. **Observed:** 22 unresolved-name errors.

- [x] **Step 3: Implement against the tests**

Add `pub mod pr;` to `xtask/src/lib.rs` beside the existing flat `mod`
declarations (`:5-21`). Write the types above in `xtask/src/pr/mod.rs`. Add to
`CommandResult` in `xtask/src/result.rs`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub pr: Option<crate::pr::PrReport>,
```

initialised `None` in `CommandResult::new`. **Do not modify `exit_code()` or
`push()`** — with exactly one pushed step whose `ok` is `outcome.is_merged()`,
`push()`'s recomputation gives `ok == merged` and the existing binary
`exit_code()` is already correct (D5).

`into_result` builds `CommandResult::new(command)`, sets `pr = Some(report)`,
and pushes exactly one step: `StepResult::ok(command)` when
`report.outcome.is_merged()`, else `StepResult::fail(command)`, with
`.detail(<serialized outcome>)`.

In `print_human()`, add a `PrReport` arm beside the existing `audit`/`traces`
informational payloads (`result.rs:139-146`) printing outcome, PR + head SHA,
and `pointer` when present. Elapsed time comes from the envelope's
`duration_ms`, which Task 7's `finalize()` call populates.

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::` Expected: PASS (6 tests)
— **observed 6 passed.**

- [x] **Step 5: Commit** — `85aabd6a`, full `cargo xtask check` green.

```bash
git add xtask/src/pr/mod.rs xtask/src/result.rs xtask/src/lib.rs
git commit -m "feat(xtask): PR watch outcome + report types on the result envelope (#729)"
```

---

### Task 2: `gh.rs` — transport and failure classification

**Files:**

- Create: `xtask/src/pr/gh.rs`
- Modify: `xtask/src/pr/mod.rs` (add `pub mod gh;`)
- Test: in-file `#[cfg(test)]` in `xtask/src/pr/gh.rs`

**Interfaces:**

- Consumes: nothing from earlier tasks (deliberately standalone).
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    GhMissing,
    Unauthenticated,
    NotFound,
    RateLimited { reset_unix: Option<u64> },
    Transport(String),
    Malformed(String),
    GraphQlErrors(String),
}

impl ApiError {
    /// Transient failures are absorbed up to the strike limit (D7). Rate-limiting is
    /// NOT transient — D7 waits for the reset instead of spending a strike.
    pub fn is_transient(&self) -> bool;
    pub fn detail(&self) -> String;
}

/// Pure — no subprocess. THIS is what makes transport failures testable offline.
pub fn classify(exit: i32, stdout: &str, stderr: &str)
    -> Result<serde_json::Value, ApiError>;

/// Pure. `classify` cannot see the reset (gh does not print response headers), so
/// the reset is fetched separately and folded in here.
pub fn enrich_rate_limit(err: ApiError, reset: Option<u64>) -> ApiError;

/// JSON-producing call: `gh api …`. On `RateLimited`, enriches via `rate_limit_reset`.
pub fn run_gh(args: &[&str]) -> Result<serde_json::Value, ApiError>;

/// Non-JSON call (`gh pr merge …` prints a human sentence). Classifies only
/// spawn/transport/auth failure; never parses stdout.
pub fn run_gh_raw(args: &[&str]) -> Result<(), ApiError>;

/// `gh api rate_limit` — REST and unmetered, so it still answers while GraphQL is
/// limited. Returns the GraphQL reset epoch.
pub fn rate_limit_reset() -> Option<u64>;
```

- [x] **Step 1: Write the failing tests**

The 404 and GraphQL-schema specimens below are **real**, captured live from this
repo (spec F5). The 401/403/502 bodies are **synthesized** from GitHub's
documented error shape — noted so a later reader does not mistake them for
evidence.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_parses_the_body() {
        let v = classify(0, r#"{"number":731}"#, "").unwrap();
        assert_eq!(v["number"], 731);
    }

    #[test]
    fn rest_404_body_classifies_as_not_found() {
        // Captured verbatim from `gh api /repos/jaunder-org/jaunder/pulls/999999`.
        let out = r#"{"message":"Not Found","documentation_url":"https://docs.github.com/rest/pulls/pulls#get-a-pull-request","status":"404"}"#;
        assert_eq!(classify(1, out, "gh: Not Found (HTTP 404)").unwrap_err(),
                   ApiError::NotFound);
    }

    #[test]
    fn graphql_schema_error_classifies_as_graphql_errors() {
        // Captured verbatim: gh writes this to stderr with empty stdout.
        let err = "gh: Field 'nosuchfield' doesn't exist on type 'Repository'\n";
        match classify(1, "", err).unwrap_err() {
            ApiError::GraphQlErrors(m) => assert!(m.contains("nosuchfield")),
            other => panic!("expected GraphQlErrors, got {other:?}"),
        }
    }

    #[test]
    fn graphql_errors_array_on_exit_zero_is_still_an_error() {
        // A 200 response carrying `errors[]` must NOT read as success.
        let out = r#"{"data":null,"errors":[{"message":"Something went wrong"}]}"#;
        match classify(0, out, "").unwrap_err() {
            ApiError::GraphQlErrors(m) => assert!(m.contains("Something went wrong")),
            other => panic!("expected GraphQlErrors, got {other:?}"),
        }
    }

    #[test]
    fn rate_limit_body_classifies_without_a_reset() {
        // gh does not print response headers, so `classify` can never see the reset.
        let out = r#"{"message":"API rate limit exceeded for user ID 1.","status":"403"}"#;
        assert_eq!(classify(1, out, "").unwrap_err(),
                   ApiError::RateLimited { reset_unix: None });
    }

    #[test]
    fn secondary_rate_limit_also_classifies_as_rate_limited() {
        let out = r#"{"message":"You have exceeded a secondary rate limit.","status":"403"}"#;
        assert_eq!(classify(1, out, "").unwrap_err(),
                   ApiError::RateLimited { reset_unix: None });
    }

    #[test]
    fn a_non_rate_limit_403_is_transport_not_rate_limited() {
        // The 403 split rule: only a rate-limit *message* means rate-limited.
        let out = r#"{"message":"Resource not accessible by integration","status":"403"}"#;
        assert!(matches!(classify(1, out, "").unwrap_err(), ApiError::Transport(_)));
    }

    #[test]
    fn enrich_fills_the_reset_only_for_rate_limits() {
        assert_eq!(enrich_rate_limit(ApiError::RateLimited { reset_unix: None }, Some(600)),
                   ApiError::RateLimited { reset_unix: Some(600) });
        assert_eq!(enrich_rate_limit(ApiError::NotFound, Some(600)), ApiError::NotFound);
    }

    #[test]
    fn auth_failure_classifies_as_unauthenticated() {
        let out = r#"{"message":"Bad credentials","status":"401"}"#;
        assert_eq!(classify(1, out, "").unwrap_err(), ApiError::Unauthenticated);
    }

    #[test]
    fn missing_binary_classifies_as_gh_missing() {
        assert_eq!(classify(127, "", "gh: command not found").unwrap_err(),
                   ApiError::GhMissing);
    }

    #[test]
    fn server_error_is_transport_and_transient() {
        let out = r#"{"message":"Server Error","status":"502"}"#;
        let e = classify(1, out, "").unwrap_err();
        assert!(matches!(e, ApiError::Transport(_)));
        assert!(e.is_transient());
    }

    #[test]
    fn exit_one_with_no_body_and_no_gh_prefix_is_transport() {
        // The empty-stdout split rule: `gh: …` on stderr means gh spoke (GraphQL);
        // anything else is the transport failing underneath it.
        assert!(matches!(classify(1, "", "connection reset by peer").unwrap_err(),
                         ApiError::Transport(_)));
    }

    #[test]
    fn unparseable_success_body_is_malformed() {
        assert!(matches!(classify(0, "not json", "").unwrap_err(), ApiError::Malformed(_)));
    }

    #[test]
    fn transience_partitions_the_variants() {
        assert!(!ApiError::RateLimited { reset_unix: None }.is_transient());
        assert!(!ApiError::GhMissing.is_transient());
        assert!(!ApiError::Unauthenticated.is_transient());
        assert!(!ApiError::NotFound.is_transient());
        assert!(ApiError::Transport("x".into()).is_transient());
        assert!(ApiError::Malformed("x".into()).is_transient());
    }
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::gh` Expected: FAIL —
`classify` / `ApiError` not defined. **Observed:** 36 compile errors.

- [x] **Step 3: Implement against the tests**

Write `classify` and `enrich_rate_limit` to the signatures above. Every branch
is pinned by a test, so the tests determine the bodies. Three ordering rules the
tests encode and the implementation must respect:

1. **Check the GraphQL `errors[]` array even on exit 0** — the
   errors-with-exit-0 test fails otherwise, and this is the "the tool lies" case
   the issue names.
2. **Classify from the body's `status` field first, stderr second** — per F5 the
   exit code carries no information and stderr text is the least stable of the
   three. Within `403`, the message decides: a rate-limit phrase →
   `RateLimited`, anything else → `Transport`.
3. **With empty stdout**, a stderr beginning `gh: ` means gh itself reported a
   GraphQL error → `GraphQlErrors`; otherwise → `Transport`.

`run_gh` / `run_gh_raw` use **`std::process::Command`** directly (not `xshell`,
whose `Error` does not expose the underlying `io::ErrorKind` — see
`xtask/src/sh.rs:27`), so a spawn failure with `ErrorKind::NotFound` maps to
`ApiError::GhMissing` without invoking `classify`. `run_gh` captures stdout,
stderr, and the exit code separately and hands all three to `classify`; on
`RateLimited` it calls `rate_limit_reset()` and returns
`enrich_rate_limit(err, reset)`. `run_gh_raw` never parses stdout — this is what
lets `gh pr merge` (which prints a human sentence) succeed.

`rate_limit_reset` runs `gh api rate_limit` and reads
`.resources.graphql.reset`, returning `None` on any failure — it must never
itself become a source of error.

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::gh` Expected: PASS (14
tests) — **observed 14 passed.**

- [x] **Step 5: Commit** — `d85a3ae3`, full `cargo xtask check` green.

```bash
git add xtask/src/pr/gh.rs xtask/src/pr/mod.rs
git commit -m "feat(xtask): typed gh transport + failure classification for pr watch (#729)"
```

---

### Task 3: `snapshot.rs` — types, captured fixtures, parsing

**Files:**

- Create: `xtask/src/pr/snapshot.rs`
- Create: `xtask/src/pr/testdata/pr-merged.json`, `pr-queued.json`,
  `pr-open-green.json`, `rules-queue.json`, `rules-strict.json`,
  `runs-merge-group.json`
- Modify: `xtask/src/pr/mod.rs` (add `pub mod snapshot;`)
- Test: in-file `#[cfg(test)]` in `xtask/src/pr/snapshot.rs`

**Interfaces:**

- Consumes: `ApiError` (Task 2); `Subject`, `PrNumber` (Task 1).
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Eq)] pub enum PrState { Open, Merged, Closed }
#[derive(Debug, Clone, PartialEq, Eq)] pub enum Mergeable { Mergeable, Conflicting, Unknown }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStateStatus { Behind, Blocked, Clean, Dirty, Draft, HasHooks, Unknown, Unstable }
#[derive(Debug, Clone, PartialEq, Eq)] pub enum CheckState { Pending, Success, Failure }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckEntry {
    pub name: String, pub state: CheckState,
    pub details_url: Option<String>,
    pub started_at: Option<String>, pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueState { pub in_queue: bool, pub position: Option<u64> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSnapshot {
    pub state: PrState,
    pub merged_at: Option<String>,
    pub merge_commit: Option<String>,
    pub mergeable: Mergeable,
    pub merge_state_status: MergeStateStatus,
    pub auto_merge_armed: bool,
    pub queue: QueueState,
    pub head_sha: String,
    pub head_committed_at: String,
    pub checks: Vec<CheckEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredChecks {
    pub contexts: Vec<String>,
    pub strict: bool,
    pub queue_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRef { pub url: String, pub created_at: String, pub conclusion: String }

pub const PR_QUERY: &str = "...";   // the single GraphQL document (F4)

pub fn parse_snapshot(v: &serde_json::Value) -> Result<PrSnapshot, ApiError>;
pub fn parse_required_checks(v: &serde_json::Value) -> Result<RequiredChecks, ApiError>;
pub fn parse_ejection_run(v: &serde_json::Value, pr: PrNumber) -> Option<RunRef>;

pub trait PrSource {
    fn resolve(&self, requested: Option<PrNumber>) -> Result<Subject, ApiError>;
    fn snapshot(&self, subject: &Subject) -> Result<PrSnapshot, ApiError>;
    fn required_checks(&self, subject: &Subject) -> Result<RequiredChecks, ApiError>;
    fn ejection_run(&self, subject: &Subject) -> Result<Option<RunRef>, ApiError>;
}

pub struct GhSource;   // the real impl; `resolve` lands in Task 7
```

- [x] **Step 1: Write `PR_QUERY` first** — validated live against PR 727;
      `mergeStateStatus` needs no preview header. `headRefName` was added beyond
      the listed fields, because the Task 6 divergence guard needs the PR's head
      ref and nothing else supplies it.

The capture in Step 2 runs this document, so it must exist before the fixtures
do. Write `PR_QUERY` as one GraphQL document requesting, per F4:

- `state`, `mergedAt`, `mergeCommit { oid }`, `mergeable`, `mergeStateStatus`
- `isInMergeQueue`, `mergeQueueEntry { position }`,
  `autoMergeRequest { enabledAt }`
- `commits(last:1) { nodes { commit { oid committedDate } } }` — the source of
  `head_sha` and `head_committed_at`
- `statusCheckRollup { contexts(first:100) { nodes { ... on CheckRun { name conclusion status detailsUrl startedAt completedAt } ... on StatusContext { context state targetUrl createdAt } } } }`

- [x] **Step 2: Capture the fixtures** (the only live-network step in the whole
      plan)

```bash
mkdir -p xtask/src/pr/testdata
gh api /repos/jaunder-org/jaunder/rules/branches/main > xtask/src/pr/testdata/rules-queue.json
gh api "/repos/jaunder-org/jaunder/actions/runs?event=merge_group&per_page=100" > xtask/src/pr/testdata/runs-merge-group.json
```

Run `PR_QUERY` against merged PR **727** → `pr-merged.json`.

`runs-merge-group.json` may be trimmed for readability, but **must retain all
three `gh-readonly-queue/main/pr-646-…` runs** — verified live, PR 646 is the
only subject with multiple merge-group runs, so it is what makes the
most-recent-wins test discriminating:

| `created_at`           | `conclusion` | base SHA suffix |
| ---------------------- | ------------ | --------------- |
| `2026-07-24T19:31:52Z` | `success`    | `158968…`       |
| `2026-07-24T18:22:35Z` | `failure`    | `97f75b…`       |
| `2026-07-24T18:00:58Z` | `failure`    | `97f75b…`       |

The newest is a **success** while the older two failed — which is exactly why
"pick the newest" has to be right: picking the first match would report a
failure that a later run superseded. PR 727 has exactly one run
(`conclusion: success`), which is what the prefix-match test uses.

`pr-queued.json`, `pr-open-green.json`, and `rules-strict.json` are
**synthesized** by editing a capture — no live PR is guaranteed to be in the
right state at authoring time:

- `pr-queued.json`: from `pr-merged.json`, set `state: OPEN`,
  `isInMergeQueue: true`, **and `mergeQueueEntry: { "position": 2 }`** (the
  position is what `queued_pr_snapshot_carries_queue_position` asserts —
  omitting it fails the test), and set every rollup context to a success
  conclusion.
- `pr-open-green.json`: from `pr-merged.json`, set `state: OPEN`,
  `isInMergeQueue: false`, `mergeQueueEntry: null`, all rollup contexts
  successful.
- `rules-strict.json`: from `rules-queue.json`, set
  `"strict_required_status_checks_policy": true` and remove the `merge_queue`
  rule — i.e. the documented ADR-0077 rollback state (D6).

Put a comment at the head of the test module naming **all three** as
synthesized, so a later reader does not treat a hand-edit as evidence about
GitHub's shape.

- [x] **Step 3: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    //! Fixtures: `rules-queue.json`, `runs-merge-group.json`, and `pr-merged.json` are
    //! captured live. `pr-queued.json`, `pr-open-green.json`, and `rules-strict.json`
    //! are SYNTHESIZED by editing a capture — they are not evidence about GitHub's
    //! response shape, only about our parsing of it.
    use super::*;

    macro_rules! fixture {
        ($name:literal) => {
            serde_json::from_str::<serde_json::Value>(include_str!(concat!("testdata/", $name)))
                .expect("fixture parses")
        };
    }

    #[test]
    fn required_checks_come_from_the_ruleset_not_a_hardcoded_list() {
        let rc = parse_required_checks(&fixture!("rules-queue.json")).unwrap();
        assert_eq!(rc.contexts, vec!["Validate (no e2e)", "e2e gate"]);
        assert!(!rc.strict, "live ruleset is non-strict (spec F1)");
        assert!(rc.queue_present, "live ruleset has a merge_queue rule (spec F1)");
    }

    #[test]
    fn strict_rollback_ruleset_parses_as_strict_without_a_queue() {
        let rc = parse_required_checks(&fixture!("rules-strict.json")).unwrap();
        assert!(rc.strict);
        assert!(!rc.queue_present);
    }

    #[test]
    fn merged_pr_snapshot_carries_commit_and_timestamp() {
        let s = parse_snapshot(&fixture!("pr-merged.json")).unwrap();
        assert_eq!(s.state, PrState::Merged);
        assert!(s.merge_commit.is_some());
        assert!(s.merged_at.is_some());
    }

    #[test]
    fn queued_pr_snapshot_carries_queue_position() {
        let s = parse_snapshot(&fixture!("pr-queued.json")).unwrap();
        assert_eq!(s.state, PrState::Open);
        assert!(s.queue.in_queue);
        assert_eq!(s.queue.position, Some(2));
    }

    #[test]
    fn checks_flatten_both_union_members() {
        // statusCheckRollup.contexts mixes CheckRun (name/conclusion) and
        // StatusContext (context/state) — both must land in `checks` (D6).
        let s = parse_snapshot(&fixture!("pr-open-green.json")).unwrap();
        assert!(s.checks.iter().any(|c| c.name == "Validate (no e2e)"));
        assert!(s.checks.iter().all(|c| !c.name.is_empty()));
    }

    #[test]
    fn head_committed_at_is_populated() {
        // Load-bearing for D10's ejection discriminator.
        let s = parse_snapshot(&fixture!("pr-open-green.json")).unwrap();
        assert!(!s.head_committed_at.is_empty());
        assert!(!s.head_sha.is_empty());
    }

    #[test]
    fn ejection_run_matches_on_branch_prefix_not_exact_name() {
        // Spec F11: the suffix is the BASE sha, so only a prefix test can match.
        assert!(parse_ejection_run(&fixture!("runs-merge-group.json"), PrNumber(727)).is_some());
    }

    #[test]
    fn ejection_run_ignores_other_prs() {
        assert!(parse_ejection_run(&fixture!("runs-merge-group.json"), PrNumber(999)).is_none());
    }

    #[test]
    fn ejection_run_picks_the_most_recent_by_created_at() {
        // PR 646 is the only subject with multiple merge-group runs, which is why the
        // fixture must retain all of them (Step 2).
        let all = fixture!("runs-merge-group.json");
        let newest = all["workflow_runs"].as_array().unwrap().iter()
            .filter(|r| r["head_branch"].as_str().unwrap()
                        .starts_with("gh-readonly-queue/main/pr-646-"))
            .map(|r| r["created_at"].as_str().unwrap().to_string())
            .max().expect("fixture must retain the pr-646 runs");
        let picked = parse_ejection_run(&all, PrNumber(646)).unwrap();
        assert_eq!(picked.created_at, newest, "must pick the newest, not the first");
    }

    #[test]
    fn ejection_run_reports_conclusion_verbatim_without_judging_it() {
        // decide.rs judges failure/recency (D3: no logic here). PR 727's run succeeded.
        let picked = parse_ejection_run(&fixture!("runs-merge-group.json"), PrNumber(727)).unwrap();
        assert_eq!(picked.conclusion, "success");
    }

    #[test]
    fn malformed_payload_is_an_api_error_not_a_panic() {
        let bad = serde_json::json!({ "data": { "repository": null } });
        assert!(matches!(parse_snapshot(&bad), Err(ApiError::Malformed(_))));
    }
}
```

- [x] **Step 4: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::snapshot` Expected: FAIL —
`parse_snapshot` / `parse_required_checks` not defined. **Observed** exactly
that.

- [x] **Step 5: Implement against the tests** — the fieldless state enums are
      `Copy` (a first pass without it hit a move-after-use in `parse_check`).

Write the three parse functions to the signatures above. Every branch is pinned
by the tests, so the tests determine the bodies. Two rules they encode:

- `parse_ejection_run` matches `head_branch` with
  `starts_with(&format!("gh-readonly-queue/main/pr-{n}-"))` (F11 — `?branch=`
  needs an exact name and cannot be used), returns the newest by `created_at`,
  and carries `html_url`, `created_at`, and `conclusion` **unfiltered** — the
  failure/recency judgment is `decide.rs`'s.
- A missing or null `repository`/`pullRequest` node yields
  `ApiError::Malformed`, never a panic and never a defaulted-empty snapshot.

Declare the `PrSource` trait and a unit `GhSource` implementing `snapshot` /
`required_checks` / `ejection_run` over `gh::run_gh` + the parsers. Leave
`resolve` as `unimplemented!("Task 7")` — **no test calls it yet**, and Task 7
replaces it.

- [x] **Step 6: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::snapshot` Expected: PASS
(11 tests) — **observed 11 passed.**

- [x] **Step 7: Commit** — `5099098d`, full `cargo xtask check` green.

```bash
git add xtask/src/pr/snapshot.rs xtask/src/pr/testdata xtask/src/pr/mod.rs
git commit -m "feat(xtask): typed PR snapshot + ruleset parsing from captured fixtures (#729)"
```

---

### Task 4: Shared test fixtures and `decide.rs` — the pure state machine

**Files:**

- Create: `xtask/src/pr/test_support.rs` — **the shared fixture home**. A
  `#[cfg(test)] mod tests` is private to its own file, so every fake and builder
  used by more than one module lives here, mirroring the existing
  `xtask/src/test_support.rs` idiom (`lib.rs:18-19`).
- Create: `xtask/src/pr/decide.rs`
- Modify: `xtask/src/pr/mod.rs` (add `pub mod decide;` and
  `#[cfg(test)] pub(crate) mod test_support;`)
- Test: in-file `#[cfg(test)]` in `xtask/src/pr/decide.rs`

**Interfaces:**

- Consumes: `PrSnapshot`, `RequiredChecks`, `RunRef`, `CheckEntry`,
  `CheckState`, `PrState`, `Mergeable`, `MergeStateStatus`, `QueueState` (Task
  3); `Outcome`, `Subject`, `PrNumber` (Task 1).
- Produces — `test_support` (Tasks 5 and 6 both depend on these exact builders):

```rust
// xtask/src/pr/test_support.rs  —  #[cfg(test)] only
pub fn queue_rules() -> RequiredChecks;   // 2 contexts, strict: false, queue_present: true
pub fn strict_rules() -> RequiredChecks;  // same contexts, strict: true, queue_present: false
pub fn check(name: &str, state: CheckState, completed: &str) -> CheckEntry;
pub fn green() -> Vec<CheckEntry>;        // both required contexts successful
pub fn open(checks: Vec<CheckEntry>) -> PrSnapshot;   // OPEN, mergeable, unarmed, unqueued
pub fn open_pending() -> PrSnapshot;      // open() with both required contexts Pending
pub fn merged_snapshot() -> PrSnapshot;   // state: Merged, merge_commit: Some, merged_at: Some
pub fn armed_snapshot() -> PrSnapshot;    // open(green()) + auto_merge_armed: true, in_queue: false
pub fn queued_at(position: u64) -> PrSnapshot;  // open(green()) + in_queue: true,
                                                // position: Some(n), auto_merge_armed: FALSE
pub fn subject() -> Subject;              // owner/repo + PrNumber(731)
pub fn ejection(created_at: &str) -> RunRef;    // conclusion: "failure"
```

`armed_snapshot` and `queued_at` must differ exactly as annotated —
`armed_snapshot` sets `auto_merge_armed` and **not** `in_queue`; `queued_at`
sets `in_queue` and **not** `auto_merge_armed`. That difference is the whole
point of Task 6's direct-enqueue test. `open()` fixes `head_committed_at` at
`"2026-07-30T13:00:00Z"` so ejection-recency tests have a stable anchor.

- Produces — `decide.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase { AwaitingChecks, Armed, Queued, Terminal }

impl Phase { pub fn as_str(self) -> &'static str }  // kebab-case; the event detail

#[derive(Debug, Clone, Default)]
pub struct Progress { pub was_queued: bool, pub last_fingerprint: Option<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Continue { phase: Phase, warn: Option<String> },
    Terminal { outcome: Outcome, detail: Option<String>, pointer: Option<String> },
}

/// Resolve a required context name against the rollup (D6): exact match on
/// CheckRun.name or StatusContext.context; latest `completed_at` wins on duplicates.
pub fn resolve_context<'a>(checks: &'a [CheckEntry], name: &str) -> Option<&'a CheckEntry>;

/// Whether the ejection-run query should be issued for this snapshot (D10).
pub fn needs_ejection_probe(snap: &PrSnapshot, req: &RequiredChecks) -> bool;

/// THE state machine. Pure: no IO, no clock.
pub fn classify(snap: &PrSnapshot, req: &RequiredChecks,
                ejection: Option<&RunRef>, progress: &Progress) -> Step;
```

- [x] **Step 1: Write `test_support.rs`** — `subject()` deferred to Task 5,
      where it is first used: an unused helper would trip `-D warnings` at this
      commit boundary.

Write every builder above. This file has no tests of its own; it is exercised by
Tasks 4–6. Declare it `#[cfg(test)] pub(crate) mod test_support;` in `pr/mod.rs`
so it compiles only under `cargo test`.

- [x] **Step 2: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::test_support::*;

    // ---- terminal outcomes ----

    #[test]
    fn merged_pr_is_terminal_merged_with_the_commit() {
        match classify(&merged_snapshot(), &queue_rules(), None, &Progress::default()) {
            Step::Terminal { outcome, pointer, .. } => {
                assert_eq!(outcome, Outcome::Merged);
                assert!(pointer.is_some(), "merged must carry the merge commit");
            }
            other => panic!("expected merged, got {other:?}"),
        }
    }

    #[test]
    fn failing_required_check_is_checks_failed_with_its_url() {
        let s = open(vec![
            check("Validate (no e2e)", CheckState::Failure, "2026-07-30T14:10:00Z"),
            check("e2e gate", CheckState::Pending, ""),
        ]);
        match classify(&s, &queue_rules(), None, &Progress::default()) {
            Step::Terminal { outcome, pointer, detail } => {
                assert_eq!(outcome, Outcome::ChecksFailed);
                assert!(pointer.is_some(), "must point at the failing job log");
                assert!(detail.unwrap().contains("Validate (no e2e)"));
            }
            other => panic!("expected checks-failed, got {other:?}"),
        }
    }

    #[test]
    fn conflicting_pr_is_terminal_conflicted() {
        let mut s = open(green());
        s.mergeable = Mergeable::Conflicting;
        assert!(matches!(classify(&s, &queue_rules(), None, &Progress::default()),
                         Step::Terminal { outcome: Outcome::Conflicted, .. }));
    }

    #[test]
    fn closed_unmerged_pr_is_terminal() {
        let mut s = open(green());
        s.state = PrState::Closed;
        assert!(matches!(classify(&s, &queue_rules(), None, &Progress::default()),
                         Step::Terminal { outcome: Outcome::ClosedUnmerged, .. }));
    }

    #[test]
    fn conflict_outranks_a_failed_check() {
        // Ordering rule: report the condition a human must act on first.
        let mut s = open(vec![check("Validate (no e2e)", CheckState::Failure,
                                    "2026-07-30T14:10:00Z")]);
        s.mergeable = Mergeable::Conflicting;
        assert!(matches!(classify(&s, &queue_rules(), None, &Progress::default()),
                         Step::Terminal { outcome: Outcome::Conflicted, .. }));
    }

    // ---- the six traps ----

    #[test]
    fn all_required_green_is_not_terminal_when_a_queue_exists() {
        // The issue's point 2: declaring victory at green checks is wrong.
        match classify(&open(green()), &queue_rules(), None, &Progress::default()) {
            Step::Continue { .. } => {}
            other => panic!("green checks must not be terminal under a queue: {other:?}"),
        }
    }

    #[test]
    fn absent_required_context_is_not_terminal() {
        // The issue's point 5: `e2e gate` appears late; "no check pending" is briefly
        // true before it exists.
        let s = open(vec![check("Validate (no e2e)", CheckState::Success,
                                "2026-07-30T14:10:00Z")]);
        assert!(matches!(classify(&s, &queue_rules(), None, &Progress::default()),
                         Step::Continue { .. }));
    }

    #[test]
    fn failing_non_required_check_does_not_fail_the_pr() {
        let mut checks = green();
        checks.push(check("some-optional-lint", CheckState::Failure, "2026-07-30T14:05:00Z"));
        assert!(matches!(classify(&open(checks), &queue_rules(), None, &Progress::default()),
                         Step::Continue { .. }));
    }

    #[test]
    fn duplicate_context_resolves_to_the_latest_completion() {
        // A red original followed by a green re-run must read green.
        let checks = vec![
            check("Validate (no e2e)", CheckState::Failure, "2026-07-30T14:10:00Z"),
            check("Validate (no e2e)", CheckState::Success, "2026-07-30T14:40:00Z"),
            check("e2e gate", CheckState::Success, "2026-07-30T14:20:00Z"),
        ];
        assert_eq!(resolve_context(&checks, "Validate (no e2e)").unwrap().state,
                   CheckState::Success);
        assert!(matches!(classify(&open(checks), &queue_rules(), None, &Progress::default()),
                         Step::Continue { .. }));
    }

    #[test]
    fn behind_is_stale_only_when_the_ruleset_is_strict() {
        let mut s = open(green());
        s.merge_state_status = MergeStateStatus::Behind;
        // Strict (the ADR-0077 rollback state): terminal.
        assert!(matches!(classify(&s, &strict_rules(), None, &Progress::default()),
                         Step::Terminal { outcome: Outcome::Stale, .. }));
        // Non-strict + queue (live): not terminal.
        assert!(matches!(classify(&s, &queue_rules(), None, &Progress::default()),
                         Step::Continue { .. }));
    }

    // ---- ejection ----

    #[test]
    fn failed_merge_group_run_newer_than_head_is_ejected_without_history() {
        // Reachable with NO prior observation of the queue entry (A6, `--once`).
        match classify(&open(green()), &queue_rules(),
                       Some(&ejection("2026-07-30T14:30:00Z")), &Progress::default()) {
            Step::Terminal { outcome, pointer, .. } => {
                assert_eq!(outcome, Outcome::Ejected);
                assert!(pointer.unwrap().contains("/actions/runs/"));
            }
            other => panic!("expected ejected, got {other:?}"),
        }
    }

    #[test]
    fn stale_merge_group_run_older_than_head_is_not_ejected() {
        // The false-`ejected`-on-a-freshly-pushed-head guard (A6 mirror case).
        let mut s = open(green());
        s.head_committed_at = "2026-07-30T15:00:00Z".into();
        assert!(matches!(
            classify(&s, &queue_rules(), Some(&ejection("2026-07-30T14:30:00Z")),
                     &Progress::default()),
            Step::Continue { .. }));
    }

    #[test]
    fn successful_merge_group_run_is_not_an_ejection() {
        let mut run = ejection("2026-07-30T14:30:00Z");
        run.conclusion = "success".into();
        assert!(matches!(classify(&open(green()), &queue_rules(), Some(&run),
                                  &Progress::default()),
                         Step::Continue { .. }));
    }

    #[test]
    fn vanished_queue_entry_with_no_run_warns_and_continues() {
        // Manual dequeue: fold back into the loop, loudly (D10).
        let progress = Progress { was_queued: true, last_fingerprint: None };
        match classify(&open(green()), &queue_rules(), None, &progress) {
            Step::Continue { warn, .. } => assert!(warn.unwrap().contains("found none")),
            other => panic!("expected a warning continue, got {other:?}"),
        }
    }

    #[test]
    fn green_and_unqueued_warns_that_nothing_will_happen() {
        match classify(&open(green()), &queue_rules(), None, &Progress::default()) {
            Step::Continue { warn, phase } => {
                assert_eq!(phase, Phase::AwaitingChecks);
                assert!(warn.unwrap().contains("pr land"));
            }
            other => panic!("expected the unarmed warning, got {other:?}"),
        }
    }

    // ---- phases & the probe trigger ----

    #[test]
    fn queued_pr_reports_the_queued_phase() {
        assert!(matches!(classify(&queued_at(2), &queue_rules(), None, &Progress::default()),
                         Step::Continue { phase: Phase::Queued, .. }));
    }

    #[test]
    fn armed_pr_reports_the_armed_phase() {
        assert!(matches!(classify(&armed_snapshot(), &queue_rules(), None, &Progress::default()),
                         Step::Continue { phase: Phase::Armed, .. }));
    }

    #[test]
    fn ejection_probe_fires_only_when_green_open_and_unqueued() {
        // D10's cost control: never during the long pre-green phase.
        assert!(needs_ejection_probe(&open(green()), &queue_rules()));
        assert!(!needs_ejection_probe(&open_pending(), &queue_rules()));
        assert!(!needs_ejection_probe(&queued_at(1), &queue_rules()));
        assert!(!needs_ejection_probe(&merged_snapshot(), &queue_rules()));
    }

    #[test]
    fn no_check_name_is_hardcoded() {
        // A8: rename both contexts and the machine must follow the ruleset.
        let req = RequiredChecks { contexts: vec!["Alpha".into()], strict: false,
                                   queue_present: true };
        let s = open(vec![check("Alpha", CheckState::Failure, "2026-07-30T14:10:00Z")]);
        assert!(matches!(classify(&s, &req, None, &Progress::default()),
                         Step::Terminal { outcome: Outcome::ChecksFailed, .. }));
    }
}
```

- [x] **Step 3: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::decide` Expected: FAIL —
`classify` / `Step` / `Phase` not defined. **Observed** exactly that.

- [x] **Step 4: Implement against the tests** — `resolve_context` additionally
      resolves an **in-flight** duplicate over a completed one (a re-run in
      progress means the context is unsettled), which the plan left to the
      implementer.

Write `resolve_context`, `needs_ejection_probe`, and `classify` to the
signatures above. Every branch is pinned by a test, so the tests determine the
bodies. The one ordering rule they encode — **evaluate terminal states before
phase states, in this order**: `Merged` → `ClosedUnmerged` → `Conflicted` →
`ChecksFailed` → `Stale` (strict only) → `Ejected` → then phases.
`conflict_outranks_a_failed_check` pins the one case where the order is
observable.

- [x] **Step 5: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::decide` Expected: PASS (19
tests) — **observed 19 passed.**

- [x] **Step 6: Commit** — `301cf804`, full `cargo xtask check` green.

```bash
git add xtask/src/pr/decide.rs xtask/src/pr/test_support.rs xtask/src/pr/mod.rs
git commit -m "feat(xtask): pure PR outcome state machine + shared test fixtures (#729)"
```

---

### Task 5: `watch.rs` — the poll loop

**Files:**

- Create: `xtask/src/pr/watch.rs`
- Modify: `xtask/src/pr/test_support.rs` (add `FakeClock`, `FakeSource`,
  `clock()`, `cfg()` — Task 6 reuses all four)
- Modify: `xtask/src/pr/mod.rs` (add `pub mod watch;`)
- Test: in-file `#[cfg(test)]` in `xtask/src/pr/watch.rs`

**Interfaces:**

- Consumes: `PrSource`, `PrSnapshot`, `RequiredChecks`, `RunRef` (Task 3);
  `classify`, `Step`, `Phase`, `Progress`, `needs_ejection_probe` (Task 4);
  `PrReport`, `Event`, `EventKind`, `Outcome`, `Subject` (Task 1); `ApiError`
  (Task 2); the Task 4 builders.
- Produces:

```rust
pub trait Clock {
    fn now_unix(&self) -> u64;
    fn now_rfc3339(&self) -> String;
    fn sleep_secs(&self, secs: u64);
}

pub struct SystemClock;

#[derive(Debug, Clone, Copy)]
pub struct WatchConfig {
    pub interval_secs: u64,      // default 30
    pub timeout_mins: u64,       // default 90
    pub once: bool,
    pub heartbeat_secs: u64,     // 600
    pub max_strikes: u32,        // 5
}
impl Default for WatchConfig { /* the D7 defaults */ }

/// Never returns Err — every terminal path is a report (D5/A3). The `Result`-free
/// return type is what makes that criterion structural rather than tested.
pub fn watch<S: PrSource, C: Clock>(
    source: &S, clock: &C, subject: &Subject, cfg: WatchConfig,
    sink: &mut dyn FnMut(&Event),
) -> PrReport;

/// The change-detection fingerprint (D8). `pub(crate)` for testing.
pub(crate) fn fingerprint(snap: &PrSnapshot, req: &RequiredChecks, phase: Phase) -> String;
```

- Produces — added to `test_support.rs`:

```rust
/// Virtual clock — a 90-minute timeout test must run in microseconds.
pub struct FakeClock { pub now: std::cell::RefCell<u64> }
impl Clock for FakeClock { /* sleep advances `now` instead of blocking */ }
pub fn clock() -> FakeClock;                 // starts at 0
pub fn cfg() -> WatchConfig;                 // WatchConfig::default()

/// Scripted source. Each `snapshot` call pops the next scripted result.
/// **When the script is exhausted it returns the LAST scripted value forever** —
/// so a budget-expiry test can script one snapshot and still poll to the timeout.
pub struct FakeSource {
    pub snaps: std::cell::RefCell<std::collections::VecDeque<Result<PrSnapshot, ApiError>>>,
    pub last: std::cell::RefCell<Option<Result<PrSnapshot, ApiError>>>,
    pub req: RequiredChecks,
    pub ejection: Option<RunRef>,
}
impl FakeSource { pub fn new(snaps: Vec<Result<PrSnapshot, ApiError>>, req: RequiredChecks) -> Self }
impl PrSource for FakeSource {
    // resolve: unreachable!("FakeSource is constructed with a Subject")
    // snapshot: pop-or-repeat-last; required_checks: clone `req`; ejection_run: clone `ejection`
}
```

- [x] **Step 1: Extend `test_support.rs`**

Write `FakeClock`, `FakeSource`, `clock()`, and `cfg()` as specified. The
pop-or-repeat-last semantics are load-bearing:
`budget_expiry_is_timed_out_not_watcher_error` depends on the script never
running dry.

- [x] **Step 2: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::test_support::*;

    // ---- A4: the failure mode that actually bit ----

    #[test]
    fn five_consecutive_api_failures_yield_watcher_error() {
        let src = FakeSource::new(
            (0..5).map(|_| Err(ApiError::Transport("boom".into()))).collect(),
            queue_rules());
        let mut seen: Vec<Event> = Vec::new();
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |e| seen.push(e.clone()));

        // Must fail if the loop returns silence, success, or a check verdict.
        assert_eq!(report.outcome, Outcome::WatcherError,
                   "sustained API failure must be watcher-error, not {:?}", report.outcome);
        assert_ne!(report.outcome, Outcome::Merged);
        assert_ne!(report.outcome, Outcome::ChecksFailed);
        assert_ne!(report.outcome, Outcome::TimedOut);
        assert!(!report.events.is_empty(), "silence must never be the answer");
        assert_eq!(report.events.iter().filter(|e| e.kind == EventKind::PollError).count(), 5,
                   "every absorbed failure is an event");
    }

    #[test]
    fn a_transient_failure_before_success_does_not_end_the_watch() {
        let src = FakeSource::new(
            vec![Err(ApiError::Transport("blip".into())), Ok(merged_snapshot())],
            queue_rules());
        assert_eq!(watch(&src, &clock(), &subject(), cfg(), &mut |_| {}).outcome,
                   Outcome::Merged);
    }

    // ---- A5: rate-limiting is not a strike ----

    #[test]
    fn rate_limit_inside_the_budget_waits_and_continues() {
        let c = clock();
        let src = FakeSource::new(
            vec![Err(ApiError::RateLimited { reset_unix: Some(600) }), Ok(merged_snapshot())],
            queue_rules());
        let report = watch(&src, &c, &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Merged);
        assert!(c.now_unix() >= 600, "must have waited for the reset");
        assert!(report.events.iter().any(|e| e.detail.contains("rate limited")));
    }

    #[test]
    fn rate_limit_beyond_the_budget_is_watcher_error_immediately() {
        let c = clock();
        let src = FakeSource::new(
            vec![Err(ApiError::RateLimited { reset_unix: Some(99_999) })], queue_rules());
        let report = watch(&src, &c, &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert!(c.now_unix() < 5_400, "must not burn the budget waiting");
    }

    // ---- A9: emit on change only ----

    #[test]
    fn identical_consecutive_snapshots_emit_exactly_one_phase_event() {
        let src = FakeSource::new(
            vec![Ok(open_pending()), Ok(open_pending()), Ok(open_pending()),
                 Ok(merged_snapshot())], queue_rules());
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        let phase_events = report.events.iter()
            .filter(|e| e.kind == EventKind::Phase && e.detail == "awaiting-checks").count();
        assert_eq!(phase_events, 1, "unchanged state must not re-emit per poll");
    }

    #[test]
    fn a_queue_position_change_emits_per_change() {
        // Poll 1 emits the full current state (Phase + Queue at position 3);
        // poll 2 emits only the changed component (Queue at position 2).
        let src = FakeSource::new(
            vec![Ok(queued_at(3)), Ok(queued_at(2)), Ok(merged_snapshot())], queue_rules());
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.events.iter().filter(|e| e.kind == EventKind::Queue).count(), 2);
    }

    #[test]
    fn fingerprint_ignores_non_required_checks_but_tracks_the_queue() {
        // Discriminating: a non-required check changing must NOT move the fingerprint,
        // while a queue position change must.
        let mut extra = queued_at(3);
        let base = fingerprint(&extra, &queue_rules(), Phase::Queued);
        extra.checks.push(check("optional-lint", CheckState::Failure, "2026-07-30T14:05:00Z"));
        assert_eq!(fingerprint(&extra, &queue_rules(), Phase::Queued), base,
                   "a non-required check must not move the fingerprint");
        assert_ne!(fingerprint(&queued_at(2), &queue_rules(), Phase::Queued), base,
                   "a queue position change must move the fingerprint");
    }

    #[test]
    fn ten_minutes_of_stasis_emits_one_heartbeat() {
        // Loop order is poll-then-sleep, so poll k happens at t = 30*(k-1);
        // poll 21 lands exactly on t=600, the heartbeat threshold.
        let mut snaps: Vec<_> = (0..21).map(|_| Ok(open_pending())).collect();
        snaps.push(Ok(merged_snapshot()));
        let src = FakeSource::new(snaps, queue_rules());
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.events.iter().filter(|e| e.kind == EventKind::Heartbeat).count(), 1);
    }

    // ---- budget & --once ----

    #[test]
    fn budget_expiry_is_timed_out_not_watcher_error() {
        // One scripted snapshot; FakeSource repeats it until the budget runs out.
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::TimedOut);
        assert_ne!(report.outcome, Outcome::WatcherError, "the tooling worked fine");
    }

    #[test]
    fn once_mode_returns_pending_without_looping() {
        let c = clock();
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());
        let mut config = cfg();
        config.once = true;
        let report = watch(&src, &c, &subject(), config, &mut |_| {});
        assert_eq!(report.outcome, Outcome::Pending);
        assert_eq!(report.phase.as_deref(), Some("awaiting-checks"));
        assert_eq!(c.now_unix(), 0, "--once must not sleep");
    }

    #[test]
    fn once_mode_reaches_a_terminal_outcome_when_one_exists() {
        let src = FakeSource::new(vec![Ok(merged_snapshot())], queue_rules());
        let mut config = cfg();
        config.once = true;
        assert_eq!(watch(&src, &clock(), &subject(), config, &mut |_| {}).outcome,
                   Outcome::Merged);
    }

    #[test]
    fn every_event_reaches_both_the_sink_and_the_report() {
        let src = FakeSource::new(vec![Ok(open_pending()), Ok(merged_snapshot())],
                                  queue_rules());
        let mut seen: Vec<Event> = Vec::new();
        let report = watch(&src, &clock(), &subject(), cfg(), &mut |e| seen.push(e.clone()));
        assert_eq!(seen.len(), report.events.len(), "one log, two renderings (D8)");
        assert_eq!(seen, report.events);
    }
}
```

- [x] **Step 3: Run the tests, verify they fail** — **deviation, recorded honestly:**
      tests and implementation were written into `watch.rs` in one pass, so there was
      no separate red run for this task. The first execution failed 1 of 13 (see
      Step 5), which is the only red signal this task produced.

- [x] **Step 4: Implement against the tests**

Write `watch` to the signature above. Every branch is pinned by a test, so the
tests determine the body. Four rules the tests encode:

- **Loop order is poll → classify → emit → (unless terminal or `once`) sleep.**
  Poll `k` therefore happens at `t = interval * (k-1)`, which is what makes the
  heartbeat test land deterministically on the 600s threshold rather than
  straddling it.
- **Event-kind assignment.** On the **first** poll, emit the full current state:
  one `Phase` event (detail = `phase.as_str()`), one `Check` event per required
  context, and one `Queue` event if queued. Thereafter emit **only the changed
  components**: a queue-component change → `Queue`; a phase change → `Phase`; a
  required-check state change → `Check`. A `Step::Continue { warn: Some(_) }`
  always emits `Warning`. The terminal step emits `Terminal`.
- **Every event goes to the sink _and_ is pushed to `report.events`** — one log,
  two renderings (D8). `every_event_reaches_both…` fails if they diverge.
- **The ejection probe is issued only when `needs_ejection_probe` says so**
  (Task 4), and a probe failure is a `PollError` event, never a silent `None` —
  a `None` would read as "not ejected."

`required_checks` is fetched once before the loop; a failure to fetch it is
subject to the same strike/rate-limit logic, not a separate path. `SystemClock`
implements `Clock` over `std::time` and `std::thread::sleep`.

- [x] **Step 5: Run the tests, verify they pass** — **13** tests (the plan's 12 plus
      `unix_seconds_format_as_rfc3339_utc`: the event log needs real timestamps and
      xtask has no date crate, so `SystemClock` formats them itself and the algorithm
      is pinned against four known epochs).

      First run: 12 passed, `a_queue_position_change_emits_per_change` failed with 3
      Queue events. Real defect — the terminal poll also emitted the PR *leaving* the
      queue, restating what the `Terminal` event already says. Fixed by suppressing
      component events on the terminal poll; second run 13/13.

      Also fixed a `clippy::type_complexity` failure by introducing the `Rendered`
      struct instead of an `#[allow]`.

- [x] **Step 6: Commit** — `b7769604`, full `cargo xtask check` green.

```bash
git add xtask/src/pr/watch.rs xtask/src/pr/test_support.rs xtask/src/pr/mod.rs
git commit -m "feat(xtask): PR watch poll loop with change-only events (#729)"
```

---

### Task 6: `land.rs` — divergence guard and arming prologue

**Files:**

- Create: `xtask/src/pr/land.rs`
- Modify: `xtask/src/pr/mod.rs` (add `pub mod land;`)
- Test: in-file `#[cfg(test)]` in `xtask/src/pr/land.rs`

**Interfaces:**

- Consumes: everything from Tasks 1–5, including the Task 4/5 `test_support`
  builders. **No git IO here** — the guard is pure and the reads happen in
  Task 7.
- Produces:

```rust
/// Arming is a mutation, so it is its own trait — a `PrSource` cannot merge anything.
pub trait PrArmer { fn arm_auto_merge(&self, subject: &Subject) -> Result<(), ApiError>; }

pub struct GhArmer;   // runs `gh pr merge <N> --auto --merge` via gh::run_gh_raw

#[derive(Debug, PartialEq, Eq)]
pub enum GuardVerdict { Proceed, Diverged { local: String, remote: String } }

/// Pure — the caller supplies what git said (D9: the guard itself is not IO).
pub fn divergence_guard(current_branch: Option<&str>, local_sha: Option<&str>,
                        pr_head_ref: &str, pr_head_sha: &str) -> GuardVerdict;

/// The exit-2 refusal message. Pure, so A11's "names both SHAs" is testable.
pub fn divergence_message(local: &str, remote: &str) -> String;

/// Never returns Err — every terminal path is a report (D5/A3).
pub fn land<S: PrSource, A: PrArmer, C: Clock>(
    source: &S, armer: &A, clock: &C, subject: &Subject, cfg: WatchConfig,
    sink: &mut dyn FnMut(&Event),
) -> PrReport;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::test_support::*;

    // ---- A11: the divergence guard ----

    #[test]
    fn matching_branch_and_sha_proceeds() {
        assert_eq!(divergence_guard(Some("feature"), Some("abc"), "feature", "abc"),
                   GuardVerdict::Proceed);
    }

    #[test]
    fn same_branch_different_sha_is_divergence() {
        match divergence_guard(Some("feature"), Some("local1"), "feature", "remote2") {
            GuardVerdict::Diverged { local, remote } => {
                assert_eq!(local, "local1");
                assert_eq!(remote, "remote2");
            }
            other => panic!("expected divergence, got {other:?}"),
        }
    }

    #[test]
    fn a_different_branch_is_location_agnostic() {
        // Invoked from elsewhere: no refusal (D9).
        assert_eq!(divergence_guard(Some("main"), Some("zzz"), "feature", "abc"),
                   GuardVerdict::Proceed);
    }

    #[test]
    fn detached_head_or_no_git_proceeds() {
        assert_eq!(divergence_guard(None, None, "feature", "abc"), GuardVerdict::Proceed);
    }

    #[test]
    fn the_refusal_message_names_both_shas() {
        let m = divergence_message("local1", "remote2");
        assert!(m.contains("local1"), "must name the local sha: {m}");
        assert!(m.contains("remote2"), "must name the PR head sha: {m}");
    }

    // ---- A10: the arming prologue ----

    struct CountingArmer { calls: std::cell::Cell<u32> }
    impl CountingArmer { fn new() -> Self { Self { calls: std::cell::Cell::new(0) } } }
    impl PrArmer for CountingArmer {
        fn arm_auto_merge(&self, _: &Subject) -> Result<(), ApiError> {
            self.calls.set(self.calls.get() + 1);
            Ok(())
        }
    }

    #[test]
    fn a_silent_no_op_arm_is_retried_once_then_succeeds() {
        // Snapshots: prologue, after arm #1 (NOT armed), after arm #2 (armed), merged.
        let src = FakeSource::new(vec![
            Ok(open_pending()), Ok(open_pending()), Ok(armed_snapshot()),
            Ok(merged_snapshot()),
        ], queue_rules());
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(armer.calls.get(), 2, "a silent no-op must be re-armed exactly once");
        assert_eq!(report.outcome, Outcome::Merged);
    }

    #[test]
    fn a_direct_enqueue_is_not_a_failed_arm() {
        // Green PR + live queue: autoMergeRequest stays null, isInMergeQueue is true.
        // The issue's stated predicate alone would misreport this as a failed arm.
        let src = FakeSource::new(vec![
            Ok(open_pending()), Ok(queued_at(1)), Ok(merged_snapshot()),
        ], queue_rules());
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(armer.calls.get(), 1, "a direct enqueue must not trigger a re-arm");
        assert_eq!(report.outcome, Outcome::Merged);
    }

    #[test]
    fn a_terminally_bad_pr_is_reported_without_arming() {
        let mut bad = open_pending();
        bad.mergeable = Mergeable::Conflicting;
        let src = FakeSource::new(vec![Ok(bad)], queue_rules());
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::Conflicted);
        assert_eq!(armer.calls.get(), 0, "never arm a PR that cannot merge");
    }

    #[test]
    fn an_already_merged_pr_is_a_no_op() {
        let src = FakeSource::new(vec![Ok(merged_snapshot())], queue_rules());
        let armer = CountingArmer::new();
        assert_eq!(land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {}).outcome,
                   Outcome::Merged);
        assert_eq!(armer.calls.get(), 0);
    }

    #[test]
    fn an_unexplained_failed_arm_is_watcher_error() {
        // Nothing blocking, yet neither arm sticks: a genuine contradiction between
        // what GitHub reports and what it does.
        let src = FakeSource::new(vec![Ok(open_pending())], queue_rules());  // repeats
        let armer = CountingArmer::new();
        let report = land(&src, &armer, &clock(), &subject(), cfg(), &mut |_| {});
        assert_eq!(report.outcome, Outcome::WatcherError);
        assert_eq!(armer.calls.get(), 2, "arm, then exactly one re-arm");
        assert!(report.detail.unwrap().contains("arm"));
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::land` Expected: FAIL —
`land` / `divergence_guard` not defined.

- [ ] **Step 3: Implement against the tests**

Write `divergence_guard`, `divergence_message`, and `land` to the signatures
above. Every branch is pinned by a test, so the tests determine the bodies.
Three rules they encode:

- **The armed predicate is `snap.auto_merge_armed || snap.queue.in_queue`** —
  the direct-enqueue test fails with the issue's narrower
  `autoMergeRequest.enabledAt`-only form.
- **The prologue takes one snapshot, arms, snapshots again, and re-arms at most
  once.** The post-arm-2 snapshot _is_ D9 step 5's "re-snapshot and classify":
  if it shows a terminal condition, report that; if it shows nothing blocking
  and still unarmed, that is `watcher-error`. `an_unexplained_failed_arm…` pins
  the call count at exactly 2.
- **`GhArmer` uses `gh::run_gh_raw`, not `run_gh`** — `gh pr merge` prints a
  human sentence, so `run_gh`'s JSON parse would classify every successful arm
  as `ApiError::Malformed`. It ignores stdout and the exit code entirely;
  verification is the next snapshot (D9 / the issue's point 4).

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml pr::land` Expected: PASS (10
tests)

- [ ] **Step 5: Commit**

```bash
git add xtask/src/pr/land.rs xtask/src/pr/mod.rs
git commit -m "feat(xtask): pr land arming prologue with honest arm verification (#729)"
```

---

### Task 7: git readers, subject resolution, and CLI wiring

**Files:**

- Modify: `xtask/src/git.rs` (three new readers — it currently has only
  `HOOKS_PATH`, `at`, `porcelain_is_dirty`, `needs_hooks_path`,
  `working_tree_status`, `hooks_path`, `ensure_hooks_path`; there is no branch,
  SHA, or remote reader)
- Modify: `xtask/src/lib.rs` (add `Command::Pr(PrCommand)`, the `run()` arms,
  and the `command_name()` rows — mirroring `Command::Traces` at
  `lib.rs:157-160` and `:294-295`)
- Modify: `xtask/src/pr/snapshot.rs` (replace `GhSource::resolve`'s
  `unimplemented!`; add `parse_remote` and `resolution_failure`)
- Test: in-file `#[cfg(test)]` in `xtask/src/git.rs`, `xtask/src/lib.rs`
  (`cli_tests`), and `xtask/src/pr/snapshot.rs`

**Interfaces:**

- Consumes: everything from Tasks 1–6.
- Produces:

```rust
// xtask/src/git.rs — all three go through `at()` so GIT_* stays scrubbed
pub fn current_branch(dir: &Path) -> anyhow::Result<Option<String>>;  // None when detached
pub fn head_sha(dir: &Path) -> anyhow::Result<Option<String>>;        // None in an empty repo
pub fn remote_url(dir: &Path, name: &str) -> anyhow::Result<Option<String>>;

// xtask/src/pr/snapshot.rs
pub fn parse_remote(url: &str) -> Option<(String, String)>;

/// D13's failure boundary, made pure so it can be table-tested: which ApiErrors
/// bail to exit 2 and which become a `watcher-error` report.
#[derive(Debug, PartialEq, Eq)]
pub enum ResolutionFailure { Bail(String), Report(Outcome) }
pub fn resolution_failure(err: &ApiError) -> ResolutionFailure;

// xtask/src/lib.rs
#[derive(clap::Subcommand)]
pub enum PrCommand {
    Watch { number: Option<u64>,
            #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(5..))]
            interval: u64,
            #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u64).range(1..))]
            timeout: u64,
            #[arg(long)] once: bool },
    Land  { number: Option<u64>,
            #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(5..))]
            interval: u64,
            #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u64).range(1..))]
            timeout: u64 },
}
```

- [ ] **Step 1: Write the failing tests**

In `xtask/src/git.rs`, using the existing `crate::test_support` throwaway-repo
helpers (the idiom the file's own tests already use, and the reason `at()`
scrubs `GIT_*`):

```rust
// `temp_repo(prefix, tag) -> PathBuf` and `commit(dir, rel, body)` — the existing
// signatures in xtask/src/test_support.rs:28,50.
#[test]
fn current_branch_head_sha_and_remote_read_a_real_repo() {
    let repo = crate::test_support::temp_repo("pr-git", "readers");
    crate::test_support::commit(&repo, "seed.txt", "seed");
    assert!(current_branch(&repo).unwrap().is_some());
    let sha = head_sha(&repo).unwrap().expect("a seeded repo has a HEAD");
    assert_eq!(sha.len(), 40, "full sha, not abbreviated");
    assert_eq!(remote_url(&repo, "origin").unwrap(), None, "no remote configured");
}
```

In `xtask/src/pr/snapshot.rs`:

```rust
#[test]
fn remote_urls_parse_to_owner_and_repo() {
    // A15: repo identity comes from the remote, never a hardcoded literal.
    for url in ["git@github.com:jaunder-org/jaunder.git",
                "https://github.com/jaunder-org/jaunder.git",
                "https://github.com/jaunder-org/jaunder"] {
        assert_eq!(parse_remote(url), Some(("jaunder-org".into(), "jaunder".into())), "{url}");
    }
    assert_eq!(parse_remote("not-a-remote"), None);
}

#[test]
fn no_hardcoded_repo_literal_in_the_module() {
    // Falsifiable guard for A15: reintroducing the literal breaks this.
    assert!(!include_str!("snapshot.rs").contains("\"jaunder-org/jaunder\""));
}

#[test]
fn resolution_failures_split_exit_two_from_watcher_error() {
    // D13's boundary, and the single most subtle rule in the spec:
    // failures to ESTABLISH the subject exit 2; tooling failures are reports.
    assert!(matches!(resolution_failure(&ApiError::NotFound), ResolutionFailure::Bail(_)));
    for tooling in [ApiError::GhMissing, ApiError::Unauthenticated,
                    ApiError::RateLimited { reset_unix: None },
                    ApiError::Transport("x".into()), ApiError::Malformed("x".into()),
                    ApiError::GraphQlErrors("x".into())] {
        assert_eq!(resolution_failure(&tooling),
                   ResolutionFailure::Report(Outcome::WatcherError),
                   "{tooling:?} is the tooling breaking, not a missing subject");
    }
}
```

In `xtask/src/lib.rs`'s `cli_tests` module:

```rust
#[test]
fn pr_watch_parses_with_defaults_and_optional_number() {
    let cli = Cli::try_parse_from(["xtask", "pr", "watch"]).unwrap();
    assert_eq!(cli.command_name(), "pr-watch");
    match cli.command {
        Command::Pr(PrCommand::Watch { number, interval, timeout, once }) => {
            assert_eq!(number, None);
            assert_eq!(interval, 30);
            assert_eq!(timeout, 90);
            assert!(!once);
        }
        _ => panic!("expected pr watch"),
    }
}

#[test]
fn pr_watch_parses_explicit_number_and_flags() {
    let cli = Cli::try_parse_from(
        ["xtask", "pr", "watch", "731", "--interval", "60", "--timeout", "10", "--once"]).unwrap();
    match cli.command {
        Command::Pr(PrCommand::Watch { number, interval, timeout, once }) => {
            assert_eq!(number, Some(731));
            assert_eq!(interval, 60);
            assert_eq!(timeout, 10);
            assert!(once);
        }
        _ => panic!("expected pr watch"),
    }
}

#[test]
fn pr_land_rejects_once() {
    // A11: arming and immediately not watching is never intended.
    assert!(Cli::try_parse_from(["xtask", "pr", "land", "--once"]).is_err());
}

#[test]
fn pr_interval_below_the_floor_is_rejected() {
    assert!(Cli::try_parse_from(["xtask", "pr", "watch", "--interval", "1"]).is_err());
}

#[test]
fn pr_land_names_itself() {
    assert_eq!(Cli::try_parse_from(["xtask", "pr", "land"]).unwrap().command_name(), "pr-land");
}

#[test]
fn pr_requires_a_subcommand() {
    assert!(Cli::try_parse_from(["xtask", "pr"]).is_err());
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml` Expected: FAIL —
`Command::Pr` / `PrCommand` / `parse_remote` / `current_branch` not defined.

- [ ] **Step 3: Implement against the tests**

Write the three `git.rs` readers via `git::at(dir)` (so `GIT_*` stays scrubbed —
`CONTRIBUTING.md:174-185`): `rev-parse --abbrev-ref HEAD`, `rev-parse HEAD`, and
`remote get-url <name>`, each mapping "not present" to `Ok(None)` rather than an
error.

Write `parse_remote` and `resolution_failure` — both pinned by the tests above.

Add `Pr(PrCommand)` to `Command` with a doc comment in the house style (see
`Command::Traces`, `lib.rs:158-160`) and an `after_help` example block. Add the
`command_name()` rows returning `"pr-watch"` / `"pr-land"`.

Add the `run()` arms. Each one, in order:

1. `let start = std::time::Instant::now();`
2. Build `GhSource` / `GhArmer` / `SystemClock`.
3. `resolve` the subject. On error, apply `resolution_failure`: `Bail(msg)` →
   `anyhow::bail!(msg)` (the `Err` path → exit 2, no report); `Report(outcome)`
   → fall through to a `PrReport` carrying it.
4. For `land` only: read `current_branch` / `head_sha`, apply
   `divergence_guard`, and on `Diverged` →
   `anyhow::bail!(divergence_message(&local, &remote))` (exit 2, no report).
5. Run `watch` / `land` with a sink that writes each event to **stderr**.
6. `let mut result = pr::into_result(name, report);` then
   **`finalize(&mut result, start)`** before returning — every other arm in
   `run()` does this, and without it `duration_ms`/`finished_at_unix` stay `0`
   in the sidecar and in `xtask-done:`, and D5's human rendering has no elapsed
   time to print.

`GhSource::resolve` reads the remote via `crate::git::remote_url` +
`parse_remote`, and when `number` is `None` asks
`gh pr list --head <branch> --state open --json number` for the current branch,
mapping an empty list to `ApiError::NotFound`.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml` Expected: PASS (whole xtask
suite)

- [ ] **Step 5: Verify the help text (A14)**

Run: `cargo run --manifest-path xtask/Cargo.toml -- pr watch --help` Expected:
documents `--interval <SECONDS>`, `--timeout <MINUTES>`, `--once`, and the
optional PR number.

- [ ] **Step 6: Commit**

```bash
git add xtask/src/git.rs xtask/src/lib.rs xtask/src/pr/snapshot.rs
git commit -m "feat(xtask): wire pr watch/land into the CLI with remote-derived subject (#729)"
```

---

### Task 8: ADR draft

**Files:**

- Create: `docs/adr/drafts/xtask-github-pr-observation.md` (numberless —
  `cargo xtask adr promote` numbers it at ship, per ADR-0048)

**Interfaces:** none (documentation).

- [ ] **Step 1: Write the ADR draft**

Follow `docs/adr/template.md`. **The H1 must be exactly `# ADR-DRAFT: <Title>`
and the status must be lowercase `accepted`** — `adr_readme.rs:25-31` defines
`STATUS_VOCAB = ["proposed", "accepted", "superseded", "deprecated", "rejected"]`
and matches case-sensitively. A capitalised `Accepted` would **not** be caught
by Step 2 (`docs/adr/drafts/` is exempt from the format check) and would instead
fail at `cargo xtask adr promote` during ship — the worst possible moment.

Record the four decisions from spec D12, each with the rationale that would
otherwise be excavated:

1. **xtask's charter extends to host-side observation of the CI/merge system.**
   ADR-0028's litmus (`0028-devtool-vs-xtask-boundary.md:63-65`) is "invoking
   `nix`, or analyzing build outputs" — this is neither, so the extension is
   stated rather than assumed.
2. **`gh` as transport, not a Rust client.** Include the measurements (spec F7):
   octocrab 0.54 resolves to 211 crates against xtask's 93, adds
   tokio/hyper/rustls/ring, models **no** merge-queue state, and its GraphQL
   surface requires the same hand-written query and self-defined structs.
3. **The gate shape is data read from the ruleset** (spec D6/F2), so the
   ADR-0077 rollback documented in `docs/ci-merge-queue.md` needs no code
   change.
4. **Observe/act split** — `watch` cannot merge; running `land` _is_ the
   approval.

Cite ADR-0028 and ADR-0077 by path.

- [ ] **Step 2: Verify the ADR gate accepts it**

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS — `adr-check`
and `doc-links` green.

- [ ] **Step 3: Commit**

```bash
prettier -w docs/adr/drafts/xtask-github-pr-observation.md
git add docs/adr/drafts/xtask-github-pr-observation.md
git commit -m "docs(adr): draft xtask GitHub PR observation boundary (#729)"
```

---

### Task 9: Tracked docs and `flake.nix`

**Files:**

- Modify: `flake.nix:1311-1320` (add `pkgs.gh` to `devOnly`)
- Modify: `docs/ci-merge-queue.md:190` (the false auto-requeue claim)
- Modify: `CONTRIBUTING.md` (manual-tools section, beside `audit-wasm` /
  `traces`)
- Modify: `CLAUDE.md` (the xtask Commands table)

**Interfaces:** none (configuration + documentation).

- [ ] **Step 1: Add `gh` to the devShell**

In `flake.nix`'s `devOnly` list, add `pkgs.gh` — **not** `ciInputs`, since these
are host-only manual commands (spec D2), the same status as `traces analyze`.

- [ ] **Step 2: Correct the merge-queue runbook**

`docs/ci-merge-queue.md:190` currently reads "Under the queue an OOM-ejected PR
is auto-requeued". Per spec F3 the live `merge_queue` rule has **no requeue
parameter**, so ejection is terminal until someone re-enqueues. Replace the
claim and adjust the surrounding rollback-trigger paragraph so it still reads
correctly. Add a pointer to `cargo xtask pr watch` as the way to observe queue
state.

- [ ] **Step 3: Document the commands**

`CONTRIBUTING.md`: add a bullet in the manual-tools section (beside the
`audit-wasm` and `traces analyze` bullets) covering `cargo xtask pr watch [N]` /
`pr land [N]`, the flags, the outcomes, and that `land` is the merge approval.

`CLAUDE.md`: one row in the xtask Commands table. Keep it terse — CLAUDE.md must
not duplicate CONTRIBUTING.

- [ ] **Step 4: Verify the tree still gates**

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS — `doc-links`
green.

Run: `git status --porcelain` Expected: only the intended files (the gate's Fix
mode auto-formats; re-stage if so).

- [ ] **Step 5: Commit**

```bash
prettier -w docs/ci-merge-queue.md CONTRIBUTING.md CLAUDE.md
git add flake.nix docs/ci-merge-queue.md CONTRIBUTING.md CLAUDE.md
git commit -m "docs: point the merge-queue protocol at cargo xtask pr watch (#729)"
```

---

### Task 10: Ship-time — manual smoke run and the out-of-tree skill update

> **Both halves happen at ship, after the PR exists and the flag names are
> final.** Neither produces a reviewable diff: the smoke run is a manual
> exercise (D11 forbids networked tests) and `.claude/` is untracked (F10).

**Files:**

- Modify: `/home/mdorman/src/jaunder/.claude/skills/jaunder-ship/SKILL.md` steps
  7–8 (**the main checkout — this path does not exist in the worktree**)

- [ ] **Step 1: Smoke-run `pr watch --once` against this branch's own PR (A1)**

Run: `devtool run -- cargo xtask --json pr watch --once` Expected: exit 1 with
`pr.outcome` = `pending` (or a real terminal outcome), `pr.pr` matching this
branch's PR number, `pr.head_sha` matching `git rev-parse HEAD`, and a non-empty
`pr.events`. This is the only exercise of the real `gh` path — it is what keeps
A1 from resting entirely on `--help`.

Run: `devtool run -- cargo xtask pr watch 999999` Expected: exit **2**, no `pr`
key in `.xtask/last-result.json` — D13's subject-cannot-be-established boundary,
observed end-to-end.

- [ ] **Step 2: Rewrite `jaunder-ship` step 7**

Replace the `gh pr checks <N> --watch` prose and the rebase-on-every-advance
instruction with `cargo xtask pr watch <N>`. **Remove the "keep current with
`main`" requirement** — spec F1 shows the live ruleset is non-strict with a
merge queue, so a stale branch no longer blocks a merge; that instruction is
obsolete, not merely superseded.

Rewrite the babysit-loop paragraph to use `cargo xtask pr watch <N> --once` per
PR instead of hand-rolled `gh pr checks`.

- [ ] **Step 3: Rewrite `jaunder-ship` step 8**

Replace `gh pr merge <N> --merge` / `--auto --merge` and the manual
`autoMergeRequest.enabledAt` verification with `cargo xtask pr land <N>`,
keeping the explicit-per-PR-approval halt exactly as it is — running `land`
**is** the approval.

- [ ] **Step 4: Verify by inspection**

Run:
`rg -n 'pr watch|pr land|gh pr checks|--watch' /home/mdorman/src/jaunder/.claude/skills/jaunder-ship/SKILL.md`
Expected: the two new commands present; no surviving `gh pr checks … --watch`.

- [ ] **Step 5: No commit** — the file is untracked. Note in the PR description
      that A13b was completed out of tree, and record the Step 1 smoke results
      there as A1's evidence.

---

## Self-review

**Spec coverage.** D1→T6/T7; D2→T2, T9 (`pkgs.gh`); D3→T1–T6; D4→T1
(`Outcome`) + T4 (reaching each); D5→T1 + T7 (`finalize`); D6→T3 (parsing) + T4
(branching); D7→T5; D8→T5; D9→T6 + T7 (the git reads and the exit-2 refusal);
D10→T3 (`parse_ejection_run`) + T4 (the discriminator); D11→every task's tests;
D12→T8, T9, T10; D13→T7 (`resolution_failure`).

**Acceptance criteria.** A1→T10 Step 1 (the manual smoke — `--help` alone was
not evidence); A2→T1 + T7 + T10 Step 1 (exit 2 observed end-to-end); A3→T1
**plus the type signature** (`watch`/`land` return `PrReport`, so the `Err` path
is unrepresentable); A4→T5; A5→T5; A6 both directions→T4; A7/A7b→T4; A8→T4
(`no_check_name_is_hardcoded`) + T3 (`strict_rollback_ruleset…`); A9→T5; A10→T6;
A11→T6 (`the_refusal_message_names_both_shas`, the guard verdicts) + T7
(`pr_land_rejects_once`, the bail wiring); A12→T5 + T7; A13→T9; A13b→T10; A14→T7
Step 5; A15→T7 (`remote_urls_parse…`, `no_hardcoded_repo_literal…`,
`resolution_failures_split…`); A16→T1.

**Placeholder scan.** No TBD/TODO; no "add error handling"; no "similar to Task
N". Every helper used across files is defined in `pr/test_support.rs` (T4 Step
1, T5 Step 1), not in a private `mod tests`. `FakeSource`'s exhaustion semantics
are specified, because two tests depend on them. `GhSource::resolve`'s
`unimplemented!("Task 7")` in T3 is deliberate, untested by design, and replaced
in T7 Step 3.

**Type consistency.** `Outcome`, `Event`/`EventKind`, `PrReport`, `Subject`,
`PrNumber` (T1, all with `Debug`) are used unchanged in T3–T7.
`PrSnapshot`/`RequiredChecks`/`RunRef`/`CheckEntry`/`QueueState` (T3) are used
unchanged in T4–T6. `Step`/`Phase`/`Progress` (T4) are used in T5. `PrSource` is
declared once (T3) and implemented by `GhSource` (T3/T7) and `FakeSource` (T5).
`WatchConfig`/`Clock` (T5) are consumed by T6 and T7. `PrArmer` (T6) is
deliberately separate from `PrSource` so no observer can mutate. `run_gh` (JSON)
and `run_gh_raw` (non-JSON) are both declared in T2, so T6's `GhArmer` does not
have to invent the second one.
