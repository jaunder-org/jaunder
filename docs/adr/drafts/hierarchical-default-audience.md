# ADR-DRAFT: Hierarchical Default Audience

- Status: proposed
- Date: 2026-09-23
- Issue: [#1625](https://github.com/jaunder-org/jaunder/issues/1625)

## Context

Jaunder creates Posts through both the web composer and AtomPub. When a create
request supplies no audience, both paths currently read the instance-wide
`posts.default_audience` setting, whose absent or malformed value fails closed
to Private. The setting has no web control, and authors cannot choose a default
that differs from the operator's instance policy.

A site fallback and an author preference answer different questions. The
operator needs a safe baseline for the instance; an author needs a convenient
choice for their own future Posts. Conflating them forces either every author to
share one preference or each creation surface to invent its own precedence.
Neither default may rewrite existing Posts: an existing Post's Audience
Selection is durable publication state and changes only through an explicit Post
mutation.

ADR-0020 defines Public, Subscribers, Private, and per-author Named Audiences.
Named Audiences have an independent lifecycle and stable IDs, but may be renamed
or deleted. Persisting one as a default would make the meaning or validity of a
future Post creation depend on separate mutable state. The safe behavior after a
Named Audience disappears would then be ambiguous: fail creation, silently fall
back, or silently change visibility.

## Decision

Jaunder distinguishes three concepts:

- **Site Default Audience** is the operator-controlled instance fallback.
- **User Default Audience** is an optional preference owned by one User.
- **Effective Default Audience** is the resolved value for a new Post without an
  explicit Audience Selection: the User Default Audience when present, otherwise
  the Site Default Audience.

Both stored defaults use the closed `DefaultAudience` range: Public,
Subscribers, or Private. Named Audiences are deliberately excluded. An absent
User Default Audience means “use site default”; it does not copy the current
site value. A malformed User Default Audience rejects resolution rather than
inheriting a potentially broader site value. An absent or malformed Site Default
Audience continues to resolve to Private, while storage failures propagate.

Every Post-creation boundary uses the same Effective Default Audience rule. An
explicit Audience Selection remains authoritative. Web and AtomPub creation do
not maintain separate precedence or fallback behavior.

Changing either default affects only later creations that omit an explicit
audience. It never mutates an existing Post. Existing Posts change only through
an explicit ordinary Post mutation, including the same authorization, revision,
transaction, and publication-side-effect rules as any other Audience Selection
change.

The operator owns the Site Default Audience through Site Configuration. Each
User owns only their User Default Audience through Profile; operator status does
not grant a separate mechanism for rewriting another User's preference.

This decision refines the default used by
[ADR-0020](../0020-content-visibility-and-subscription-model.md) without
changing its Audience union or visibility-resolution rules. AtomPub audience
omission in
[AtomPub Post Audience Round-Trip](../0207-atompub-post-audience-round-trip.md)
resolves through the Effective Default Audience.

## Consequences

Authors can establish a durable default without weakening the operator's safe
instance baseline, and every creation protocol resolves omission identically.
The optional override follows the site setting until the author deliberately
chooses a value.

Existing unconfigured instances remain fail-closed to Private. Operators who
want Public or Subscribers must choose it explicitly; an upgrade cannot silently
widen future publication. Corrupt user configuration is visible as an error
instead of silently discarding the User's intended override.

Named Audiences cannot serve as defaults. This gives up a convenient shortcut
for authors who publish primarily to one group, but avoids dangling preferences
and visibility-changing fallback policy coupled to Named Audience deletion.
Authors may still select Named Audiences explicitly per Post and may apply them
to existing Posts through ordinary or bulk audience mutation.

The system must persist an optional per-User closed value, expose separate
operator and User controls, and test precedence and authorization across both
storage backends and every creation boundary. Documentation and UI must qualify
“default audience” as Site, User, or Effective where the distinction matters.
