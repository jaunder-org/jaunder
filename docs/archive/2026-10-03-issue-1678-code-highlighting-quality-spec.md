# Issue #1678 — useful syntax highlighting throughout the supported catalog

## Outcome

Published, explicitly labeled Org and Markdown code blocks have accurate enough
syntax distinctions to be readable and good-looking across Jaunder's supported
language catalog, not merely the two reported examples. New and existing Posts
share the improved presentation; unknown languages remain safely escaped,
uncolored code. Perfect semantic classification of every possible program is not
promised.

## Load-bearing decisions

- Keep the supported, pinned broad language catalog and aliases (currently 42
  grammar variants) as the coverage boundary. Do not narrow the catalog to make
  the examples pass. Emacs Lisp and Haskell production samples are regressions,
  not the complete quality definition. Only eligible Org source blocks and
  explicitly labeled Markdown fences are highlighted; other code and raw
  authored HTML retain the existing safe behavior.
- Require a checked-in corpus manifest with representative, language-appropriate
  source and **named expected token roles/ranges** for each supported grammar,
  chosen from distinctions its pinned grammar can reliably identify. Each
  canonical label runs through both Org and Markdown; every declared alias is
  checked for lookup and correct rendering in both formats. The proof inspects
  spans and decoded source text; the mere presence of a `j-syn-*` span, or a
  query that paints whole lines as one category, is not a quality pass. Wrong or
  misleading captures must be repaired rather than hidden with CSS. Retain
  malformed-input safety and bounded fallback.
- Choose the highlighter engine, query source, and HTML renderer based on
  comparative evidence of catalog breadth, capture quality, maintainability and
  integrated output. The existing `tree-sitter-highlight` decision is the
  starting point, not a justification for keeping poor output. Tree-sitter's
  `highlight` CLI, especially `--html --layout fragment --style minimal` and
  query/capture inspection, is a diagnostic/reference tool; it is not
  automatically a production renderer or a substitute for testing Jaunder's
  sanitized HTML. If evidence warrants replacing the #1655 engine choice, record
  the changed decision explicitly in a new ADR instead of silently contradicting
  its draft.
- Preserve reliably distinguishable syntax roles in the markup. Where the
  current ten closed semantic token classes erase useful distinctions, add a
  small reviewed set of additive semantic hooks with styling defaults and custom
  Theme Package overrides. The sanitizer may retain an author-forged
  **permitted** hook because it cannot inspect ancestry; that hook must be
  visually inert outside Post-body `pre code`. Strip unsupported hooks, inline
  styles and executable markup; never admit arbitrary query names. Home retains
  built-in styling. Existing Style Contract v1 themes must continue to work
  without modification.
- Preserve the decoded code text exactly as the same format's unhighlighted
  exporter emits it, and preserve authored Post source, AtomPub Member content
  and Post Revisions. Keep the existing per-block/per-Post parsing limits and
  typed failure boundary; an unsupported or over-budget block remains escaped
  plain code.
- Refresh already-stored **active** Org and Markdown Post projections through
  the established bounded, resumable presentation-only transition before serving
  the new version; its bounded pass skips Deleted and HTML-format Posts. Use a
  new durable enqueue/version transition so installations that already drained
  the #1655 and later renderer migrations run the changed renderer once, then
  checkpoint without duplicate feed events if the offline rebuild already
  updated a row. As in the existing offline rebuild contract, that separate
  operation may update changed current derivatives of retained Deleted/HTML
  Posts; it does not rewrite their source or revisions. Preserve Post identity,
  authoring format, timestamps, AtomPub Member content ETag and immutable
  revisions; maintain derived Media references, public Syndication Feeds and
  validators when rendered bytes change. Old-version writers must be fenced.
  Document any new security, Style Contract, engine or transition decision in a
  numberless ADR draft and project it into `docs/ARCHITECTURE.md`; consider
  `CONTEXT.md` for vocabulary changes.

## Acceptance

- The corpus manifest covers **every supported grammar**, names exact expected
  roles for stable token ranges in each sample, exercises each canonical label
  and alias in both exporters, and checks decoded-text fidelity. A fixture
  derived from the
  [Haskell production Post](https://tendentious.org/~mdorman/2014/06/01/replacing-nothing-values-with-just-values-in-a-nested-structure)
  must not wrap nearly every line in `j-syn-keyword`; the
  [Emacs Lisp production Post](https://tendentious.org/~mdorman/2014/12/28/how-i-would-start-out-with-emacs-now)
  must distinguish appropriate unquoted call heads from punctuation, strings,
  comments and quoted forms. Preserve fixture provenance and actual exporter
  text, including malformed source, rather than silently correcting it to make
  the test pass. Failure messages identify grammar and misplaced or missing
  roles; smoke tests alone do not certify quality.
- Comparative evidence exercises the current renderer and any proposed
  replacement on representative corpus samples, records capture quality and
  unsupported-language behavior, and explains the chosen path. If query capture
  names change, the audited mapping remains bounded; tests strip unsupported
  forged classes and inline styles, and show that permitted forged classes
  outside Post-body code cannot color content. Existing HTML safety,
  resource-limit, error-propagation and source-fidelity tests stay green.
- Tests on SQLite and PostgreSQL prove new and existing active Post projections,
  a fresh durable enqueue on databases whose earlier refreshes completed,
  restart/resume and idempotent refresh, bounded-pass exclusion of Deleted/HTML
  Posts, preservation of source/revisions/timestamps/Member ETags, and correct
  public Syndication Feed/validator updates for changed markup. A changed
  projection must not be an unrecorded author edit.
- Before presentation changes, capture baseline images. Afterward, provide
  comparable **transient manual-review** before/after screenshots for the two
  production permalinks above and representative Local and authenticated Home
  states at desktop and narrow mobile widths, including built-in and applicable
  custom Theme Package styling. These are not Playwright `@visual` snapshots and
  do not add mobile/theme variants to the repository's fixed snapshot
  population. Check visibly distinct, legible roles without whole-line
  over-coloring or layout regressions. Browser assertions verify markup and
  readability on permalink, Local and Home; use the project e2e and
  accessibility conventions.
- Existing supported languages remain available, and an unknown label remains
  safely legible without token colors. A newly added catalog language must meet
  the same corpus and presentation proof before admission.

## Boundaries

- This is a quality repair for host-rendered Post code, not an editor, runtime
  code execution, browser-side grammar loader, change to the Post source/AtomPub
  contract, new arbitrary language plugin system, or a promise of perfect
  semantic analysis for all future snippets.
- Do not change the meaning of unlabeled, unknown, HTML-source, inline or
  indented code blocks. Do not weaken sanitization, theme isolation, parse
  budgets or backend parity to improve appearance. No engine swap or palette
  expansion is presumed until comparative evidence justifies it.
