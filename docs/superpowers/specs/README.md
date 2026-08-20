# Superpowers specs

This directory normally holds in-flight issue-cycle specs used by
`jaunder-develop` to derive cycle state from `issue-<N>` filenames.

When an issue ships, its spec belongs in `docs/archive/`. Archive the spec even
when the cycle was an umbrella issue with no matching plan; otherwise future
state derivation sees a closed issue as active work.

Explicit live design drafts retained here for unshipped work:

- `2026-06-16-emacs-blogging-frontend-design.md` — Emacs blogging frontend epic
  design.
- `2026-06-19-content-visibility-layer-c-design.md` — Content Visibility Layer C
  design.

Do not archive those two drafts until their work lands or a later issue chooses
a new home.
