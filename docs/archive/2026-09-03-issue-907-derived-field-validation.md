# Issue #907: Derived field validation

## Outcome

Every web form field derives its validation state from its current input, so a
programmatic value write cannot leave rendered validity, submit gating, and
parsing based on different field state.

## Load-bearing decisions

- `Field<T>` privately owns its input value and validation error. Consumers
  cannot mutate either reactive primitive directly.
- The validation error is a read-only reactive value derived from the current
  input and the field's required/optional policy. It is never stored or updated
  independently of that value; reactive memoization of the derivation is
  allowed.
- `Field` exposes the smallest consumer API needed by current forms: a value
  snapshot, a value setter, a read-only error handle, parsing, validity,
  touching, and reset. The standard forms renderer alone may access the private
  value signal needed for DOM reactivity.
- The synchronization-era `set_input` operation and public arbitrary-input
  validation operation are removed. Programmatic seeding uses the ordinary value
  setter; validity follows automatically without marking the field touched.
- Required input remains valid exactly when it parses as its domain newtype.
  Optional blank or whitespace-only input remains valid absence and parses to no
  value; this intentional distinction is not a stale-state disagreement.
- `is_valid()`, `parsed()`, and the rendered error all derive from the same
  current input. No submit path may retain a separately written validation cache
  or a proxy validation rule.
- `Field<T>` remains a `Copy` handle whose copies observe the same underlying
  reactive state.
- ADR-0065, ADR-0113, and the architecture projection are updated from their
  synchronization-era wiring and documented follow-up/debt state to the enforced
  derived-validation state.
- ADR-0113 continues to prohibit independently authored gate and payload
  validation. It permits `Field::is_valid()` gating alongside `Field::parsed()`
  payload assembly after this change because both operations derive from the
  same private current value; optional valid absence remains a deliberate `None`
  payload, not a swallowed parse failure. This fulfills an existing decision
  rather than creating a new architectural decision.

## Acceptance

- A value-only programmatic write immediately changes `is_valid()`, `parsed()`,
  and the read-only error consistently for required and optional fields.
- A consumer cannot write `Field`'s value or error primitive directly; every
  production consumer uses the encapsulated API or the standard validated input
  components.
- Reset restores an empty required field to invalid and an empty optional field
  to valid absence, and clears touched state in both cases.
- Prefilled fields and copied `Field` handles retain their existing behavior,
  including shared updates and untouched initial state.
- Existing validated input and textarea components display the domain newtype's
  error after touch and expose no independently writable error path.
- Post slug and summary gates, and every other `Field` consumer in `web/src`,
  use only derived validity and parsing from the same private current value. No
  consumer retains a separately writable validity proxy or a silent parse
  failure arm.
- Host tests cover direct programmatic writes, required and optional semantics,
  reset, prefill, copy aliasing, and the submit-gate contracts affected by the
  API cutover.
- Relevant focused web tests and the repository's static checks pass.

## Boundaries

- Do not change domain newtype parsing rules, validation messages, touched-state
  display policy, form markup, or server-side validation.
- Do not redesign the erased `Labelled`/`validated_error` interface settled by
  ADR-0117.
- Do not add a compatibility alias for `set_input` or preserve direct field
  access through public signal getters.
- No schema, endpoint, protocol, dependency, or ubiquitous-language change is in
  scope.
