# Wire Arg Error Gate Implementation Plan

**For agentic workers:** Execute with `jaunder-iterate`; delegate individual
tasks with `jaunder-dispatch` when useful. Tick each task checkbox before its
commit gate.

## Review Header

**Goal:** Preserve useful wire-arg validation errors for clients/users while
preventing raw server-fn arg-decode error text from entering exported telemetry,
then add an xtask gate that keeps the sanitization and wire-arg display
classifications honest.

**Scope in:** `web/src/error/server.rs` decode telemetry behavior/tests; a new
`xtask/src/steps/server_fn_wire_arg_error_check.rs`; `xtask/src/lib.rs` step
wiring; unit tests for population expansion, display-shape analysis, allowlist
hygiene, current Jaunder fixtures, decode-telemetry sanitization, and version
drift for `email_address`/`croner`.

**Scope out:** General Rust typechecking, scanning non-server-wire `FromStr`
errors, changing metric names/kind/class/context/event identity/HTTP response
shape, parsing third-party crate source, sanitizing or removing
`BackupSchedule`'s detailed user/client-facing validation message.

**Tasks:**

1. Sanitize arg-decode telemetry source while preserving outward server-function
   errors.
2. Add the xtask step shell, fixtures, and server input population model.
3. Add first-party `FromStr::Err` display-shape analysis.
4. Add version-pinned display allowlist for `Email` and `BackupSchedule`.
5. Wire the gate into check/validate and validate the final branch.

**Key risks/decisions:**

- Server-fn arg decode happens before function body auth, so `BackupSchedule`
  cannot be justified by `require_operator()` having run.
- User-facing/client response text and telemetry source text are different
  surfaces. Preserve `WebError::ServerFunction(value.to_string())`; sanitize
  only `error.source` for decode telemetry.
- Request aggregates must be expanded structurally, not by `*Request` suffix,
  because `PostInputs` is a server input aggregate.
- `PasswordError` requires reachability from `FromStr`; whole-enum scanning
  would falsely fail on hashing/verification variants.
- `Email` is telemetry-safe only through a version-pinned
  `email_address = 0.2.9` allowlist. `BackupSchedule`/`croner = 2.2.0` is
  user-facing-only and passes only while decode telemetry source remains
  sanitized.
- `Filename::TooLong { encoded: usize }` is allowed because numeric counters are
  not input echo.

## Global Constraints

- Follow the approved spec:
  `/home/mdorman/src/jaunder/agent-1/docs/superpowers/specs/2026-08-20-issue-846-wire-arg-error-gate.md`.
- Use `devtool run -- <cmd>` for all run-and-inspect commands. No
  package-manager wrappers, no shell pipelines.
- Keep the new gate pure where possible: `problems(...) -> Option<String>`
  style, sorted detail, recovery footer,
  `StepResult::fail("server-fn-wire-arg-error")` on violations.
- Fail loud on unreadable/unparsable policed sources and stale/malformed
  allowlist entries.
- Do not add `#[allow(...)]`/`#[expect(...)]` without explicit user approval.
- Before each commit: tick the relevant checkbox, run
  `devtool run -- cargo xtask check`, inspect/stage any mechanical fixes, then
  commit with no `Co-Authored-By` trailer.

## Task 1: Sanitize Decode Telemetry Source

**Files:**

- Modify: `web/src/error/server.rs`

**Interfaces:**

- Change `emit_arg_decode_failure` to emit the same boundary event without
  preserving the raw `ServerFnErrorErr` as source. Prefer the existing
  source-free constructor:

```rust
InternalError::validation("invalid request arguments")
    .with_context("stage", "decode")
    .emit_boundary_failure();
```

- Keep `web/src/error/wire.rs` behavior unchanged:
  `WebError::from_server_fn_error` still returns
  `Self::server_function(value.to_string())`.

**Steps:**

- [x] Update `arg_decode_failure_emits_a_boundary_event` so it proves both sides
      of the split:
  - returned `WebError` remains `ServerFunction` and still contains the original
    framework/newtype text;
  - boundary event has `Validation`/`Client`, `stage = decode`, event identity
    `server function failed`, and public message `invalid request arguments`;
  - `error.source` does **not** contain a unique submitted marker from the
    sample decode text.

Run:

```bash
devtool run -- cargo nextest run -p web --features server error::server::tests::arg_decode_failure_emits_a_boundary_event
```

Expected: FAIL before the code change because `error.source` still contains the
decode text.

- [x] Change `emit_arg_decode_failure` from
      `validation_source(..., value.clone())` to source-free `validation(...)`
      and update comments to state the telemetry/user-response split in present
      tense.

Run:

```bash
devtool run -- cargo nextest run -p web --features server error::server::tests::arg_decode_failure_emits_a_boundary_event
```

Expected: PASS.

- [x] Tick this task checkbox, then run the commit gate.

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS.

Commit exactly:

```bash
devtool run -- git add web/src/error/server.rs docs/superpowers/plans/2026-08-20-issue-846-wire-arg-error-gate.md docs/superpowers/specs/2026-08-20-issue-846-wire-arg-error-gate.md
devtool run -- git commit -m "fix(web): sanitize arg decode telemetry source"
```

## Task 2: Add Gate Skeleton and Server Input Population

**Files:**

- Add: `xtask/src/steps/server_fn_wire_arg_error_check.rs`
- Modify: `xtask/src/lib.rs` only if needed for local test compilation after
  adding the module; final command wiring can wait until Task 5.

**Interfaces:**

- New step-local types:

```rust
struct WireInput {
    server_fn: String,
    root: String,
    field_path: Vec<String>,
    ty: String,
}

struct TypeIndex { /* local structs/enums and impls from web/src + common/src */ }
```

- Pure functions with unit-testable seams:

```rust
fn wire_inputs(web_sources: &[(String, String)], common_sources: &[(String, String)]) -> Result<Vec<WireInput>, String>;
fn expand_type(root: &str, index: &TypeIndex, out: &mut Vec<WireInput>) -> Result<(), String>;
```

Exact names may change, but keep these seams: enumerate server roots, index
local aggregate fields, recursively expand leaf types.

**Steps:**

- [x] Create the module with no command wiring yet. Import
      `crate::web_server_fns` and use `syn` parsing, not regex, for function
      params and struct fields.

- [x] Add tests that fail until implemented:
  - direct arg: `#[macros::server] async fn update(email: Email)` yields
    `Email`;
  - nested request field: `LoginRequest { password: ProfferedPassword }` yields
    `ProfferedPassword` with field path;
  - non-suffix aggregate: `PostInputs { title: PostTitle }` passed as
    `post: PostInputs` yields `PostTitle`;
  - container unwrapping: `Option<DestinationPath>` and `Vec<Email>` yield their
    leaf types;
  - server return type is ignored;
  - unparsable source returns an error, not a smaller input set.

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_wire_arg_error_check
```

Expected: FAIL with missing/incomplete gate behavior.

- [x] Implement minimal local type indexing for `web/src` and `common/src`:
      last-segment names, local structs with named/tuple fields as needed, and
      obvious container unwrapping. Do not implement a full import resolver.

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_wire_arg_error_check
```

Expected: PASS for population tests.

- [x] Tick this task checkbox, then run the commit gate.

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS.

Commit exactly:

```bash
devtool run -- git add xtask/src/steps/server_fn_wire_arg_error_check.rs xtask/src/lib.rs docs/superpowers/plans/2026-08-20-issue-846-wire-arg-error-gate.md
devtool run -- git commit -m "feat(xtask): enumerate server wire arg leaves"
```

## Task 3: Analyze First-Party Error Display Shapes

**Files:**

- Modify: `xtask/src/steps/server_fn_wire_arg_error_check.rs`

**Interfaces:**

- Add first-party error analysis over reachable leaf types:

```rust
fn from_str_error_for(type_name: &str, index: &TypeIndex) -> Option<ErrorType>;
fn display_classification(error: &ErrorType, reachable: &Reachability, index: &TypeIndex) -> DisplayClass;
```

- Reachability can be implemented as either:
  - simple body analysis of the relevant `FromStr` implementation and helper
    calls returning the same error type; or
  - a typed stale-checked unreachable-variant allowance.

Use whichever keeps the code smaller and clearer. Do not add a blanket
`PasswordError` pass.

**Steps:**

- [x] Add failing tests for display-shape policy:
  - unit/literal `#[error("post title must be non-empty")]` is telemetry-safe;
  - const/static placeholder
    `#[error("password must be at least {MIN_LENGTH} characters")]` is
    telemetry-safe;
  - numeric scalar field
    `#[error("filename too long: {encoded} bytes")] { encoded: usize }` is
    telemetry-safe;
  - tuple `String` interpolation `#[error("bad {0}")] struct Bad(String);` is
    unsafe for telemetry;
  - named `String` debug interpolation `#[error("bad {value:?}")]` is unsafe for
    telemetry;
  - `#[error(transparent)]` over an unproven inner type is unsafe for telemetry;
  - `ProfferedPassword::FromStr<Err = PasswordError>` is accepted without
    accepting the hashing/verification variants.

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_wire_arg_error_check
```

Expected: FAIL before analysis is implemented.

- [x] Implement first-party `thiserror` shape analysis for structs/enums and
      constants. Keep failure messages path/type/variant-specific.

- [x] Implement the chosen `PasswordError` reachability treatment narrowly, with
      tests that would fail if `HashingFailed`/`VerificationFailed` were treated
      as decode-reachable.

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_wire_arg_error_check
```

Expected: PASS.

- [x] Tick this task checkbox, then run the commit gate.

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS.

Commit exactly:

```bash
devtool run -- git add xtask/src/steps/server_fn_wire_arg_error_check.rs docs/superpowers/plans/2026-08-20-issue-846-wire-arg-error-gate.md
devtool run -- git commit -m "feat(xtask): classify wire arg error displays"
```

## Task 4: Add Version-Pinned Display Allowlist and Sanitization Check

**Files:**

- Modify: `xtask/src/steps/server_fn_wire_arg_error_check.rs`

**Interfaces:**

- Add a typed allowlist similar in spirit to `sqlx_newtype_decode_check`:

```rust
enum ExternalDisplayCategory {
    TelemetrySafe,
    UserFacingOnly,
}

struct AllowedExternalDisplay {
    wire_type: &'static str,
    error_type: &'static str,
    wrapped_type: &'static str,
    crate_name: &'static str,
    crate_version: &'static str,
    category: ExternalDisplayCategory,
    reason: &'static str,
}
```

- Initial legitimate entries:

```rust
AllowedExternalDisplay {
    wire_type: "Email",
    error_type: "InvalidEmail",
    wrapped_type: "email_address::Error",
    crate_name: "email_address",
    crate_version: "0.2.9",
    category: ExternalDisplayCategory::TelemetrySafe,
    reason: "email_address 0.2.9 Error is a unit-variant enum whose Display emits literals and constants only; re-review on version change (#846)",
}
AllowedExternalDisplay {
    wire_type: "BackupSchedule",
    error_type: "InvalidBackupSchedule",
    wrapped_type: "croner::errors::CronError",
    crate_name: "croner",
    crate_version: "2.2.0",
    category: ExternalDisplayCategory::UserFacingOnly,
    reason: "croner's detailed schedule parse message is useful user feedback, but decode telemetry is source-sanitized; re-review on croner version change (#846)",
}
```

- Add a pure check that `web/src/error/server.rs::emit_arg_decode_failure` does
  not pass raw `ServerFnErrorErr` into `InternalError::validation_source` or
  equivalent source-preserving construction.

**Steps:**

- [x] Add failing tests for allowlist and sanitization hygiene:
  - `InvalidEmail(email_address::Error)`-like external wrapper fails without an
    allowlist entry;
  - telemetry-safe matching allowlist with nonblank reason and matching lockfile
    version passes;
  - user-facing-only matching allowlist with nonblank reason, matching lockfile
    version, and sanitized decode telemetry passes;
  - user-facing-only entry fails if fixture `emit_arg_decode_failure` uses
    `InternalError::validation_source(..., value.clone())`;
  - blank reason fails;
  - duplicate entry fails;
  - mismatched `Cargo.lock` version fails;
  - stale entry for an unreachable wire type fails;
  - `BackupSchedule`/`InvalidBackupSchedule(croner::CronError)` fails on
    `croner` version drift.

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_wire_arg_error_check
```

Expected: FAIL before allowlist/version/sanitization checks are implemented.

- [x] Implement Cargo.lock version extraction for the small allowlist check.
      Keep it deterministic and unit-testable with fixture lockfile text.

- [x] Implement the decode telemetry sanitization check over
      `web/src/error/server.rs`.

- [x] Add the `Email` allowlist entry with the current lockfile version `0.2.9`
      and the `BackupSchedule` allowlist entry with the current lockfile version
      `2.2.0`.

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_wire_arg_error_check
```

Expected: PASS.

- [x] Tick this task checkbox, then run the commit gate.

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS.

Commit exactly:

```bash
devtool run -- git add xtask/src/steps/server_fn_wire_arg_error_check.rs docs/superpowers/plans/2026-08-20-issue-846-wire-arg-error-gate.md
devtool run -- git commit -m "feat(xtask): pin wire arg display telemetry safety"
```

## Task 5: Wire Gate and Validate Final Branch

**Files:**

- Modify: `xtask/src/lib.rs`
- Modify: `xtask/src/steps/server_fn_wire_arg_error_check.rs`
- Modify: `docs/superpowers/plans/2026-08-20-issue-846-wire-arg-error-gate.md`

**Interfaces:**

- Add module export under `xtask/src/lib.rs` steps module list.
- Call `steps::server_fn_wire_arg_error_check::run(&mut result);` in both
  `Command::Check` and `Command::Validate`, near the existing `server_fn_*` and
  newtype static gates.
- Step name: `server-fn-wire-arg-error`.

**Steps:**

- [x] Wire the step into both check and validate.

- [x] Run the focused step tests.

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_wire_arg_error_check
```

Expected: PASS.

- [x] Run the fast static gate and confirm the new step appears and passes.

Run:

```bash
devtool run -- cargo xtask check --no-test
```

Expected: PASS, including `[ ok ] server-fn-wire-arg-error`.

- [x] Tick every remaining task checkbox in this plan, then run the per-commit
      gate.

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS.

Commit exactly:

```bash
devtool run -- git add xtask/src/lib.rs xtask/src/steps/server_fn_wire_arg_error_check.rs docs/superpowers/plans/2026-08-20-issue-846-wire-arg-error-gate.md
devtool run -- git commit -m "feat(xtask): gate server wire arg error telemetry"
```

- [ ] Validate exact final HEAD before `jaunder-ship`.

Run:

```bash
devtool run -- cargo xtask validate
```

Expected: PASS.

- [ ] Confirm clean final worktree.

Run:

```bash
devtool run -- git status --short
```

Expected: no output.
