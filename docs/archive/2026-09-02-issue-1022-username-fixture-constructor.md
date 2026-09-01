# Issue #1022: Reuse the username fixture constructor

## Problem

`end2end/tests/auth.spec.ts` and `end2end/tests/invite.spec.ts` each reproduce
the timestamp-plus-random username formula already owned by
`helpers::generateUsername`. The duplicated formulas differ only by their
semantic prefixes.

## Decision

Import `generateUsername` from `./helpers` in both specs and replace the two
audited formulas:

- authentication registration holdout: `generateUsername("newuser")`;
- invite-link registration: `generateUsername("invitee")`.

The helper produces the same prefix, decimal `Date.now()` value, and
six-character base-36 random slice as both inline formulas. No helper or
public/test interface changes.

## Preserved boundaries

Keep each distinct UI flow unchanged:

- the authentication test enters `/register`, fills `newpassword123`, submits,
  and asserts logout readiness with no registration error;
- the invitation test creates and follows an invite-code URL in a separate
  traced browser context, fills `testpassword123`, races logout readiness
  against a detailed error, and closes the context in `finally`.

Keep the `pending${Date.now()}` and `failure${Date.now()}` values in the
registration concurrency/error tests explicit. They do not duplicate the full
timestamp-plus-random constructor and their prefixes describe the scenario under
test.

## Verification

Run focused e2e tests for the authentication registration holdout and
invite-link registration, then run `cargo xtask check`. Review the final diff
against repository standards, the issue, and this specification.

## Non-goals

- Adding or changing a username helper.
- Migrating scenario-specific timestamp-only usernames.
- Consolidating the two registration flows.
- Changing registration, invitation, navigation, password, timeout, error, or
  browser-context behavior.
