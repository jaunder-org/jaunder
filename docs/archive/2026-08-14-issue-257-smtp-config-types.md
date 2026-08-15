# Issue #257: Move the SMTP configuration aggregate to the host layer

- Status: Draft
- Issue: [#257](https://github.com/jaunder-org/jaunder/issues/257)
- Date: 2026-08-14

## Context

`storage/src/smtp.rs` defines `SmtpConfig`, although the type itself has no
persistence dependency. It is the validated aggregate consumed by the host-side
SMTP mailer and returned by `SiteConfigStorage::get_smtp_config`. Only the
storage reader and its error classification depend on storage.

The issue originally included `SmtpTlsMode`. Issue #687 already moved that value
type and its parser/display/error implementation to `common::smtp_tls_mode`, but
`storage::smtp` still re-exports it and repeats its parsing/display tests.

ADR-0058 distinguishes the shared crate floors by target: `common` is
target-agnostic host-and-wasm code, while `host` owns strictly host-focused
shared code and may depend on no workspace crate above `common`. SMTP relay
configuration is used only by `storage` and `server`; both already depend on
`host`. It is not a web form or wire DTO. Jaunder currently configures SMTP
through typed `SiteConfigKey` CLI writes and assembles `SmtpConfig` only when
host storage reads the settings.

## Decisions

### D1. Home the aggregate in `host`

Move `SmtpConfig` to a new `host::smtp_config` module. Preserve its fields,
field types, visibility, derives, behavioral guarantees, and semantics exactly:

- `SmtpHost` relay host;
- `SmtpPort` relay port;
- `SmtpTlsMode` transport-security mode;
- optional `SmtpUsername` and `SmtpPassword` credentials;
- `SmtpSender` sender mailbox.

The module depends only on `common` value types, preserving ADR-0058's
host-floor invariant. `host/src/lib.rs` exposes the module alongside the crate's
other focused tenants. Rewrite the aggregate's documentation in layer-local
terms: it may describe the validated fields and host-side relay role, but it
cannot link to or name `storage::SiteConfigStorage`. Keep the storage-specific
assembly guarantee and intra-doc links beside
`SiteConfigStorage::get_smtp_config` and `load_smtp_config`, where those
concepts are owned.

`SmtpConfig` does not move to `common`: it describes strictly host-side relay
configuration and has no browser or target-agnostic contract. A future SMTP web
form would use typed wire arguments or a dedicated request DTO and assemble the
host aggregate server-side; it would not serialize the stored aggregate across
the wasm boundary.

### D2. Keep storage behavior in `storage`

Keep these in `storage::smtp`:

- `SmtpConfigError`;
- `load_smtp_config`;
- the private SQLx-error classifier;
- tests of storage reads, defaults, invalid values, credential redaction, and
  error classification.

`SiteConfigStorage::get_smtp_config` continues returning `Option<SmtpConfig>`,
importing the aggregate from `host`. Database queries, defaults, decode
behavior, credential disclosure boundaries, and public error messages do not
change.

### D3. Complete the clean cutover

Remove the `storage` compatibility re-exports of `SmtpConfig`, `SmtpTlsMode`,
and `InvalidSmtpTlsMode`. Production and test callers import their owners
directly:

- `host::smtp_config::SmtpConfig`;
- `common::smtp_tls_mode::SmtpTlsMode` where the enum is named.

Delete the redundant `storage::smtp` parsing/display tests for `SmtpTlsMode`;
`common::smtp_tls_mode` already owns equivalent round-trip, default, and
rejection coverage. Keep storage tests that use the enum to prove SMTP
configuration read behavior.

No deprecated aliases or compatibility shims remain.

### D4. Preserve observable behavior

This is a type-home and import cutover only. It does not change configuration
keys, CLI input, stored values, SQL, defaults, mailer construction, TLS
selection, credential handling, public errors, or any web interface.

No new behavioral test is required. Existing focused `common`, `storage`, and
`server::mailer` tests are the regression proof; the repository gate proves both
storage backends and target layering remain valid.

## Non-goals

- Adding an SMTP web settings interface or wire request type.
- Changing the individual SMTP value types or moving them out of `common`.
- Redesigning `SiteConfigStorage` or its SQL queries.
- Changing password update semantics, SMTP defaults, validation, or error
  messages.
- Addressing whether `SmtpCredentials` is redundant to `SmtpConfig` (#673).
- Moving unrelated host-only modules tracked by #855.
- Recording a new ADR or glossary term; ADR-0058 already decides the applicable
  layer.

## Acceptance criteria

1. `SmtpConfig` is defined only in `host::smtp_config`, with the same fields,
   field types, visibility, derives, behavioral guarantees, and documentation
   semantics as before; its documentation contains no storage-layer reference.
2. `host::smtp_config` depends only on `common` workspace types and preserves
   ADR-0058's host-floor dependency invariant.
3. `SiteConfigStorage::get_smtp_config`, `load_smtp_config`, and the SMTP mailer
   use the moved aggregate without changing signatures beyond its module path or
   changing runtime behavior.
4. `SmtpConfigError`, SQLx error classification, storage reads, defaults, and
   credential-redaction behavior remain in `storage` and retain their existing
   observable results.
5. No `storage` re-export or compatibility alias remains for `SmtpConfig`,
   `SmtpTlsMode`, or `InvalidSmtpTlsMode`; every caller imports the owning
   module directly.
6. Redundant TLS parser/display tests are removed from `storage`; the owning
   `common::smtp_tls_mode` tests remain, and storage-specific SMTP tests remain
   unchanged except for imports.
7. No configuration key, CLI contract, database representation, SMTP default,
   TLS choice, credential behavior, public error message, or web interface
   changes.
8. The focused affected tests and `cargo xtask check` pass.
