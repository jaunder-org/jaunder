# D2 deterministic AtomPub pull — implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for bounded slices.
> Trigger: public Atom representation plus race-safe no-clobber filesystem
> installation and a D2→D3 interface.

## Scope

In:

- Empty-versus-real Atom title projection in Rust and Member GET.
- Deterministic Member Entry → Org bytes, including reversible multiline titles
  and canonical XHTML.
- One D3-facing server-only pull operation with atomic no-replace installation.
- Pure Rust/ERT, shared-backend AtomPub integration, and live ERT proof.

Out:

- D1 inventory changes; D3 report/confirmation; D4 deletion; matched-Post pull;
  media localization; overwrite/suffix behavior; new ADR.

## Task outline

- [x] **Task 1 — Correct the public Atom title projection.**
  - Files: `server/src/atompub/mapping.rs` and shared-backend AtomPub Post
    tests.
  - Contract: `post_to_entry` always emits the required title element;
    `None`→empty text, `Some(title)`→exact text with no slug fallback.
  - Verification: mapper unit tests cover empty, real, and title-equals-slug;
    `#[apply(backends)]` Member GET tests assert serialized response bodies on
    SQLite and PostgreSQL.

- [x] **Task 2 — Prepare compatible Atom and Org parsing seams.**
  - Files: `elisp/jaunder-atom.el`, `elisp/jaunder-org.el`, and focused pure
    ERT.
  - Contract: `jaunder--harvest-response-fields` becomes a compatible superset:
    old keys/meaning remain; new direct-child values and cardinalities preserve
    wire order but do not require Post Member fields. Media/content-only Entries
    still harvest successfully. `jaunder--org->atom` joins repeated title values
    with LF while preserving single-title behavior.
  - Verification: existing media/publish harvest cases stay exact; new ERT pins
    direct-child versus nested metadata, duplicate cardinality representation,
    and single/repeated/empty title mapping.

- [ ] **Task 3 — Implement and load pure Member Entry → Org synthesis.**
  - Files: new `elisp/jaunder-pull.el`, its `jaunder.el` require, pure ERT, and
    removal of the `jaunder--atom->org` stub from publish.
  - Contract:
    `(jaunder--atom->org ENTRY-XML ETAG CAPTURED-AT ZONE) -> ORG-BYTES`. It has
    no I/O and owns Member-only validation over Task 2's harvested shape.
  - Title contract: empty text means no title lines; LF-delimited text maps to
    repeated `#+TITLE`, reversible through Task 2's forward parser.
  - XHTML contract: require the Atom XHTML `div`; canonically serialize its
    children in order, excluding the wrapper and escaping text nodes.
  - Verification: fixed clock/zone exact-byte ERT covers every header field,
    draft/scheduled/published, formats/XHTML, multiline title/summary
    round-trip, categories, body/media URL, malformed cardinality, and strong
    ETag.

- [ ] **Task 4 — Add the D3-facing no-clobber pull operation.**
  - Files: extend loaded `elisp/jaunder-pull.el` and its pure ERT.
  - Contract: `(jaunder--pull-member ROOT MEMBER) -> jaunder-pull-result`, with
    `status` exactly `pulled` or `blocked` and an exact direct-child `path`.
    `MEMBER` is D1's shipped `jaunder-inventory-member`.
  - Flow: preflight D1 slug path; GET `member.edit-uri` inside
    `jaunder--with-blog`; require response ID/slug identity; capture clock/zone
    once; synthesize bytes; write a same-directory temp; atomically claim the
    destination with `add-name-to-file` and no overwrite; always remove temp.
    Preflight/race collision returns blocked; other failures signal.
  - Verification: ERT pins GET URL/context, result values/path, unsafe or stale
    identity, weak/missing ETag, non-2xx/transport/mapping errors, temp-write
    and install fault cleanup, race winner preservation, and no leaked temp
    files.

- [ ] **Task 5 — Prove the complete pull against the live server.**
  - Files: focused `*-integration.el` only, after Tasks 1–4.
  - Contract: use the shared live server without assuming an empty Collection;
    create a uniquely identified untitled Org Post, inventory its D1 Member, and
    pull only that Member.
  - Verification: exact `<slug>.org` bytes omit title, preserve native body and
    a server media URL, and carry ordered metadata/sync markers; a pre-existing
    destination stays byte-identical and returns blocked.

## Ordering and ownership

- Tasks 1 and 2 may run in parallel: Rust owns the wire producer; Emacs prepares
  compatible parsers. Task 3 depends on Task 2; Task 4 on Task 3 and D1; Task 5
  on all prior tasks.
- Only Task 2 changes the shared harvester and forward title parsing. Only Task
  3 defines and loads `jaunder--atom->org`; only Task 4 defines
  `jaunder-pull-result` and `jaunder--pull-member`.
- Callers migrate cleanly; no alias, compatibility wrapper, second parser, or
  alternate pull path remains.

## Risk checks

- Atom title remains required; untitled is empty text, not absent and not slug.
- Harvester uses direct-child cardinality so nested XHTML tags cannot masquerade
  as Entry metadata; old media/publish consumers remain behaviorally identical.
- The previewed D1 slug and fetched Member identity must agree before mutation.
- Destination claim is atomic no-replace; every failure leaves root bytes and
  any race winner untouched, with no temporary artifact.
- Clock/zone are captured once; status, dates, and sync time cannot disagree.
- Native content and server media URLs are not rendered, trimmed, or localized.
- Run pure ERT, focused/shared-backend AtomPub Rust tests,
  `devtool run -- cargo xtask elisp-integration`, and each task's
  `devtool run -- cargo xtask precommit` commit gate. No lint suppressions.
