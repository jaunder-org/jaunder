# #1678 — comparative syntax-highlighting evidence

## Method and scope

The checked-in `host/src/code_highlight/quality/catalog.rs` pins UTF-8 byte
starts, source text and typed expected roles for all **42** grammar variants.
The test renders each canonical label through both Org and Markdown, compares
decoded code to the same exporter's unknown-language/plain rendering, and checks
role coverage even when captures nest. A separate alias test renders every
declared label through both exporters and compares its token markup with the
canonical label. The two reported production excerpts are byte-for-byte fixtures
captured from their public `<code>` elements on 2026-10-04, including the
Haskell typo; malformed, quoted and unknown-language cases are separate
regressions.

`tree-sitter` CLI is not on the pinned devShell PATH in this checkout. Instead
of installing an unpinned CLI or changing the devShell for a one-off diagnostic,
the comparison test calls the pinned `tree-sitter-highlight` API directly with
the upstream grammar query and the separately bundled Syntastica Haskell query.
This exercises the same parser, query captures and event stream that the CLI
would inspect, **not** an independent highlighting engine or a second HTML
sanitizer. Its limits matter: shared grammar/query bugs cannot be resolved by
matching a CLI serialization. The integrated test additionally checks the actual
sanitized Post HTML; only that output is Jaunder's contract.

## Material comparisons

| Corpus case                                     | Reference/query evidence                                                                                                                                                                                                                     | Prior Jaunder output                                                                            | Reviewed integrated output                                                                                                                                          |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Production Haskell, opening `{-#` with no `#-}` | Both upstream and bundled queries capture **over 90% of all source bytes** as `keyword.directive`: the parser's entire tree is `(haskell (pragma))` without an error node. This is a grammar recovery behavior, not a missing palette color. | Every line was wrapped as `j-syn-keyword`, including imports and blank lines.                   | Intact escaped plain code for this unclosed leading pragma; a well-formed pragma still receives multiple roles. No invented edit to authored source.                |
| Production Emacs Lisp call heads                | The upstream query captures **zero bytes** as `function.call` on the production excerpt. A reviewed first-symbol-of-list query captures `add-to-list` and `package-list-packages`.                                                           | Punctuation, `progn`, and strings had roles, but ordinary call heads were plain.                | `j-syn-function-call` marks evaluated heads; a bounded quote-aware AST walk suppresses literal data but retains unquoted expressions, even with nested quasiquotes. |
| Diff addition/removal                           | Bundled captures name `diff.plus` and `diff.minus` distinctly.                                                                                                                                                                               | Both mapped to `j-syn-string`.                                                                  | Closed `j-syn-diff-plus`/`j-syn-diff-minus` roles use themeable positive/negative colors.                                                                           |
| Markdown code heading/quote                     | Bundled captures name `markup.heading` and `markup.quote` distinctly. An EOF zero-width capture can yield one phantom renderer line.                                                                                                         | Both mapped to `j-syn-string`; the phantom line changed decoded text on a trailing block quote. | Closed `j-syn-heading`/`j-syn-quote` roles, with renderer output limited to the exporter's actual source lines and authored CRLF boundaries restored.               |
| Unknown language and alias coverage             | No parser or capture is available for the unknown label; catalog/aliases all resolve to the same pinned grammar per label.                                                                                                                   | Escaped plain code for unknown, canonical markup for aliases.                                   | Same eligibility and plain fallback; source-equivalence and alias checks now cover every grammar and both formats.                                                  |

The other corpus samples initially exposed test-serialization mismatches rather
than missing captures: Dart, Nix, Ruby, Swift, XML, CSS and Diff legitimately
nest or split role spans. The oracle now checks decoded byte ranges under active
nested scopes rather than demanding one flat `<span>` per token. Containerfile
shell-form `RUN` and Make recipe shell text are raw/uninjected in their pinned
grammars; their JSON-form command and Make variable/target roles are tested
instead. Inline Markdown emphasis is not captured by the pinned block grammar;
heading and quote are its reviewed block-level distinctions. The pinned TSX
query marks the closing `h1` tag name but not the opening one; its corpus oracle
pins the closing byte range rather than allowing either occurrence to satisfy
the expectation. These are explicit remaining limits, not a claim of semantic
classification of every expression.

## Browser presentation evidence

A preserved local sandbox upgraded an already-populated database through `0049`
(progress version 2, completed; offline queue empty). Chromium captures compare
the linked production excerpts before this change with local recreations using
the same code and supported labels after it; the live production permalinks have
**not** been deployed with this change. The malformed Haskell excerpt is intact
plain code instead of a whole-block keyword cascade. The Emacs Lisp excerpt
retains existing roles and now distinguishes evaluated call heads.

To make the before/after proof reproducible rather than compare unrelated local
states, `issue-1678-matched` was seeded and captured under `origin/main`
(`df0dc8f72`), then **the same SQLite workspace, four Posts and published Theme
Package** were upgraded and captured under `cd5c28ef` (this branch). The Haskell
and Emacs Lisp code uses the byte-exact production fixtures and supported
labels; Diff and Markdown exercise the other repaired roles. The manifest and 24
captures per phase are transient under `/tmp/pi-playwright/issue-1678/matched/`:
`manifest.json`, `before-evidence.json`, `after-evidence.json`, and
`{before,after}-{studio,terminal,reader,custom}-{name}-{390,1280}.png`. Public
permalinks cover both excerpts at both widths in every built-in theme and the
custom package; Studio also covers Local and authenticated Home at both widths.
The custom **public** selection applies to the author's permalinks, not the
viewer's Local/Home surfaces. For a direct review, five transient side-by-side
images named `pair-studio-haskell-mobile.png`, `pair-studio-elisp-mobile.png`,
`pair-custom-diff-mobile.png`, `pair-home-desktop-code.png`, and
`pair-local-desktop-code.png` are in that folder. The latter two crop the same
Post-body code region from desktop feed captures because their scroll-container
positions differ; the unmodified full images remain alongside them. No image is
a repository `@visual` snapshot. Production-before screenshots remain a separate
real-site anchor, **not** a claim that a live production after-image exists
before deployment.

The focused browser flow passed `expectAccessible` on authenticated Home.
Measured minimum contrast among visible code-token roles in the local captures
was **6.34:1** in Studio, **6.14:1** in Terminal, **6.98:1** in Reader, and
**4.65:1** with the custom package's author-chosen color overrides. The
malformed Haskell block has no colored roles by design. Narrow code blocks
retain horizontal scrolling without expanding the page. These are scoped browser
observations, not a guarantee that arbitrary custom package colors meet contrast
requirements.

## Decision

Retain the pinned host `tree-sitter-highlight` engine and broad catalog. The
material regressions came from one grammar's malformed-input cascade, a missing
Emacs Lisp query pattern, a too-coarse capture-to-hook mapping and a renderer
EOF detail; changing engines would not cure the shared Haskell parse or
source-fidelity constraints. Preserve the existing budgets, typed errors,
class-only sanitization and source contract. Add only reviewed hooks, with CSS
scoped to Post-body `pre code`; refresh stored projections through new durable
version 2 rather than altering authored source. The CLI's absence and the
shared-query limitation are recorded rather than presented as independent
confirmation.
