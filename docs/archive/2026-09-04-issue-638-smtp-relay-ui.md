# Operator-managed SMTP Relay Configuration

Issue: #638

## Outcome

An operator can inspect and atomically change Jaunder's SMTP Relay configuration
at `/admin/smtp` without exposing the stored password. Saved settings take
effect after Jaunder is restarted through its external service manager.

## Product behavior

- The statically registered `/admin/smtp` route follows the existing admin-route
  denial behavior. The operator sidebar exposes it as `SMTP Relay`; anonymous
  and non-operator sidebars hide the destination, while direct unauthorized
  navigation renders the established authorization error. Both server functions
  independently require operator authorization.
- The page exposes:
  - an enabled toggle;
  - relay host;
  - port;
  - TLS mode (`plain`, `starttls`, or `tls`);
  - sender mailbox;
  - an authentication toggle;
  - username; and
  - a write-only password replacement field.
- With no stored `smtp.host`, the page loads disabled with the established
  defaults: port 587, STARTTLS, and `Jaunder <noreply@localhost>`. Host and
  credentials are blank.
- Loading an enabled configuration returns its non-secret effective values. The
  read model exposes only whether a password is configured; the password is
  never returned, rendered into markup, or used as an input value.
- Saving an enabled configuration validates every visible value through the
  existing shared domain newtypes. The submit control remains disabled until the
  enabled fields and credential state are valid, and touched invalid fields show
  their newtype error locally.
- Authentication is a pair:
  - turning authentication off atomically deletes both username and password;
  - turning it on requires a username;
  - a non-empty password replaces the stored password;
  - a blank password preserves an existing password;
  - when no password exists, authentication cannot be enabled until a valid new
    password is supplied.
- A legacy partial credential pair is presented as authentication enabled and
  cannot be saved as enabled until the missing side is supplied. Saving with
  authentication disabled removes both remnants.
- Disabling SMTP atomically deletes all six SMTP keys: host, port, TLS mode,
  sender, username, and password. No credential or dormant relay settings are
  retained.
- A successful save reports: settings were saved and Jaunder must be restarted
  through its service manager to apply them. Jaunder does not shut down, restart
  itself, or rebuild the injected `MailSender` in this change.

## Secret boundary

- Move the stored, serde-free, SQLx-capable `SmtpPassword` domain type from
  dual-target `common` to host-only `host`, matching the existing
  `common::password::ProfferedPassword` → `host::password::Password` boundary.
  Update every storage, mailer, and test-support consumer in one clean cutover;
  no compatibility re-export remains in `common`.
- Keep the public valueless `InvalidSmtpPassword`, shared non-empty shape
  validator, zero-sized `SmtpPasswordShape`, and new `ProfferedSmtpPassword` in
  `common`. The inbound twin uses `#[str_newtype(secret, serde)]`; every type
  delegates to the same validator and preserves submitted bytes without trimming
  or normalization.
- Implement
  `TryFrom<ProfferedSmtpPassword> for host::smtp_password::SmtpPassword` and
  convert inward immediately at the server boundary. The stored secret retains
  redacting `Debug` and borrowed exposure only; it gains no serde, `Display`,
  `Deref`, equality, or owned-string extraction.
- Bind the browser field through `Field<SmtpPasswordShape>` so validation does
  not retain a second typed secret. Parse the inbound twin only while
  constructing the request, dispatch it, and immediately clear the raw field and
  DOM input on every dispatch; a failed request requires re-entry. Leptos clears
  `ServerAction::input()` when no request remains in flight. Jaunder does not
  write the credential to localStorage or sessionStorage; external password
  manager behavior is outside this boundary.
- The mutation is one cohesive request aggregate and uses
  `#[macros::server(skip_all)]`. Contradictory requests—password supplied while
  SMTP or authentication is disabled—fail with fixed, valueless validation
  errors and perform no write.
- Extend the `proffered-secret` static registry and its positive and negative
  tests so `ProfferedSmtpPassword` is permitted only in its owner, sanctioned
  wasm staging, and a server-function request position. It is forbidden in
  response DTOs, return types, storage/config types, ordinary helpers, and
  telemetry fields.

## Storage and runtime semantics

- The six-key configuration is one storage-owned aggregate. Its mutation seam
  receives explicit disabled/enabled and password keep/replace/clear intent,
  serializes aggregate writers, evaluates a password `Keep` against the current
  `smtp.password` row through the caller-owned `WriteTransaction`, and applies
  all changes inside that same `WriteScope` transaction on SQLite and
  PostgreSQL. A stale `Keep` with no current password fails with a fixed,
  valueless error and writes nothing.
- Convert the existing shared `SiteConfigStorage::get_smtp_config` method—not a
  web-only alternative—to one coherent snapshot so both startup mailer
  construction and the UI observe a complete before-or-after aggregate rather
  than values straddling a concurrent commit.
- Absence of `smtp.host` remains the sole disabled representation consumed by
  mailer construction. Empty strings are never sentinels.
- Missing stored port, TLS mode, and sender retain their existing effective
  defaults. A save writes the operator-selected effective values; it does not
  change those domain defaults.
- The live `MailSender` remains the immutable startup-injected service. The web
  surface edits persisted configuration only. Existing outbound mail continues
  using the startup configuration until an external restart.

## Feedback and failures

- Initial load uses the established settings loading and error states.
- Mutation success uses the established confirmed `MutationOutcome` feedback.
- Authorization, typed wire decode, storage, and transaction failures use the
  existing public error boundary. Password shape, conflict, storage, and
  telemetry errors never include submitted or stored credential bytes.
- The page does not claim to test connectivity or delivery. A bad but
  syntactically valid hostname, credential, or TLS choice can be discovered only
  when the restarted mailer connects.

## Acceptance

- Operator navigation reaches `/admin/smtp`; non-operator and anonymous sidebars
  hide it, direct unauthorized navigation renders the established denial state,
  and both server functions reject non-operator requests.
- Disabled, enabled unauthenticated, enabled authenticated, legacy partial, and
  password-configured read states render without returning the password.
- Client validation covers all typed fields and the conditional credential
  rules. Dispatch clears the password field immediately, no browser storage
  receives it, and direct malformed wire input remains rejected server-side.
- `TryFrom<ProfferedSmtpPassword> for host::smtp_password::SmtpPassword`
  preserves exact bytes and the stored secret remains host-only.
- Password keep, replacement, paired clearing, full disable, and rollback are
  covered on SQLite and PostgreSQL. A stale keep after concurrent password
  deletion fails atomically without creating a partial credential pair.
- `get_smtp_config` has a dual-backend concurrent before-or-after snapshot
  regression covering an aggregate update.
- The read model and every serialized response contain only
  `password_configured`, never either SMTP password type or its bytes.
- Server-function registration, wire-path census, tracing skip policy, and the
  `proffered-secret` gate cover the new API.
- End-to-end coverage proves the real operator route, write-only password UI,
  immediate post-dispatch field clearing, absence from Jaunder browser storage,
  save feedback, persistence after in-app re-entry, paired credential clearing,
  full disable, non-operator denial, and no extra document load.
- Existing mailer construction and SMTP loading tests continue to prove that
  persisted changes become active on the next process start.

## Boundaries

- No in-process mailer reload, self-shutdown, self-restart, or service-manager
  integration. Issue #142 remains deferred.
- No SMTP connectivity test, test-message button, delivery diagnostics, or
  credential verification.
- No arbitrary site-config editor and no change to the CLI's per-key interface.
- No schema migration; the existing `site_config` rows remain authoritative.
- No password readback, placeholder secret, encrypted-at-rest redesign, or
  Jaunder-owned browser persistence of the password. External password-manager
  storage is outside Jaunder's control and is not promised.
- No new visual snapshot state; the dedicated browser flow may add accessibility
  coverage without expanding the repository's fixed visual-state policy.
