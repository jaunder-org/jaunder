# Password Shape Bounds Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

## Review

**Goal:** Enforce one non-normalizing, Unicode-scalar password-length rule of
8–512 values across domain, client, and wire construction.

**Scope:** In: `common::password` constants, typed validation errors, the shared
validator, and its in-file tests. Out: normalization, grapheme counting,
password-strength policy, legacy compatibility, endpoint-specific validation,
request-body limits, and dependency changes.

**Tasks:**

1. Pin the scalar-counting, inclusive bounds, error-secrecy, wire, and
   byte-preservation contracts; implement them in the shared password validator.

**Key risks/decisions:** Count `str::chars()` without normalization; accept 8
and 512, reject 7 and 513; retain exact UTF-8 bytes; keep `Password` and
`ProfferedPassword` on one private validator; use unit error variants so
submitted secrets cannot enter error state.

**Architecture:** Add a private `MAX_LENGTH` beside `MIN_LENGTH`, extend
`PasswordError` with a unit `PasswordTooLong` variant, and make the existing
private validator count Unicode scalar values once before enforcing the
inclusive range. Existing `FromStr` implementations remain the only
domain/client and wire-validation doors, so no caller or web component changes.

**Tech Stack:** Rust 2024, `thiserror`, `serde_json`, Cargo Nextest, jaunder
`cargo xtask` gate.

**Approved spec:**
[`docs/superpowers/specs/2026-08-13-issue-534-password-shape-bounds.md`](../specs/2026-08-13-issue-534-password-shape-bounds.md)

## Global Constraints

- Measure length with `str::chars().count()`; do not use byte length, grapheme
  segmentation, or Unicode normalization.
- Accept 8–512 Unicode scalar values inclusive.
- Preserve the exact submitted UTF-8 bytes for storage and Argon2 input.
- Apply one private validator uniformly through both `Password::from_str` and
  `ProfferedPassword::from_str`; add no endpoint-specific or client-only rule.
- Too-short and too-long errors are distinct typed variants, state their
  respective bound in characters, and carry no submitted password material.
- Add no dependency and no compatibility shim.
- Follow `CONTRIBUTING.md`; run commands through `devtool run --`; no lint or
  coverage suppression.
- Commits use Conventional Commits, reference #534, and contain no trailers.

---

### Task 1: Enforce Unicode-scalar password bounds

**Files:**

- Modify: `common/src/password.rs:6-50` — constants, documentation, typed error,
  and shared validator.
- Test: `common/src/password.rs:146-179` — domain/client parsing, wire decoding,
  non-normalization, and error secrecy.

**Interfaces:**

- Consumes:
  - `impl FromStr for Password` and `impl FromStr for ProfferedPassword`, both
    already calling `validate_password_shape(s)`.
  - `#[str_newtype(secret, serde)]` on `ProfferedPassword`, whose serde bridge
    delegates to `FromStr`.
  - Client `Field<Password>` / `ValidatedInput<Password>` wiring, which
    delegates client validation to `Password::from_str` and remains unchanged.
- Produces:
  - `const MIN_LENGTH: usize = 8;`
  - `const MAX_LENGTH: usize = 512;`
  - Unit variants `PasswordError::PasswordTooShort` and
    `PasswordError::PasswordTooLong`.
  - `fn validate_password_shape(s: &str) -> Result<(), PasswordError>` enforcing
    `MIN_LENGTH <= s.chars().count() <= MAX_LENGTH` without rewriting `s`.

- [x] **Step 1: Replace the loose shape tests with the complete boundary
      contract**

In `common/src/password.rs`'s existing `tests` module, replace the overlapping
`password_accepts_minimum_length`, `password_rejects_too_short`,
`proffered_from_str_valid_and_invalid`, and
`proffered_serde_roundtrips_and_validates_on_the_wire` coverage with the
following tests. Preserve the unrelated redaction, conversion, hashing, and
verification tests.

```rust
#[test]
fn password_types_accept_inclusive_unicode_scalar_bounds() {
    assert_eq!(MIN_LENGTH, 8);
    assert_eq!(MAX_LENGTH, 512);

    let minimum = "é".repeat(MIN_LENGTH);
    let maximum = "a".repeat(MAX_LENGTH);

    assert_eq!(minimum.chars().count(), MIN_LENGTH);
    assert!(minimum.len() > MIN_LENGTH);
    assert!(minimum.parse::<Password>().is_ok());
    assert!(minimum.parse::<ProfferedPassword>().is_ok());

    assert_eq!(maximum.chars().count(), MAX_LENGTH);
    assert!(maximum.parse::<Password>().is_ok());
    assert!(maximum.parse::<ProfferedPassword>().is_ok());
}

#[test]
fn password_types_reject_outside_unicode_scalar_bounds_without_echoing_input() {
    let too_short = "é".repeat(MIN_LENGTH - 1);
    let too_long = "x".repeat(MAX_LENGTH + 1);
    assert_eq!(too_long.chars().count(), MAX_LENGTH + 1);

    let password_short = too_short.parse::<Password>().unwrap_err();
    let proffered_short = too_short.parse::<ProfferedPassword>().unwrap_err();
    assert!(matches!(password_short, PasswordError::PasswordTooShort));
    assert!(matches!(proffered_short, PasswordError::PasswordTooShort));
    assert!(password_short
        .to_string()
        .contains(&format!("at least {MIN_LENGTH} characters")));
    assert!(!password_short.to_string().contains(&too_short));

    let password_long = too_long.parse::<Password>().unwrap_err();
    let proffered_long = too_long.parse::<ProfferedPassword>().unwrap_err();
    assert!(matches!(password_long, PasswordError::PasswordTooLong));
    assert!(matches!(proffered_long, PasswordError::PasswordTooLong));
    assert!(password_long
        .to_string()
        .contains(&format!("at most {MAX_LENGTH} characters")));
    assert!(!password_long.to_string().contains(&too_long));
}

#[test]
fn password_validation_does_not_normalize_before_counting() {
    let decomposed = "e\u{301}".repeat(7);
    assert_eq!(decomposed.chars().count(), 14);

    let password: Password = decomposed.parse().unwrap();
    let proffered: ProfferedPassword = decomposed.parse().unwrap();
    assert_eq!(password.as_ref(), decomposed);
    assert_eq!(proffered.as_ref(), decomposed);
}

#[test]
fn proffered_serde_enforces_unicode_scalar_bounds() {
    let accepted: ProfferedPassword = "password123".parse().unwrap();
    assert_eq!(serde_json::to_string(&accepted).unwrap(), "\"password123\"");
    let roundtrip: ProfferedPassword = serde_json::from_str("\"password123\"").unwrap();
    assert_eq!(roundtrip.as_ref(), "password123");

    let too_short = serde_json::to_string(&"é".repeat(MIN_LENGTH - 1)).unwrap();
    let too_long = serde_json::to_string(&"a".repeat(MAX_LENGTH + 1)).unwrap();
    assert!(serde_json::from_str::<ProfferedPassword>(&too_short).is_err());
    assert!(serde_json::from_str::<ProfferedPassword>(&too_long).is_err());
}
```

These assertions pin every validator branch: below minimum, inclusive minimum,
inclusive maximum, above maximum, plus exact-byte preservation and serde
delegation. The unit-variant patterns also make it impossible to add password
material to the new validation errors without changing the contract.

- [x] **Step 2: Run the focused tests and verify the red state**

Run:

```bash
devtool run -- cargo nextest run -p common password::tests
```

Expected: FAIL to compile because `MAX_LENGTH` and
`PasswordError::PasswordTooLong` do not exist. Do not weaken the tests to reach
a runtime failure.

- [x] **Step 3: Implement the shared bounded validator**

In `common/src/password.rs`:

1. Keep `MIN_LENGTH` at 8 and add `MAX_LENGTH` at 512.
2. Update `Password` and validator rustdoc to state the inclusive 8–512
   Unicode-scalar invariant and exact-byte/no-normalization behavior.
3. Add the unit variant below; it intentionally carries no input or measured
   length.

```rust
#[error("password must be at most {MAX_LENGTH} characters")]
PasswordTooLong,
```

4. Compute `let length = s.chars().count();` exactly once in
   `validate_password_shape`.
5. Return `PasswordTooShort` when `length < MIN_LENGTH`, `PasswordTooLong` when
   `length > MAX_LENGTH`, and `Ok(())` otherwise. Do not allocate, normalize, or
   copy `s` inside validation.
6. Leave both `FromStr` implementations, the `TryFrom<ProfferedPassword>`
   conversion, serde derive, and all web `Field<Password>` call sites unchanged;
   they already converge on this validator.

- [x] **Step 4: Run the focused tests and verify the green state**

Run:

```bash
devtool run -- cargo nextest run -p common password::tests
```

Expected: PASS for every `password::tests` test, including hashing/redaction
regressions outside the new boundary tests.

- [x] **Step 5: Run the per-commit gate**

Read and follow `jaunder-commit`, then run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS. If Fix mode formats either changed Markdown or Rust file,
restage the formatted content and rerun until the tree is unchanged and green.
Do not suppress lints or coverage.

- [x] **Step 6: Commit the independently complete behavior change**

Stage exactly the implementation, tests, approved spec, and this tracked plan:

```bash
git add common/src/password.rs docs/superpowers/specs/2026-08-13-issue-534-password-shape-bounds.md docs/superpowers/plans/2026-08-13-issue-534-password-shape-bounds.md
git commit -m "fix(common): bound password scalar length (#534)"
```

The pre-commit hook reruns `cargo xtask check`; commit only the exact staged
tree that passed. Do not add a `Co-Authored-By` or any other trailer.
