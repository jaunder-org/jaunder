# Issue #1750: User theme and Site theme UX revision

- Status: Agreed visible UX direction; cold Spec review found no findings and
  Lead accepted this working contract. This document grants no UI
  implementation, baseline capture or merge permission; Manager Site BEFORE
  admission is pending.
- Starting PR 1788 HEAD `13b94dc9362675f937a74eceb3be26c00841a2e3`, tree
  `a73b21cd45b574fce13ae8f204265eb7d6980f9f`; prior iteration contracts retired.
- Approval: direct human answer **“agreed”** to the exact visible proposal
  below; Lead acknowledges that bounded direction, not hidden document bytes or
  hashes.

> The proposed revision is: User theme: how visitors see your public pages. Site
> theme: the operator-only default for public site pages and users using that
> theme. Each gets its own clear view, sharing components underneath. The main
> flow becomes add or choose → review your choice → use this theme, with the
> current theme shown nearby and editing kept in Advanced. No Catalog scope, and
> no merely renamed combined form.

Historical backend approval remains proposal SHA-256
`7ea0ca96897eb47d46c82ec0059072592237dde122515dbdb70329792e6fb84c`;
compatibility precision
`a5787a2d21923af248b4fa1d9b46cca0c3f52e9596e230225ab458e997d149f5` is
Manager-qualified nonmaterial, not a separately human-approved hash. Prior
outline approval
`ae201d07737b6dc5637bf80d112e398a6eb93704b85b2e13403aa9170c93fe66` does not
approve this revision's implementation detail or outline.

## Outcome

Make public-theme management understandable through distinct **User theme** and
**Site theme** views, a visible current setting and a plain-language journey.

## Load-bearing decisions

- Keep authenticated `/themes`: ordinary users see User theme only; operators
  explicitly navigate User theme / Site theme. No generic ownership card or
  merely renamed combined form. Share cohesive behavior rather than copy
  editors.
- User copy: “Choose how visitors see your public pages. This does not change
  your private workspace.” Site copy: “Choose the default look for public site
  pages and user pages using the site theme.” Home/private Studio stay
  protected.
- Show the known current name, or “Using the site theme” for confirmed author
  inheritance. Failed reads show “Current theme could not be checked,” never a
  fabricated name/default. Do not invent dirty/revision metadata or new reads.
- Order Add or choose a theme → Review your choice → Use this theme, without a
  forced wizard. Offer Upload theme / Available themes. Optional Advanced field
  “Theme name (optional)” explains blank uses the theme's own name.
- Upload/choice never activates. “Saved, not live yet” applies only to a known
  unused uploaded choice; “Currently in use” requires confirmed current
  identity. Do not imply edits to an already-used Theme ID are necessarily
  nonlive.
- Keep editing/source/JSON/CSS, preview/export/rename/removal and standalone
  publication optional in Advanced. Primary copy avoids catalog/scope and
  draft/published/inherited/publication/selection exposition.
- Preserve validated manifest-default naming, invalid explicit-name rejection,
  bounds/quotas, authorization, private/unpublished/unselected import and wire
  compatibility. Preserve owner resets/input isolation and pending navigation,
  duplicate-operation and captured Theme ID/owner guards.
- Only Confirmed publication chains to existing selection. Rejection prevents
  chaining; database/transport/decode uncertainty stops it and rereads both
  states, without retries or false atomicity/rollback. Already-selected
  publication can advance public revision despite a later failed switch. Failed
  rereads stay unknown.
- Primary progress: “Getting your theme ready…” then “Applying your theme…”.
  Confirmed completion: “Your theme is now in use.” Uncertain preparation says
  readiness was unconfirmed and switching was not attempted; an already-used
  theme may have changed. Confirmed preparation with failed switching says the
  updated theme is ready but switching failed; it may already be live if used.
  Report actual refreshed setting/unknown. Keep accessible stage-correct
  styling.
- Bundled themes and Use site theme are explicit selection-only recovery paths;
  no publication. Preserve ADR-0184/0197 package/security/lifecycle boundaries.

## Acceptance

- Both roles see literal, distinct views/effects/current summaries; Site
  controls remain operator-only. Basic controls/copy follow the ordered journey,
  not jargon.
- Prove known/unknown summaries, private import/name defaults, nonactivating
  choice, confirmed custom use, bundled/inheritance recovery, owner isolation
  and pending guards. Preserve
  rejection/uncertainty/reread/already-used-revision proof.
- At 1440×900 and 390×844, light/dsf1/reduced motion, verify keyboard
  disclosure, focus/live feedback and ordinary-wheel access to the expanded
  final control.
- Reuse qualified User BEFORE at actual `1d8ac681…`/`2bcf1a91…`, presentation
  unchanged through `13b94…`. Before mutation, obtain accepted Site BEFORE at
  both viewports: initial, uploaded choice, expanded bottom (six images), under
  separate Manager resource admission and an accepted clean planning checkpoint.
- Deliver matched final User/Site pairs in one browser-openable HTML gallery;
  retain exact source/method/fixture labels. Preserve original 13+9 images/HTML
  and raw `84d7…` ZIP. Old CI/coverage/images do not certify future
  presentation.

## Boundaries

No new route/API/parser/schema/storage/auth/role/protocol/framework, discovery,
Media behavior or atomic operation. This working contract authorizes planning
only; Lead owns review/classification/checkpoint/baseline and execution
admission.
