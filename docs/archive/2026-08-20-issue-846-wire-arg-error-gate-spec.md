# Spec — #846: keep wire-arg decode telemetry input-free

- **Status:** draft for approval
- **Issue:** #846
- **Scope:** server-fn arg-decode telemetry sanitization plus an xtask gate that
  keeps unsafe wire-arg displays from re-entering telemetry unnoticed
- **Context:** #822 made server-fn arg-decode failures emit the standard
  boundary event. That event currently records the decode source in
  `error.source`, and `error.source` is exported to trace backends.

## Problem

Typed `#[server]` wire args deserialize through each newtype's validation path
before the function body. Since #822, a decode failure is observable:
`web::error` maps the framework error into an
`InternalError::validation_source(...)`, adds `stage = decode`, then
`InternalError::emit_boundary_failure` records the preserved source as
`error.source`.

ADR-0011's PII discipline applies to that field: exported span fields and
boundary fields must not contain user PII or secrets. The current property is
only an audit result, not a construction:

- most first-party wire-arg errors are constant-message unit errors;
- `Email` wraps `email_address::Error`, which is safe today only because
  `email_address` 0.2.9 displays unit variants and constants;
- `Filename` reports a byte count for `TooLong`, which is not an input echo but
  is still runtime interpolation;
- `BackupSchedule` wraps `croner::CronError`, and `croner` 2.2.0 displays
  runtime `String` payloads from invalid patterns/components.

A future wire arg, or a dependency bump, can silently move caller-supplied text
into exported telemetry.

The `BackupSchedule` case separates the real risks: `croner`'s detailed parse
text is useful to the person correcting the schedule, and no part of it is
inherently too sensitive to show back to that submitter. The problem is that
server-fn argument decode runs before `update_settings` reaches
`require_operator()`, so an unauthenticated malformed request can trigger decode
telemetry. Same-operator UX is therefore not a sound control for `error.source`.

## Decisions

### D1 — Sanitize arg-decode telemetry at the boundary

Change `web::error::emit_arg_decode_failure` so the boundary event remains
observable but does **not** preserve the `ServerFnErrorErr` as `error.source`.

Use the existing fixed public message, context, kind, class, event name, and
metric:

- `error.public = "invalid request arguments"`;
- `error.kind = validation`;
- `error.class = client`;
- `error.context` contains `stage = decode`;
- event identity remains `server function failed`;
- the outward `WebError::ServerFunction(value.to_string())` response remains
  unchanged.

This preserves useful user/client feedback while removing the telemetry path
that can export submitted text. It intentionally revises #822's
diagnostic-source choice for the decode path only; in-body `server_boundary`
errors still preserve typed sources as before.

### D2 — Gate the server wire-arg population, not every `FromStr` type

The gate's subject is **types that can be decoded as caller-supplied `#[server]`
inputs**. It must not scan every `FromStr` implementation in the repository.

Reason: issue #846 is about decode telemetry from server wire args.
Declaration-wide scanning would overreach into storage/config/CLI-only types,
force unrelated allowlists, and make the gate noisy enough to ignore.

Implementation seam:

- reuse `xtask/src/web_server_fns.rs` to enumerate `#[macros::server]` functions
  and direct parameters;
- then expand local DTO/request aggregates reachable from those parameters;
- include both `web/src` and `common/src`, because current request roots and
  domain DTOs are split across those crates;
- ignore server return types and storage/backend structs unless they are
  reachable from a server-fn input.

### D3 — Expand request aggregates by structure, not by name suffix

The gate must recursively inspect fields of local serde input aggregates, not
only direct server parameters.

Direct parameters catch `update_settings(schedule: BackupSchedule, ...)`; they
miss request bodies such as `LoginRequest`, `CreateInviteRequest`,
`DeleteMediaRequest`, and non-`*Request` aggregates such as `PostInputs`.
ADR-0129 makes cohesive request aggregates the server-fn boundary rule, so
suffix heuristics are already stale.

Required behavior:

- unwrap transparent containers that preserve input leaf semantics: `Option<T>`,
  `Vec<T>`, arrays, tuples, and similar obvious `syn::Type` containers;
- follow local struct fields used by a server-fn input root;
- report path/fn/root/field context for any offending leaf so the fix is
  findable;
- fail loud on unreadable or unparsable policed files rather than shrinking the
  population.

A deliberately small resolver is acceptable: last-segment type matching plus a
local type index is enough for the current tree. The spec does not require a
general Rust typechecker.

### D4 — Classify wire-arg display shapes so unsafe ones are explicit

For each reachable wire leaf whose deserialize path can call `FromStr`, the gate
must inspect `FromStr::Err`'s displayed error shape. The purpose is not to ban
useful user-facing messages; it is to ensure any display that can echo caller
text is explicit and cannot be mistaken for telemetry-safe source text.

Telemetry-safe by construction:

- literal `#[error("...")]` messages with no placeholders;
- placeholders that resolve to `const`/`static` items, e.g. `{MIN_LENGTH}`;
- numeric scalar fields such as `usize` counts, when the field type is scalar
  numeric and not string/custom-display text.

Unsafe for telemetry unless explicitly handled:

- `String`, `&str`, `Cow<str>`, or other string-like fields in an error display;
- tuple/named fields displayed through `{0}`, `{field}`, `{0:?}`, `{field:?}`,
  or transparent/custom `Display`;
- `#[error(transparent)]` unless the inner type is first-party and proven safe
  or externally allowlisted.

Unsafe-for-telemetry does not mean unsafe-for-user. `BackupSchedule` is the
named example: the detailed message is useful user feedback, but it must not be
exported as decode telemetry source.

### D5 — Use reachability from `FromStr`, not whole-error-enum pessimism

When one error enum has variants from multiple operations, the gate must reason
about the variants reachable from the relevant `FromStr` implementation, or
require a narrow local annotation/allowlist that records unreachable variants.

`ProfferedPassword::from_str` returns `PasswordError`, but it only calls the
shape validator, so only `PasswordTooShort` and `PasswordTooLong` are reachable.
The same enum also contains Argon2 wrapper variants for hashing and
verification. A whole-enum scan would either produce false failures or force a
type split outside this issue's scope.

Acceptable implementation choices:

1. inspect simple `FromStr` bodies well enough to see directly constructed enum
   variants and direct helper calls that return the same error; or
2. require a typed allowlist/annotation for unreachable variants with a reason
   and stale-entry checks.

A blanket `PasswordError` allowlist is not acceptable; it would hide future
decode-reachable string interpolation.

### D6 — External/user-facing wrappers require version-pinned allowlist entries

The gate cannot prove dependency `Display` implementations by syntax unless it
parses that crate. For wrapped external errors, require an explicit reviewed
allowlist entry keyed at least by:

- wire type;
- error type;
- wrapped/external type;
- crate name and version from `Cargo.lock`;
- category/rationale.

The allowlist must distinguish two categories:

1. **Telemetry-safe external display** — e.g.
   `InvalidEmail(email_address::Error)` with `email_address` 0.2.9, where the
   dependency's `Display` was audited to emit literals/constants only.
2. **User-facing only display** — e.g.
   `InvalidBackupSchedule(croner::CronError)` with `croner` 2.2.0, where the
   dependency may echo schedule text, but decode telemetry is sanitized at D1
   and the detailed message is intentionally retained for user/client feedback.

The allowlist must self-check:

- nonblank reason;
- matching crate version in `Cargo.lock`;
- no duplicate entry;
- no stale entry when the wrapped type is no longer reachable from server wire
  args;
- a user-facing-only entry is accepted only while the decode telemetry
  sanitization check passes.

This is the dependency-bump ratchet: updating `email_address` or `croner` must
force re-review rather than silently inheriting the old audit.

Use the `sqlx_newtype_decode_check` style: typed entries, stable keys, grouped
rationale, stale-entry tests. Inline Rust is preferable here; a JSON allowlist
is not required.

### D7 — Gate the decode telemetry sanitization contract

The new xtask step must also pin the D1 boundary contract: decode failures must
not call `InternalError::validation_source(...)` or otherwise pass the raw
`ServerFnErrorErr` into `error.source`.

A targeted syntax check over `web/src/error/server.rs::emit_arg_decode_failure`
is enough. The check should fail if that function reintroduces raw decode source
preservation while any reachable wire-arg display is user-facing-only or
otherwise unsafe for telemetry.

## Acceptance criteria

- **AC1** A failed arg decode still emits the boundary event and error metric
  with `Validation`/`Client`, `stage = decode`, event identity
  `server function failed`, and public message `invalid request arguments`.
- **AC2** Decode telemetry no longer records the raw `ServerFnErrorErr`/newtype
  error text in `error.source`; a test proves a message such as
  `password must be at least 8 characters` is absent from `error.source`.
- **AC3** The outward `WebError::ServerFunction(value.to_string())` response is
  unchanged, preserving useful client/user-facing decode errors.
- **AC4** A new xtask step derives its population from `#[macros::server]`
  inputs plus recursively expanded local request/DTO aggregates; it covers
  direct params, nested `*Request` structs, and non-suffix aggregates such as
  `PostInputs`.
- **AC5** First-party display analysis accepts literal messages, const/static
  interpolation, and numeric scalar counters, and classifies
  string/custom-display/transparent runtime interpolation as unsafe for
  telemetry unless proven safe or explicitly user-facing-only.
- **AC6** Shared error enums are handled by `FromStr` reachability or a narrow
  stale-checked unreachable-variant allowance; `ProfferedPassword` must not
  require a blanket `PasswordError` exemption.
- **AC7** Wrapped third-party displays require a version-pinned allowlist entry.
  `Email`/`InvalidEmail(email_address::Error)` is accepted only as
  telemetry-safe for the current `email_address` version;
  `BackupSchedule`/`InvalidBackupSchedule(croner::CronError)` is accepted only
  as user-facing-only for the current `croner` version and only while decode
  telemetry sanitization is in force.
- **AC8** The allowlist rejects blank reasons, duplicate entries, version
  mismatches, stale entries, and user-facing-only entries when decode telemetry
  source preservation is reintroduced.
- **AC9** `BackupSchedule` keeps using `croner`'s detailed parse error for
  useful feedback; no constant-message replacement is required.
- **AC10** The xtask step is wired into both `cargo xtask check` and
  `cargo xtask validate` with a stable step name.
- **AC11** Unit tests cover at least: direct arg enumeration, nested request
  field enumeration, non-`*Request` aggregate enumeration, unsafe string
  interpolation, safe const interpolation, safe numeric scalar interpolation,
  external wrapper rejection without allowlist, telemetry-safe external wrapper
  acceptance with matching version and reason, user-facing-only external wrapper
  acceptance with matching version/reason/sanitized telemetry,
  stale/duplicate/blank/version-drift allowlist failures, and the
  `PasswordError` reachability case.
- **AC12** `cargo xtask validate` is green.

## Out of scope

- General Rust typechecking or a full import resolver.
- Scanning every repository `FromStr` error unrelated to server wire args.
- Changing metric names, `ErrorKind`, `ErrorClass`, event identity,
  `error.context`, or HTTP response body.
- Parsing third-party crate source automatically; reviewed version-pinned
  allowlists are the dependency boundary.
- Sanitizing or removing `BackupSchedule`'s detailed user/client-facing
  validation message.
