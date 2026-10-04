# #1678 — comparative syntax-highlighting evidence

## Method and scope

The checked-in `host/src/code_highlight/quality/catalog.rs` pins UTF-8 byte
starts, source text and typed expected roles for all **42** grammar variants.
The test renders each canonical label through both Org and Markdown, compares
decoded code to the same exporter's unknown-language/plain rendering, and checks
role coverage at each exact range even when captures nest. A separate guard
rejects any rendered category spanning at least 90% of nonblank sample bytes,
including categories absent from the expected-range list. An alias test renders
every declared label through both exporters and compares its token markup with
the canonical label. The checked-in Haskell and Emacs Lisp fixtures are
byte-for-byte copies of their current public `<code>` elements. The Haskell
author corrected the originally reported unterminated pragma and one trailing LF
after the first capture; the exact original is preserved separately as a
historical malformed regression, not silently rewritten. Quoted and
unknown-language cases remain separate regressions.

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

| Corpus case                                              | Reference/query evidence                                                                                                                                                                                                                     | Prior Jaunder output                                                                                 | Reviewed integrated output                                                                                                                                            |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Historical Haskell before the author's pragma correction | Both upstream and bundled queries capture **over 90% of all source bytes** as `keyword.directive`: the parser's entire tree is `(haskell (pragma))` without an error node. This is a grammar recovery behavior, not a missing palette color. | Every line was wrapped as `j-syn-keyword`, including imports and blank lines.                        | Intact escaped plain code for this unclosed leading pragma; a well-formed pragma still receives multiple roles. No invented edit to authored source.                  |
| Current Haskell with corrected closing `#-}`             | The pinned production query now emits keyword, type, operator, punctuation, variable, function and string scopes across the code instead of a whole-block directive.                                                                         | The currently deployed renderer has 170 token spans in seven classes on the exact 1,160-byte source. | Jaunder on this branch has 170 token spans on the same decoded bytes; eight call heads gain the distinct `function-call` role while other captures remain structured. |
| Production Emacs Lisp call heads                         | The upstream query captures **zero bytes** as `function.call` on the production excerpt. A reviewed first-symbol-of-list query captures `add-to-list` and `package-list-packages`.                                                           | Punctuation, `progn`, and strings had roles, but ordinary call heads were plain.                     | `j-syn-function-call` marks evaluated heads; a bounded quote-aware AST walk suppresses literal data but retains unquoted expressions, even with nested quasiquotes.   |
| Diff addition/removal                                    | Bundled captures name `diff.plus` and `diff.minus` distinctly.                                                                                                                                                                               | Both mapped to `j-syn-string`.                                                                       | Closed `j-syn-diff-plus`/`j-syn-diff-minus` roles use themeable positive/negative colors.                                                                             |
| Markdown code heading/quote                              | Bundled captures name `markup.heading` and `markup.quote` distinctly. An EOF zero-width capture can yield one phantom renderer line.                                                                                                         | Both mapped to `j-syn-string`; the phantom line changed decoded text on a trailing block quote.      | Closed `j-syn-heading`/`j-syn-quote` roles, with renderer output limited to the exporter's actual source lines and authored CRLF boundaries restored.                 |
| Unknown language and alias coverage                      | No parser or capture is available for the unknown label; catalog/aliases all resolve to the same pinned grammar per label.                                                                                                                   | Escaped plain code for unknown, canonical markup for aliases.                                        | Same eligibility and plain fallback; source-equivalence and alias checks now cover every grammar and both formats.                                                    |

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
(offline queue empty). These captures predate the removal of an unnecessary
version-2 checkpoint reset; that change affects startup bookkeeping, not the
renderer, stored HTML, or visual output. The final enqueue-only migration has
separate dual-backend tests. Chromium captures compare the reported production
excerpts at the original capture with local recreations using those same
historical bytes and supported labels after it; the live production permalinks
have **not** been deployed with this change. The historical malformed Haskell
excerpt becomes intact plain code rather than a whole-block keyword cascade. The
unchanged Emacs Lisp excerpt retains existing roles and now distinguishes
evaluated call heads.

After the author corrected Haskell, the public code block was recaptured as
`host/src/code_highlight/fixtures/production-haskell.hs` (SHA-256
`8593ca9132f7d96d6b59016a58a8a0ff6834d67b98e1f221424e0efdebb26e9c`). The current
production HTML and the local branch both carry **170 token spans** on those
exact 1,160 bytes. The branch distinguishes eight function-call captures that
production groups as functions; neither output colors the whole code block as
one role. Current 390px/1280px structural counts and screenshots are transient
under `/tmp/pi-playwright/issue-1678/corrected/` in `evidence.json`,
`live-current-*.png`, `local-studio-*.png`, `local-custom-*.png` and paired code
crops `pair-corrected-haskell-{mobile,desktop}.png`. These are a comparison of
the old deployed renderer and this branch on corrected source, not a claim that
this branch is deployed. The local corrected Post has eight semantic roles and
170 spans under Studio, Terminal, Reader and the custom Theme Package at both
widths. Minimum observed token contrast was 6.34:1, 6.14:1, 6.98:1 and 5.11:1
respectively; page-level horizontal overflow was zero in each capture.

To make the before/after proof reproducible rather than compare unrelated local
states, `issue-1678-matched` was seeded and captured under `origin/main`
(`df0dc8f72`), then **the same SQLite workspace, four Posts and published Theme
Package** were upgraded and captured under `cd5c28ef`. This is the
**historical** before/after packet: its Haskell bytes now match
`historical-malformed-haskell.hs`, not the corrected current production fixture.
The manifest's Haskell and unchanged Emacs Lisp code hashes are
`e3fcf0b2326dfc74b902a3efd6936f6b0e84e09f4b153c5daeaa47e5f3a6a364` and
`deb7e0040bc9cfb03c5905e800d1af893c864b56714c47af2b1a92340337250f`. Diff and
Markdown exercise the other repaired roles. The manifest and 24 captures per
phase are transient under `/tmp/pi-playwright/issue-1678/matched/`:
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
historical malformed Haskell block has no colored roles by design; the now
corrected public Haskell source is richly tokenized. Narrow code blocks retain
horizontal scrolling without expanding the page. These are scoped browser
observations, not a guarantee that arbitrary custom package colors meet contrast
requirements.

## Decision

Retain the pinned host `tree-sitter-highlight` engine and broad catalog. The
material regressions came from one grammar's malformed-input cascade, a missing
Emacs Lisp query pattern, a too-coarse capture-to-hook mapping and a renderer
EOF detail; changing engines would not cure the shared Haskell parse or
source-fidelity constraints. Preserve the existing budgets, typed errors,
class-only sanitization and source contract. Add only reviewed hooks, with CSS
scoped to Post-body `pre code`; refresh stored projections by enqueueing the
existing offline rebuild without restarting version-1 bounded progress or
altering authored source. The CLI's absence and the shared-query limitation are
recorded rather than presented as independent confirmation.
