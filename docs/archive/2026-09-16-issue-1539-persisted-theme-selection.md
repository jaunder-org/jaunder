# Persist persisted Theme Package selection on cold load

## Outcome

On a cold Studio `/themes` entry, the Public selection control displays the
author’s persisted Theme Package selection once the catalog and selection
resources resolve. Inherit, built-in, and custom selection behavior remains
unchanged.

## Load-bearing decisions

- The server-provided public theme selection remains the authoritative value for
  the Studio selection control.
- The control has exactly one reactive selection mechanism; option elements do
  not independently declare selected state.
- This is a client rendering correction only. Persistence, server-function
  contracts, author/site precedence, and Theme Package lifecycle semantics do
  not change.
- Resource settlement may occur in either order: the control must converge when
  the selected custom option becomes available after the selection value, or
  vice versa.
- The existing custom-theme lifecycle’s fresh browser context remains the
  focused regression scenario because it exercises a new document with persisted
  server state rather than retained in-page state.

## Acceptance

- After an author publishes and selects a custom Theme Package, a fresh
  authenticated context entering `/themes` shows that Theme Package as the
  Public selection value.
- The inherit option remains selected when an author has no override.
- Built-in and custom selections still submit, revalidate, and display their
  confirmed server state.
- The selection markup uses no competing option-level selected state alongside
  the select-level controlled value.
- The focused `theme-management.spec.ts` lifecycle proof passes without
  increased assertion or whole-test timeouts.
- The authoritative PostgreSQL/Firefox E2E lane that exposed the regression
  passes.

## Boundaries

- No loading-state redesign or new intermediate selection UI.
- No changes to Theme Package storage, APIs, catalog ownership, publication, or
  removal behavior.
- No new selection semantics, fallback rules, or theme terminology.
- No broad refactor of the Theme Studio component beyond what is required to
  restore one authoritative control value.
