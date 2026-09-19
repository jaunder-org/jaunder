# Local Post Links Implementation Outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` for an individual task
> when useful. This outline exists because the feature changes the durable
> publish/pull representation boundary of the Emacs Protocol Client.

## Scope

In:

- Org Local Post Link classification, exact target validation, and media
  precedence.
- Server-authoritative Member alternate-URL harvesting and identity joins.
- Sent-body-only publish substitution before any server mutation.
- Syntax-aware pull reversal for exact, uniquely matched local targets.

Out:

- Markdown or HTML local Post links.
- Server permalink routes or new AtomPub fields.
- Absolute, attachment, cross-root, search-target, query, or fragment support.
- Target search, automatic pull, repair, or rename behavior.

## Task outline

- [x] **Task 1: Carry authoritative public-link identity through Member
      inventory**
  - Contract: the shared remote Member record carries Post ID, slug, edit URI,
    and a per-Member alternate outcome: either one validated exact `href` or a
    typed invalid reason. The local record carries path, Post ID, and slug
    evidence needed to prove `<slug>.org`. Alternate selection accepts exactly
    one direct Atom `rel="alternate"` link, validates the approved active-origin
    URL shape, and retains its exact string; one invalid Member does not fail or
    disappear from the complete inventory.
  - Verification: focused Atom-harvest and reconciliation ERT cases cover zero,
    one, malformed, cross-origin, and duplicate alternates—including identical
    duplicates—plus stable invalid reasons and stale, duplicate, or mismatched
    local ID/slug/filename evidence.

- [x] **Task 2: Localize Local Post Links in the publish preflight**
  - Contract: a body-only Org-link pass claims every relative `.org` file-link
    candidate before media, resolves the exact same-root regular target without
    searching, joins its local identity to the Member inventory, and returns a
    sent-body copy containing exact harvested `href` values. A referenced
    target's candidate, identity, or alternate failure emits its visible
    diagnostic and aborts before media upload or Post mutation; invalid evidence
    on an unrelated Member does not block the publish. Ordinary successful
    write-back and rename remain owned by publish orchestration.
  - Verification: focused Org/media/publish ERT cases prove both supported link
    spellings, repeated and distinct targets, description and body-byte
    preservation, unsupported suffixes, path escapes, drafts/orphans, no media
    fallback, unrelated invalid Members, no mutation on failure, unchanged
    external/media behavior, successful metadata write-back and rename, and
    ADR-0047 retry ordering after pre-response and committed-response failures.

- [x] **Task 3: Reverse exact canonical Post links during both pull paths**
  - Contract: one syntax-aware Org body-link pass receives the remote and local
    inventory evidence, compares HTTP(S) destination strings byte-for-byte with
    harvested alternates, skips every invalid alternate outcome, and rewrites
    only qualifying destination spans to `./<slug>.org`. Invalid or incomplete
    evidence preserves the canonical URL. The pass runs in the shared staged
    representation used by server-only and matched server-ahead pull without
    weakening either install path's safety checks.
  - Verification: focused pull/reconcile ERT cases cover exact matches, partial
    inventories, normalization-equivalent mismatches, mismatched ID/slug/path,
    descriptions, surrounding bytes, plain text, code, metadata, media links,
    server-only install, and revalidated matched replacement.

## Risk checks

- Keep URL authority with the Member response; no route, slug, username, or ID
  based permalink construction.
- Keep invalid relative `.org` candidates out of media upload while leaving
  absolute and `attachment:` behavior outside this feature.
- Preserve ADR-0024's authored-versus-sent representation split and ADR-0047's
  mutation/write-back ordering.
- Preserve ADR-0160's syntax-aware staged pull and ADR-0200's matched-Post
  snapshot revalidation, modified-buffer, destination, and recovery guarantees.
- Keep diagnostics free of credentials and Post body content.
- Before each task commit, tick its checkbox and use `jaunder-commit`; the
  enforced pre-commit gate owns broad formatting, ERT, byte-compilation, and
  documentation checks.
