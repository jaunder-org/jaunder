# Issue #1138 Sink-Specific Telemetry Implementation Outline

> Execute with `jaunder-iterate`, delegating individual tasks through
> `jaunder-dispatch` when useful. This outline exists because the change creates
> durable type/macro policy boundaries for trace and user-facing sinks.

## Scope

In:

- Type-owned zero-allocation trace projections and macro-generated span fields.
- Removal of trace type-name admission while retaining source grammar/skip
  enforcement.
- Owned user-facing parse-error messages, stable telemetry codes, and a marked
  external-format door.
- Removal of foreign-display/version admission while retaining decode telemetry
  sanitization.
- A draft ADR plus architecture projection of the new policy mechanism.

Out:

- New trace-admission grounds, automatic omission of non-implementers, removal
  of author-intent `skip`/`skip_all`, generic string/`Debug`/`Display` tracing,
  raw foreign error text in telemetry, decode-stage behavior changes, schema or
  UI redesign.

## Task outline

- [x] Task 1: Add type-owned trace projections
  - Contract: export `common::trace_field::TraceField` with the approved GAT
    signature. Implement only the exact primitive, domain, `Option<T>`, and `&T`
    projections from the spec; every projection is by value or borrow and
    allocates nothing. Keep excluded types without an implementation.
  - Verification: `devtool run -- cargo xtask test-local -- -p common` proves
    primitive/domain values, recursive `Option<T>` and references, exact `Debug`
    content, and excluded policy boundaries where compile-time coverage
    conventions permit.

- [x] Task 2: Generate projected server-fn fields and remove name admission
  - Contract: `macros::server` always instruments original parameters with
    generated `skip_all`, then emits
    `field = ?::common::trace_field::TraceField::trace_value(&field)` for every
    unskipped identifier. Source `skip(name)` omits one field; source `skip_all`
    omits all generated parameter fields but preserves manual
    `fields(... = tracing::field::Empty)` declarations. Pattern parameters still
    require source `skip_all`. Delete `RECORDABLE_TYPES`, name reduction, and
    recovery/tests that classify type names; retain enumeration and all
    grammar/skip/declaration checks.
  - Verification: `devtool run -- cargo xtask test-local -- -p macros server_fn`
    covers traceable and `Option<T>` parameters, skipped non-implementers,
    source `skip_all`, patterns, and combined
    `skip_all, fields(... = tracing::field::Empty)`;
    `devtool run -- cargo test --manifest-path xtask/Cargo.toml server_fn_tracing_check`
    proves the reduced static policy remains fail-closed.

- [ ] Task 3: Introduce owned parse-error sink surfaces
  - Contract: export `common::UserFacingMessage` with only `from_external` and
    `as_str`, redacted exact `Debug`, and no `TraceField`. `InvalidEmail`
    discards its foreign source and exposes the exact static user message/code.
    `InvalidBackupSchedule` stores the wrapper, preserves detailed Croner text
    through `user_message`/`Display`, exposes only the static telemetry code,
    and constructs the wrapper through the exact immediately marked source door.
  - Verification: `devtool run -- cargo xtask test-local -- -p common` proves
    stable email behavior and detailed/redacted backup behavior;
    `devtool run -- cargo xtask test-local -- -p web forms::field` proves the
    existing client invalid-cron assertion remains green.

- [ ] Task 4: Replace external-display admission with owned-surface enforcement
  - Contract: `server-fn-wire-arg-error` recognizes owned
    `user_message`/`telemetry_code` surfaces and derives a census of immediately
    marked `UserFacingMessage::from_external` calls. Delete the external-display
    entries/categories, Cargo.lock parsing, version/liveness rules, and their
    recovery/tests. Retain aggregate expansion, reachable `FromStr::Err`
    discovery, literal/primitive owned display, and the exact source-free decode
    telemetry invariant.
  - Verification:
    `devtool run -- cargo test --manifest-path xtask/Cargo.toml server_fn_wire_arg_error_check`
    proves marked pass, unmarked failure, and stale/bare/trailing/shared/orphan
    marker failures;
    `devtool run -- cargo xtask test-local -- -p web error::server` proves the
    submitted marker cannot enter `Validation`/`Client`, `stage=decode`, public
    `invalid request arguments` telemetry.

- [ ] Task 5: Project and certify the policy replacement
  - Contract: add tracked draft
    `docs/adr/drafts/sink-specific-telemetry-interfaces.md` with the canonical
    `# ADR-DRAFT` heading and `Status: proposed`. It supersedes ADR-0011's
    `RECORDABLE_TYPES` mechanism while retaining the four admission grounds and
    accounting for ADR-0147. Add only a reciprocal past-tense navigation
    annotation to ADR-0011. Update `docs/ARCHITECTURE.md` with a draft-path
    citation plus `TraceField`, generated field ownership, owned parse-error
    surfaces, and source-site markers. Leave ADR-0065 unchanged unless
    implementation alters its decode contract. Do not promote/renumber the draft
    or edit `docs/README.md` on the feature branch.
  - Verification: explicit diff review proves the draft path, heading, proposed
    status, reciprocal annotation, architecture citation, unchanged ADR index,
    and absence of a numbered promotion. `devtool run -- cargo xtask check`
    proves host and wasm/server compilation, real server-fn trait coverage, both
    static gates, changed behavior tests, and no unapproved policy escape.

## Risk checks

- Trait resolution is the default-deny compile-time assertion; the procedural
  macro never infers implementations or silently skips non-implementers.
- Original parameter `Debug` is always hidden from `tracing::instrument` before
  projected fields are added.
- Every current source `skip`/`skip_all` remains an author-intent opt-out;
  pattern-bound and manual decision-field behavior does not drift.
- No blanket implementation admits free strings, generic formatters, secrets,
  email, arbitrary content, tokens, or request aggregates.
- Foreign error text crosses only the immediately marked
  `UserFacingMessage::from_external` door and cannot implement the trace sink.
- `emit_arg_decode_failure` remains fixed, source-free, and
  submitted-input-free.
- The draft ADR and architecture view ship together; numbered ADR decision text
  is not rewritten as present-state documentation.
