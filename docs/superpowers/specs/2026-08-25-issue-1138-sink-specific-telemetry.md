# Issue #1138 — Sink-specific telemetry interfaces

## Outcome

Tracing admissibility and parse-error exposure become interfaces owned by the
value or error type instead of name/version allowlists in xtask. Existing useful
telemetry and operator-facing validation remain, while secrets, arbitrary
content, rejected raw input, and foreign error text stay out of traces.

## Load-bearing decisions

- Preserve ADR-0011's four existing trace-admission grounds: a value may be
  recorded when its type is intrinsically bounded, it is operator configuration,
  it is already public in a permalink, or it is `Username`. This change moves
  policy ownership; it does not broaden admission.
- `common::trace_field::TraceField` is the sole value-level interface generated
  server-fn instrumentation may use:

  ```rust
  pub trait TraceField {
      type Value<'a>: std::fmt::Debug
      where
          Self: 'a;

      fn trace_value(&self) -> Self::Value<'_>;
  }
  ```

- `bool` and `u32` project by value. `Option<T>` projects with
  `self.as_ref().map(TraceField::trace_value)`. `&T` delegates to `T`. No other
  blanket implementation exists.
- These exact types project as `&Self`, preserving today's `Debug` content
  without allocation: `PostId`, `AudienceId`, `SubscriptionId`, `ContentHash`,
  `PageSize`, `PageOffset`, `RetentionCount`, `InviteTtlHours`, `UtcInstant`,
  `PageCursor`, `PostFormat`, `MediaSource`, `BackupMode`, `DestinationPath`,
  `SiteTitle`, `BaseUrl`, `BackupSchedule`, `Slug`, `PermalinkDate`, `Tag`, and
  `Username`.
- No `TraceField` implementation is added for `Filename`, `AudienceName`,
  `Email`, `Bio`, `DisplayName`, `SessionLabel`, tokens, request aggregates,
  free `String`, generic `Debug`, or generic `Display`.
- `#[macros::server]` instrumentation always applies `skip_all` to original
  parameters, then adds one generated field per unskipped identifier using
  `field = ?::common::trace_field::TraceField::trace_value(&field)`. The
  expression is the compile-time assertion: an unskipped non-implementer cannot
  compile, and its generic `Debug` never reaches a span.
- Existing source `skip(name)` omits that generated field; source `skip_all`
  emits none. Pattern-bound parameters still require source `skip_all`.
  Declaration-only `fields(name = tracing::field::Empty)` remain supported.
- The `server-fn-tracing` static check retains server-fn enumeration,
  legal-key/default-deny grammar, skip-name and pattern checks, and
  declaration-only field checks. It performs no type-name classification and
  owns no `RECORDABLE_TYPES`, `is_recordable`, or `reduce_type` policy.
- Trace policy remains unchanged: usernames, public permalink values, bounded
  values, and operator configuration remain traceable. Secrets, email, arbitrary
  content, request aggregates, and every existing `skip`/`skip_all` site remain
  unrecorded.
- `common::UserFacingMessage` is an owned string wrapper with exactly
  `from_external(value: impl Display) -> Self` and `as_str(&self) -> &str` as
  public methods. Construction stores `value.to_string()`; `Debug` renders
  exactly `UserFacingMessage([redacted])`. It does not implement `TraceField`.
- Every `UserFacingMessage::from_external` call requires an immediately
  preceding, non-empty source marker
  `// server-fn-wire-arg-error:allow <reason>`. Bare, trailing, shared, stale,
  or orphan markers fail the static check.
- `InvalidEmail` discards `email_address::Error` text at the parse boundary. It
  exposes `pub fn user_message(&self) -> &'static str`, returning
  `"invalid email address"`, and `pub fn telemetry_code(&self) -> &'static str`,
  returning `"invalid_email"`. `Display` delegates only to `user_message`.
- `InvalidBackupSchedule` stores `UserFacingMessage`. It exposes
  `pub fn user_message(&self) -> &str`, preserving the current detailed Croner
  feedback with prefix `"invalid backup schedule: "`, and
  `pub fn telemetry_code(&self) -> &'static str`, returning
  `"invalid_backup_schedule"`. `Display` delegates only to `user_message` and
  `Debug` remains redacted through the wrapper.
- The one intentional Croner conversion uses the marked source door
  `UserFacingMessage::from_external(format_args!("invalid backup schedule: {error}"))`.
  Detailed feedback reaches only the submitting operator. Neither the wrapper
  nor either parse error gains a tracing-safe interface.
- Decode-stage server-fn telemetry remains source-free and unchanged:
  `error.public = "invalid request arguments"` and `stage = "decode"`. Raw
  submitted input, detailed user messages, and foreign sources never enter that
  telemetry.
- The `server-fn-wire-arg-error` static check retains server-fn aggregate
  expansion, reachable `FromStr::Err` discovery, literal/primitive owned-display
  classification, and the exact decode-sanitization invariant. It recognizes
  owned `user_message`/`telemetry_code` surfaces and source-marker census, but
  owns no external-display allowlist, category enum, or Cargo.lock version pin.

## Acceptance

- `common` tests prove every approved zero-allocation trace projection,
  recursive `Option<T>` and `&T`, stable email message/code behavior, detailed
  backup feedback, and redacted backup/wrapper debug output.
- Macro expansion tests prove generated `skip_all` plus projected fields for a
  traceable parameter and `Option<T>`, omission of a skipped non-implementer,
  source `skip_all`, and pattern handling. A combined
  `skip_all, fields(... = tracing::field::Empty)` case proves `skip_all`
  suppresses only generated parameter fields and preserves manual declarations.
- Real host and wasm/server compilation proves every unskipped server-fn
  parameter implements `TraceField`; no original parameter `Debug` is recorded.
- `server-fn-tracing` tests prove grammar/skip/declaration enforcement remains
  fail-closed and that all type-name admission code, recovery text, and tests
  are gone.
- `server-fn-wire-arg-error` tests prove unmarked external formatting fails; one
  correctly marked conversion passes and enters the derived census; stale, bare,
  trailing, shared, and orphan markers fail; no central foreign-display or
  version allowlist remains.
- Email exposes only `"invalid email address"` and `"invalid_email"`, without
  storing or formatting the foreign source.
- `BackupSchedule` preserves detailed Croner feedback through
  `user_message`/`Display`; its `Debug` and telemetry code contain no raw text.
  The existing client field assertion remains green.
- `web::error::server::emit_arg_decode_failure` still emits
  `Validation`/`Client`, `stage = "decode"`, and public
  `"invalid request arguments"` for a unique submitted marker, with no marker in
  telemetry.
- A new numberless draft ADR supersedes ADR-0011's `RECORDABLE_TYPES` storage
  mechanism while retaining its four admission grounds and accounting for
  ADR-0147's reference to that gate. ADR-0011 receives only a permitted
  past-tense reciprocal navigation annotation. `docs/ARCHITECTURE.md` projects
  `TraceField`, generated fields, owned parse-error surfaces, and source-site
  external-format markers. ADR-0065 changes only if its decode-stage contract
  changes.

## Boundaries

- No broader definition of trace-safe data; no generic `Debug`, `Display`, or
  string admission; no new recording of existing skipped parameters.
- No raw email error detail and no raw Croner detail in telemetry. Detailed
  backup feedback remains user-facing only.
- No change to decode-stage public classification, stage name, or source-free
  behavior.
- No schema, protocol, storage, metrics, retry, localization, or UI redesign.
