# ADR-DRAFT: Unify bundled themes and application styling asset delivery

- Status: proposed
- Date: 2026-10-07
- Issue: [#1695](https://github.com/jaunder-org/jaunder/issues/1695)

## Context

Production Safari reused stale built-in CSS at stable `/style/` URLs. Clearing
browser caches resolved the observed desktop and iPhone image-sizing symptoms;
current CSS already expressed the intended sizing policy.

[ADR-0003](../0003-asset-management.md) placed built-in CSS in a separate
embedded file handler. [ADR-0184](../0184-css-package-public-themes.md) gives
published Theme Packages digest-addressed immutable assets, durable eligibility,
and reference/deadline-based retention. Separate built-in delivery permits these
contracts to diverge even though both style the same application.

Adding revalidation to stable stylesheet URLs does not invalidate responses
already fresh in browser caches. Hashing embedded files independently would
repair URL identity while retaining a second publication/retention lifecycle:
replacing the binary would remove earlier bytes still referenced by documents.

Application CSS also styles protected controls and private application surfaces.
Its authority differs from scoped owner-authored public CSS, but that
distinction does not require different content addressing, serving, or
retention.

## Decision

Studio, Terminal, and Reader are system-managed bundled Theme Packages. They use
the same package validation, compilation, immutable revision, and presentation
resolution pipeline as custom packages. Existing named selections remain valid
and follow the installed release's revisions. Site/author precedence and Studio
fallback remain unchanged. Bundled packages are selectable but not mutable or
removable through custom catalogs and do not consume custom-theme quotas.

System-authored application CSS remains a protected, non-selectable asset role.
It shares package content's persistent installation, digest identity, serving
eligibility, HTTP response behavior, and collector. Shared delivery never widens
custom package CSS scope or grants permission to affect trusted controls.

The binary carries its complete system artifacts. Before accepting requests,
startup idempotently installs and verifies them in the shared asset store and
advances system references atomically. Installation or verification failure
prevents startup; there is no separate embedded styling fallback. Superseded
system assets retain their exact bytes while referenced and through the existing
one-year asset lifetime plus five-minute HTML freshness window after detachment.
Backup/restore and rollback preserve retained bytes and eligibility. System
ownership remains separate from owner quotas without bypassing content identity
or retention correctness. Both database backends have the same contract.

Every document consumer uses generated digest references. Published application
and theme styling responses use `public, max-age=31536000, immutable`, exact
MIME, ETags, and conditional request behavior. Database-independent thumbnail
commands share artifact production and serving semantics through their existing
loopback-only command transport, not a separate `/style` service.

The stable `/style/jaunder.css` and `/style/jaunder-themes.css` routes are
retired without compatibility aliases and return 404, never a SPA document.
Legacy HTML may be unstyled until refreshed; removing the route cannot evict
previously cached responses. The existing five-minute public-document freshness
window remains. Older digest references continue to resolve under retention.

This amends ADR-0003's separate stylesheet-serving mechanism and ADR-0184's
built-in presentation implementation, preserving single-binary distribution,
custom package isolation, ownership, deterministic presentation, and retention.
CSR and Media retain their existing content-addressed contracts; consolidating
their backing stores is not part of this decision.

## Consequences

One lifecycle owns deployment invalidation for application and theme styling;
bundled defaults cannot silently receive weaker caching than custom packages.
Persistent system references add startup, upgrade, rollback, and backup/restore
responsibilities and require backend-parity evidence. A refresh obtains new
digest references rather than depending on browser heuristic freshness.

Public theme source is portable package input rather than a second built-in CSS
format. Protected application CSS is distinguished by authority, not transport.
The deliberate legacy-URL break avoids hiding old documents behind aliases;
there is no promise of immediate invalidation of already cached legacy HTML.
