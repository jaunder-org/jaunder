# Issue 1034: Centralize Public Unit Error Emission

## Issue

[refactor(macros): centralize public unit error emission](https://github.com/jaunder-org/jaunder/issues/1034)

## Context

The repository-wide boilerplate audit found that `macros/src/num_newtype.rs` and
`macros/src/text_enum.rs` each generate the same public unit error policy:

- a public unit struct;
- `Debug`, `Clone`, `Copy`, `PartialEq`, and `Eq` derives;
- an automatically-derived `Display` impl that writes the already-resolved
  message tokens;
- an automatically-derived `std::error::Error` impl;
- fully-qualified emitted paths such as `::core::fmt`, `::core::clone`,
  `::core::marker`, `::core::cmp`, and `::std::error`.

The duplicated code lives in the two private `error_type` helpers:
`num_newtype::error_type` and `text_enum::error_type`. Their callers still
differ in the inputs they own: `NumNewtype` derives `Invalid<Name>` and a
numeric bounds message, while `#[text_enum]` receives an explicit public error
identifier and message and also needs caller-specific rustdoc mentioning the
enum.

ADR-0091 is load-bearing here. It records that the generated closed-string-enum
error deliberately avoids `thiserror`, remains a public unit struct, and must be
constructible as a bare unit expression because `host` registers these
validation errors by name and tests construct them directly.

## Decision

Add a shared private codegen helper in `macros/src/lib.rs` for this public unit
error shape. The helper will accept:

- the public error type identifier;
- the rustdoc text to put on that type;
- the already-resolved `Display` message token stream.

`num_newtype::error_type` and `text_enum::error_type` will become thin
caller-specific adapters that compute those inputs and delegate to the shared
helper. No other macro family is in scope for this issue.

The helper must preserve the emitted public token shape. In particular, the
generated error type must remain a public unit struct with the same derive list,
the same emitted paths, the same `Display` behavior, the same
`std::error::Error` impl, and the same ability to be constructed as a bare unit
expression.

The caller-owned details remain caller-owned:

- `NumNewtype` still derives the public error name as `Invalid<Name>`.
- `NumNewtype` still uses its current explicit or generated numeric bounds
  message.
- `#[text_enum]` still uses the explicitly declared `error = <Ident>` and
  `message = "..."` options.
- `#[text_enum]` still emits rustdoc tying the parse error to the enum name.
- The `#[text_enum]` parse error function still returns the bare unit
  expression.

## Non-Goals

- Do not change any public macro arguments.
- Do not rename generated error types.
- Do not change generated error messages or docs except for token-equivalent
  formatting changes caused by shared codegen.
- Do not change emitted paths or visibility for the generated public error types
  and impls.
- Do not introduce `thiserror` or any new macro-crate dependency.
- Do not broaden the helper to unrelated hand-written error types outside the
  audited macro callers.
- Do not reopen ADR-0091.

## Acceptance Criteria

- `num_newtype::error_type` and `text_enum::error_type` both route through the
  shared helper in `macros/src/lib.rs`.
- `#[text_enum]` generated errors preserve the current observable behavior:
  public unit struct, derive list, declared `Display` message, `Error` impl, and
  bare unit construction in the parse error function, with the same emitted
  paths as today.
- `NumNewtype` generated errors preserve the current observable behavior: public
  unit struct, derive list, explicit-or-generated `Display` message, `Error`
  impl, and `Invalid<Name>` naming, with the same emitted paths as today.
- Existing public tests and doctests keep passing without changing call sites.
- Focused macro verification passes for the affected surfaces.
- `cargo xtask check` passes.

## Verification Plan

- Inspect the implementation diff to confirm the duplicated error-shape code now
  lives in one helper in `macros/src/lib.rs` and the two audited `error_type`
  adapters delegate to it rather than retaining local copies.
- Add or adjust focused macro tests so the shared public unit error shape is
  asserted for both `#[text_enum]` and `NumNewtype`.
- Run the focused macro tests, including `macros` tests that exercise
  `text_enum` and `num_newtype`.
- Run `cargo xtask check`.
