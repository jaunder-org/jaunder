# ADR-0119: lettre is patched to a jaunder-org fork, pinned by rev

- Status: accepted
- Date: 2026-08-11

## Context

lettre's RFC 2822 mailbox grammar omits `domain-literal` and decodes a quoted
`local-part`, so `Mailbox`'s `Display` does not round-trip through its
`FromStr`. `Headers` stores each header as a string and re-parses it on `get`,
so `MessageBuilder::build` fails with `MissingFrom`/`MissingTo` for an address
lettre's own `Address` accepts — every RFC-legal quoted local part or
address-literal is stored but unsendable (#297).

## Decision

`[patch.crates-io]` points lettre at the jaunder-org fork
(`fix/mailbox-quoted-local-part-and-domain-literal`) until the fix lands
upstream; drop the patch when it does.

The patch is **pinned by rev, not branch**: it is a build input, so it should
change only when someone changes it here. A branch would re-resolve silently as
the PR is revised. To move it, push the fork, update the rev, and re-resolve
with `cargo update -p lettre` — the lockfile pins the commit either way, so
editing the manifest line alone does nothing.

`deny.toml` allows `jaunder-org` as a git source for the same reason —
`unknown-git` is only "warn", so the entry is not what makes the gate pass; it
is what makes the exception deliberate, and its removal the signal that the
patch is gone. Drop both together once the fix is released upstream.

## Consequences

- Two files (`Cargo.toml` patch, `deny.toml` allow) must be removed as a pair
  when upstream releases the fix.
- Fork revisions are moved deliberately, never by silent re-resolution.
