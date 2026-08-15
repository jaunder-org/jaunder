# ADR-0143: Emacs auth-source App Password Storage

- Status: accepted
- Date: 2026-08-14
- Issue: [#938](https://github.com/jaunder-org/jaunder/issues/938)
- Supersedes: [#76](https://github.com/jaunder-org/jaunder/issues/76)

## Context

The Emacs Protocol Client publishes AtomPub entries from Emacs buffers. The
server authenticates these writes with an App Password.
[ADR-0014](0014-atompub-authentication.md) defines App Passwords as bearer
credentials stored server-side as keyed hashes; the client-side storage boundary
remained implicit.

The current Emacs code retrieves credentials through Emacs `auth-source`: it
searches by the base URL host and configured username, uses at most one match,
and constructs the Basic auth header at request time. Publishing code currently
retries the broad signalled-error path; #1062 tracks narrowing that so a missing
credential is not retried as transport work.

Issue #76 proposed a different flow: log in once, call `create_app_password`,
store the returned token, and discard the login session. That would make the
Emacs client responsible for minting and persisting App Passwords. The approved
boundary is narrower and was recorded when #76 was closed as superseded by #938.

## Decision

The Emacs Protocol Client delegates App Password storage to Emacs `auth-source`.
It may read the configured user's secret from `auth-source` at request time and
use it to build the AtomPub Basic auth header. It must not mint, prompt for,
write, rotate, or otherwise persist an App Password itself.

Credential lookup identity is the active blog's URL host and configured
username. The URL port is intentionally excluded, so a single instance-level App
Password can serve multiple endpoint ports for the same host/user pair. The
client does not normalize username values at this boundary; server-side username
rules remain a server concern.

A missing `auth-source` entry is a configuration error. It is not a transient
transport failure and must not be retried by the publish retry loop. Current
`jaunder--create-with-retry` still retries the broad signalled-error path before
surfacing that failure;
[#1062](https://github.com/jaunder-org/jaunder/issues/1062) tracks narrowing
that behavior. The client may surface the error to the user, but recovery is to
add or fix the `auth-source` entry outside Jaunder.

This decision preserves the neighboring Emacs boundaries. ADR-0035 may provision
a temporary `auth-source` entry only inside its live-test harness; it is not a
client persistence path. ADR-0038 remains the HTTP transport decision, and
ADR-0047 remains the multi-blog/configuration-threading decision. A future
interactive or non-interactive flow for creating credentials would need a new
ADR because it would change which component is allowed to mint or persist App
Passwords.

## Consequences

Emacs stays inside the existing Emacs secret-storage ecosystem and avoids
inventing a second password store. The cost is that first-time setup requires an
operator/user step outside the package.

One App Password can cover the same host/user across endpoint ports. That fits
local reverse-proxy and development deployments, but a future design that needs
port-scoped credentials must reopen this boundary explicitly.

The Protocol Client remains simpler: no login session handling, no App Password
creation endpoint dependency, and no local persistence or rotation logic.

Rejected alternatives:

- Implementing #76's self-provisioning flow. It crosses the client persistence
  boundary by making Emacs mint and store App Passwords.
- Prompting and saving credentials from the client as a fallback. That still
  makes Jaunder an auth-source writer and changes secret ownership.
- Including the URL port in the lookup identity. That would force separate
  secrets for one service reached through different local/proxy ports without an
  approved need.
- Retrying missing credentials as transport failure. Absence from `auth-source`
  is deterministic misconfiguration, not a transient network/server condition.
