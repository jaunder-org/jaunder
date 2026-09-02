# ADR drafts

New ADRs are authored and committed here without a number. Their slug-bearing
path is the draft's identity while the decision remains proposed; promotion
assigns the shared number and accepts the decision. The local mechanics live in
the `jaunder-adr` skill and `CONTRIBUTING.md`.

## Why drafts are numberless

An ADR number is a shared monotonic sequence. Assigning it on a feature branch
means concurrent decisions can claim the same filename and generated-index row.
A tracked draft instead carries **no number**, so feature branches can merge
their distinct slug paths without ADR bookkeeping conflicts.

Markdown files in this directory are tracked. This `README.md` remains the
explainer rather than a draft, and `promote` intentionally skips it.

## Authoring a draft

1. Copy [`../template.md`](../template.md) to `docs/adr/drafts/<slug>.md`.
2. Keep the draft heading exactly `# ADR-DRAFT: <Title>` — `promote` swaps the
   `DRAFT` token for the assigned number. Leave `- Status: proposed` alone: this
   pen **is** the proposed state, and `promote` rewrites the token to `accepted`
   when it numbers the file, because numbering is the acceptance event. Set the
   status by hand only to say something else — a draft marked `superseded` or
   `rejected` is a deliberate claim and survives promotion untouched.
3. Reference the draft **by path** (`docs/adr/drafts/<slug>.md`) from any code
   or prose that needs it. There is no bare `ADR-DRAFT` token — use the path so
   `promote` can rewrite it to the real number.
4. Link a numbered sibling ADR relative to the draft's current location —
   `[ADR-0061](../0061-web-keyed-list-reactive-store.md)`. This target resolves
   while the draft is tracked here; when promotion moves the file up one
   directory, it strips the leading `../`, so the same link resolves from
   `docs/adr/` afterward.
5. Link **another draft** as `[Aaa](../drafts/aaa.md)`. Promotion strips one
   level to `drafts/aaa.md`, which `promote` then rewrites to the number it
   assigned. Do **not** use the rule-3 repo-root form (`docs/adr/drafts/aaa.md`)
   in a markdown link from one draft to another: it becomes
   `docs/adr/NNNN-aaa.md`, which is dead from inside `docs/adr/` and will fail
   the `doc-links` gate. Rule 3 still applies to references from code and prose
   _outside_ `docs/adr/`.

## Automated promotion

Feature authors commit drafts and their path citations; they do **not** run
`cargo xtask adr promote` while shipping. After the feature reaches `main`, the
serialized ADR promoter invokes that deterministic local mutation against fresh
`main` and opens its merge-queue PR.

For each tracked draft, sorted by slug, promotion assigns the next free number,
stages the source deletion and numbered destination, rewrites the heading and
the `proposed` status to `accepted`, rewrites path-form citations, and syncs and
stages the generated ADR index. A successful rerun with no drafts is a clean
no-op. The draft remains proposed during the normal promoter CI/queue interval;
if the promoter fails, it remains proposed until that visible automation failure
is repaired.

## Gate invisibility

The `identifier-collisions`, `adr-format`, and `adr-readme-parity` gates share
one enumeration rule — `is_file` → `.md` → leading number, applied by a
non-recursive `read_dir` over `docs/adr/`. A numberless draft in this
subdirectory is excluded twice over, so drafts never trip a gate.

`doc-links` enumerates tracked Markdown, so it checks drafts before promotion.
Use the numbered-ADR and cross-draft link forms above: they resolve in the
tracked draft, then `promote` adjusts them for the one-directory move and path
rewrite so they resolve in the numbered ADR too.
