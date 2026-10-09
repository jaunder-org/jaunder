# ADR-DRAFT: Catalog-wide code token quality and repeatable projection refresh

- Status: proposed
- Date: 2026-10-04
- Issue: [#1678](https://github.com/jaunder-org/jaunder/issues/1678)

## Context

The
[host highlighting decision](../drafts/host-code-block-highlighting-and-projection-refresh.md)
introduced a pinned 42-grammar catalog, ten closed semantic hooks and a
versioned presentation-only refresh. At the original report, a Haskell Post had
an unterminated opening pragma that its grammar parses as one whole-block
`pragma` without an error; its query colored every line as a keyword. The author
has since corrected that Post, while a historical fixture preserves the
malformed-input regression. The upstream Emacs Lisp query colors definitions but
not ordinary call heads. Other reviewed capture names (`diff.plus`/`diff.minus`,
headings/quotes and calls/definitions) collapse into indistinguishable classes.
Earlier offline rebuilds and the version-1 bounded refresh can already be
complete on deployed installations. Changing only the renderer would leave old
stored Post projections stale. The existing offline rebuild already visits
**every** current Post; restarting the separate bounded pass for an ordinary
re-render would duplicate the work without adding coverage.

The approved
[quality spec](https://github.com/jaunder-org/jaunder/blob/150db0d178cd9a80ff5af3cf594a68ea8927686f/docs/archive/2026-10-03-issue-1678-code-highlighting-quality-spec.md)
requires source fidelity, all existing languages and aliases, Theme Package
compatibility, closed sanitization, and both database backends. This decision
refines the earlier draft's token vocabulary and malformed-input behavior; it
does **not** replace its `tree-sitter-highlight` engine or its security
boundary.

## Decision

Retain the pinned, host-only `tree-sitter-highlight` path and the full catalog.
Require a checked-in representative corpus with named source ranges and expected
semantic roles for every grammar; run every canonical label and alias through
Org and Markdown, checking decoded text against the same format's unhighlighted
exporter. The renderer may make a conservative plain-code fallback for an
unterminated leading Haskell pragma: the grammar otherwise accepts the entire
remainder as one directive, so painting it all as a keyword is misleading. Never
alter the authored source. Add call-head captures to Emacs Lisp and suppress
them for structurally quoted data, including nested and explicit quotes, while
allowing unquoted expressions inside quasiquotes. Limit renderer output to
actual source lines when a zero-width EOF capture invents a trailing line, and
restore authored CRLF line boundaries; retain the existing per-block/per-Post
budgets and typed unexpected-error boundary.

Extend the earlier ten closed `j-syn-*` span hooks with exactly five reviewed
roles: `function-call`, `diff-plus`, `diff-minus`, `heading`, and `quote`. A
call is different from a function definition; an added diff line is different
from a removed line; a code-heading capture is different from quoted code
markup. Map only audited capture names into these hooks. Keep the sanitizer
allowlist closed, strip inline styles and executable attributes, and scope all
fifteen color selectors to Post-body `pre code`. An author-forged permitted hook
can survive sanitization but cannot color content outside that scope. Provide
built-in defaults that mix each theme's semantic palette with its ink for
legible code text, and allow Theme Packages to override each additive hook;
existing Style Contract v1 packages remain valid, and authenticated Home retains
the built-in stylesheet. This refines the earlier draft's **ten**-hook
vocabulary without opening arbitrary classes.

Migration `0049` **only enqueues** the existing offline `rebuild_rendered_posts`
operation anew. SQLx applies that request once even when older queue rows were
already drained; the shared dispatcher needs no new operation case. Drain the
request under the existing storage lock before traffic. Its transaction
recomputes changed current derivatives for active, retained Deleted and
HTML-format Posts, reconciles Media references and affected public Syndication
Feed outbox events, and deletes the queue row together. A byte-equal projection
enqueues no duplicate event; a failed transaction leaves the request to retry.
The worker asynchronously regenerates feed bytes and validators from committed
events. Leave the established version-1 bounded refresh checkpoint and pass
implementation untouched: an unfinished original pass may complete through its
existing startup path, but this ordinary re-render does not restart it. Source,
timestamps, AtomPub Member ETags and Post Revisions remain unchanged.

## Consequences

The quote-aware Emacs Lisp pass parses a bounded block again to inspect quote
ancestry; malformed Haskell with an unterminated leading pragma displays intact
plain code instead of false certainty. The corpus is a gate for adding or
changing grammars. Adding any further visual role needs a reviewed sanitizer and
scoped-style change, not a raw query name. Installation upgrades perform one
newly enqueued offline rebuild before traffic, not a second bounded pass; a
failed queue transaction remains retryable. Concurrent old-version writers must
be stopped before the migration, as with the previous refresh decision.
