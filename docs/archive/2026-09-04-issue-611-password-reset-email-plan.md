# Username-or-Email Password Reset Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for bounded tasks.
> This outline exists because issue #611 changes an enumerable-identifier
> security boundary, detached-work concurrency, a storage interface, and the
> generated server-function wire contract.

## Scope

In:

- A password-reset-scoped typed Username-or-Email request value and one mixed
  identifier form control.
- Shared SQLite/PostgreSQL lookup of all Users by exact canonical Email.
- Immediate neutral responses with isolated detached reset-token/mail work.
- Server, wire, browser, architecture, and flow-document proof.

Out:

- Email uniqueness or migration, a durable outbox, retries, delivery status,
  rate limiting, and reset-confirmation behavior changes.
- Registration, login, Email verification, SMTP configuration, and other mail.
- A new ADR or `CONTEXT.md` term.

## Task outline

- [x] Task 1: Add the typed identifier and Email lookup contract
  - Contract: the password-reset request value is exactly
    `Username(Username) | Email(Email)`; `@` selects Email parsing and all other
    input selects Username parsing. `UserStorage::get_users_by_email(&Email)`
    returns every exact canonical match without imposing uniqueness.
  - Verification: value tests prove classification, Username canonicalization,
    Email local-part/domain behavior, and malformed-input rejection; shared
    `#[apply(backends)]` storage tests prove zero, one, duplicate, verified, and
    unverified rows remain observable to the caller on both backends.

- [x] Task 2: Cut over the neutral request and detach delivery
  - Contract: the existing endpoint, form, and every generated-wire caller move
    atomically to the typed `identifier` field and a `WebResult<()>` response;
    unit means only that a structurally valid request was accepted, never that
    account-dependent work completed. The form is labelled `Username or email`
    and performs no unconditional lowercase transform. The request captures
    owned `UserStorage`, `PasswordResetStorage`, `WriteScope`,
    `SiteConfigStorage`, mailer, and host error-reporting dependencies, then
    spawns before lookup or base-URL resolution. Detached work filters Email
    matches by verified status and processes each eligible User independently.
  - Verification: dual-backend server integration tests cover Username and Email
    selection, duplicate verified Email fan-out, exclusion of unverified
    matches, per-User failure isolation, stable wire routing, and full caller
    cutover. Gated lookup and token dependencies expose separate entered,
    release, and worker-terminal signals: observe entry, observe the public
    response while held, release work, await its terminal signal, then assert
    mail and telemetry. Lookup, base-URL, token, and mail failures each produce
    a bounded `swallowed/server` report with no Email, token, password, or
    secret fields and never change the public result.

- [x] Task 3: Prove the end-to-end path and update current-truth docs
  - Contract: no legacy username request field remains. `docs/ARCHITECTURE.md`
    and `docs/flows/password-reset.md` describe the username-or-email form,
    neutral detached request boundary, best-effort delivery, and retained
    confirmation boundary.
  - Verification: browser coverage proves the accessible form behavior and
    mixed-case Email preservation; the Email-based e2e request receives and
    consumes its absolute reset link; the existing Username end-to-end flow
    remains green.

## Risk checks

- Structurally valid matched and unmatched requests traverse the same public
  response path; no account-dependent future is awaited before return.
- Detached work owns every dependency and value it uses. No Leptos context,
  borrowed transaction, or request lifetime escapes into the task.
- A `Confirmed` token is mailed. A `CommitIndeterminate` token is also mailed,
  preserving the existing chance of successful recovery when the commit may have
  landed; `WriteScope` is the sole reporter of its acknowledgement failure, so
  detached work emits no duplicate swallowed report. Confirmed, indeterminate,
  and rollback-confirmed outcomes have explicit coverage.
- One matching User's write or mail failure cannot cancel later matching Users.
- Email matching uses the existing canonical Email value exactly; no hidden
  case-folding or uniqueness assumption is introduced.
- Email addresses and reset tokens never enter tracing fields, error text,
  metrics labels, snapshots, or public responses.
- Every storage behavior change is exercised through the shared generic store on
  SQLite and PostgreSQL.
- The generated server-function path stays stable while every request field
  caller changes cleanly to the typed identifier.
- Each completed task reaches `jaunder-commit`; no commit includes a
  `Co-Authored-By` trailer.
