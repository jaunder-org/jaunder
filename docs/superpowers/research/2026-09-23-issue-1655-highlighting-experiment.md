# #1655 highlighting experiment — no-go at the original latency gate

**Subsequent decision:** The owner accepted full warm p95 below 1 second for 64
KiB pathological blocks and authorized a production attempt without a
Tree-sitter CLI or alternative-library profiling comparison. The measurements
below remain the historical evidence for the failed **original** +25 ms overhead
limit, not the revised production verdict. The new attempt still needs
integrated safety/fidelity and final binary-size proof.

The original approved
[spec](../specs/2026-09-23-issue-1655-org-syntax-highlighting.md) and
[outline](../plans/2026-09-23-issue-1655-org-syntax-highlighting.md) required at
most 25 ms additional warm p95 at 64 KiB in **each** Org/Markdown × Emacs
Lisp/Haskell pair. Direct Tree-sitter missed this gate in all four pairs. The
conditional production renderer, sanitizer, CSS, error-propagation, ADR draft
and architecture edits were **reverted**, without attempting the existing-Post
refresh or opening a PR. This report and the reproducible isolated probe remain
for a revised design decision; no partial feature is shipping.

## Unchanged baseline

- Checkout HEAD: `9ade8021bb823ba8753769a309df16d5168475d6`; devShell release
  profile, `devtool run -- cargo build --release -p jaunder --bin jaunder`
  followed by
  `devtool run -- strip -o .xtask/highlight-experiment/baseline-jaunder target/release/jaunder`.
- Stripped server: **39,505,472 bytes**, SHA-256
  `56bfa937e56b971807e626b38d844dd09cc7897050e74de9191621a4afdb7e79`.
- Benchmark:
  `devtool run -- cargo run --release -p host --example code_block_bench`. The
  preliminary harness used four format/language pairs × 1/8/64 KiB **authored
  source**, 30 fresh-process cold renders and 100 in-process warm renders
  through `host::render::render`, including export and sanitization. Both
  exporters add an LF, so decoded code was one byte larger than each table size.
  In particular, 64 KiB of source becomes 65,537 decoded bytes: this preliminary
  table **cannot serve as the final 64 KiB comparison**. After correcting the
  checked-in `host/examples/code_block_bench.rs` to measure decoded sizes, the
  unchanged host was rerun with the corrected corpus. The corrected baseline and
  temporary highlighted run are paired below; preliminary raw samples are
  retained at
  [baseline-preliminary.jsonl](../../../tools/issue-1655-highlight-probe/evidence/baseline-preliminary.jsonl).
  The fixture code is **synthetic**, repeated to size; real source was
  separately evaluated for readability. Cold timings measure render after
  process start, excluding OS startup. Percentiles are nearest-rank, in ms:

| Format/language       |   Size |  Cold p50 / p95 |  Warm p50 / p95 |
| --------------------- | -----: | --------------: | --------------: |
| Org / Emacs Lisp      |  1 KiB |   0.300 / 0.397 |   0.213 / 0.289 |
| Org / Emacs Lisp      |  8 KiB |   1.716 / 2.316 |   1.695 / 2.774 |
| Org / Emacs Lisp      | 64 KiB | 13.075 / 17.717 | 12.943 / 16.525 |
| Org / Haskell         |  1 KiB |   0.275 / 0.405 |   0.182 / 0.315 |
| Org / Haskell         |  8 KiB |   1.502 / 2.286 |   1.336 / 2.233 |
| Org / Haskell         | 64 KiB | 11.169 / 14.154 | 10.831 / 13.278 |
| Markdown / Emacs Lisp |  1 KiB |   0.205 / 0.341 |   0.121 / 0.151 |
| Markdown / Emacs Lisp |  8 KiB |   1.000 / 1.071 |   0.945 / 1.427 |
| Markdown / Emacs Lisp | 64 KiB |  7.577 / 10.647 |  7.533 / 11.437 |
| Markdown / Haskell    |  1 KiB |   0.171 / 0.273 |   0.081 / 0.143 |
| Markdown / Haskell    |  8 KiB |   0.693 / 1.147 |   0.563 / 0.941 |
| Markdown / Haskell    | 64 KiB |   4.895 / 7.743 |   4.584 / 6.945 |

## Candidate viability (isolated, not integrated)

- Direct
  [`tree-sitter-highlight` 0.27.0](https://docs.rs/tree-sitter-highlight/0.27.0/tree_sitter_highlight/)
  accepts both
  [`tree-sitter-elisp` 1.7.2](https://docs.rs/tree-sitter-elisp/1.7.2/tree_sitter_elisp/)
  and
  [`tree-sitter-haskell` 0.23.1](https://docs.rs/tree-sitter-haskell/0.23.1/tree_sitter_haskell/)
  grammars and their highlight queries. The pinned isolated harness is at
  `tools/issue-1655-highlight-probe/`; run
  `devtool run -- cargo run --release --locked --manifest-path tools/issue-1655-highlight-probe/Cargo.toml`.
  Real source samples:
  [Emacs Lisp](https://tendentious.org/~mdorman/2025/12/14/basics-of-consult-and-embark)
  and
  [Haskell](https://tendentious.org/~mdorman/2013/08/26/using-user-authentication-with-couchdb-and-couchdb-conduit).
  It reproduces the present Org and Markdown exporter calls and options, then
  verifies highlighted decoded `<code>` text equals the same exporter's
  unhighlighted decoded text after span removal, for all four real-sample pairs
  and both-format/language malformed, HTML-looking, Unicode/shortcode-looking
  and exactly 65,536-byte exported inputs. The exporters append an LF, so the
  limit fixture uses 65,535 source bytes. This isolated proof **does not replace
  tests at the integrated host/render/sanitizer boundary**.
- The bundled Haskell query has a trailing blanket `(variable) @type` capture;
  without removing that one pattern, it overrides the earlier function/value
  captures and incorrectly styles ordinary variables as types. The scratch
  experiment removes it and preserves actual `(name) @type`, producing visibly
  distinct function, variable, type, string, comment and keyword regions on the
  real sample. This must be pinned and tested if adopted; silently accepting the
  upstream query would be misleading. `HtmlRenderer::lines()` adds a final
  newline for unterminated input: the harness removes only this invented LF when
  the original payload lacks it. The exact-text proof caught this on the 64 KiB
  boundary.
- [Syntastica 0.6.1](https://docs.rs/syntastica/0.6.1/syntastica/) bundles
  Haskell but neither its
  [parser collection](https://docs.rs/syntastica-parsers/0.6.1/syntastica_parsers/)
  nor its
  [git parser collection](https://docs.rs/syntastica-parsers-git/0.6.1/syntastica_parsers_git/)
  includes Emacs Lisp. Supporting it requires a custom
  [`LanguageSet`](https://docs.rs/syntastica/0.6.1/syntastica/language_set/trait.LanguageSet.html).
  Its default
  [HTML renderer](https://docs.rs/syntastica/0.6.1/syntastica/renderer/struct.HtmlRenderer.html)
  uses inline styles, incompatible with the approved class-only sanitizer/theme
  contract without replacing the renderer. Direct Tree-sitter is the
  lower-complexity initial candidate, **conditional on integrated safety,
  fidelity, performance and size results**.

## Integrated trial and verdict

A temporary host production-path implementation highlighted eligible Org source
blocks and Markdown fences at their exporter event seams, passed focused
four-pair text-fidelity, eligibility, fallback, shortcode and common sanitizer
tests, and propagated typed render errors through create/update callers. It was
removed after measurement; it was **not** tested against the proposed
existing-Post refresh, a real Theme Package, or the full e2e matrix.

The same pinned devShell release profile, `host::render::render` exporter and
sanitizer path, synthetic fixture generator, 30 fresh-process cold and 100
in-process warm samples were used for the corrected baseline and trial. Each
fixture requests the stated _decoded_ `<code>` UTF-8 byte size. The raw paired
samples are
[baseline-corrected.jsonl](../../../tools/issue-1655-highlight-probe/evidence/baseline-corrected.jsonl)
and
[highlighted.jsonl](../../../tools/issue-1655-highlight-probe/evidence/highlighted.jsonl).
Warm nearest-rank p95 (milliseconds; delta = highlighted p95 − baseline p95):

| Format/language       | Decoded size | Baseline | Highlighted |   Delta |
| --------------------- | -----------: | -------: | ----------: | ------: |
| Org / Emacs Lisp      |        1 KiB |     0.20 |        1.39 |   +1.18 |
| Org / Emacs Lisp      |        8 KiB |     1.46 |       11.25 |   +9.79 |
| Org / Emacs Lisp      |       64 KiB |    11.36 |       63.31 |  +51.95 |
| Org / Haskell         |        1 KiB |     0.23 |        2.43 |   +2.21 |
| Org / Haskell         |        8 KiB |     1.95 |       18.71 |  +16.76 |
| Org / Haskell         |       64 KiB |    10.25 |      336.80 | +326.55 |
| Markdown / Emacs Lisp |        1 KiB |     0.23 |        1.18 |   +0.96 |
| Markdown / Emacs Lisp |        8 KiB |     0.81 |        8.98 |   +8.17 |
| Markdown / Emacs Lisp |       64 KiB |     6.92 |       53.77 |  +46.85 |
| Markdown / Haskell    |        1 KiB |     0.15 |        3.56 |   +3.42 |
| Markdown / Haskell    |        8 KiB |     0.51 |       15.75 |  +15.24 |
| Markdown / Haskell    |       64 KiB |     4.20 |      183.14 | +178.94 |

The failure is not an isolated p95 outlier: at 64 KiB, the trial's warm medians
were 47.98 / 167.68 / 46.40 / 129.51 ms respectively, compared with baseline
medians 10.88 / 9.04 / 6.28 / 3.85 ms. The trial's minimum observed Haskell
render was over 120 ms. The repeated synthetic Haskell declaration fixture may
amplify parser recovery cost; the approved gate nevertheless uses this fixed 64
KiB corpus, so its measured latency requires a revised design or budget rather
than an unapproved smaller cutoff.

The temporary integrated stripped server was **44,105,984 bytes**, SHA-256
`ab66a47bdacd6f7682eeee4ea36bcc91221a85fbff1410c74bd8c3b9a42b9567`: **+4,600,512
bytes (4.387 MiB)** over the unchanged binary, inside the 5 MiB limit with just
642,368 bytes of headroom _before_ refresh code. The host-only closure
introduced `tree-sitter-highlight` 0.27.0, `tree-sitter-elisp` 1.7.2,
`tree-sitter-haskell` 0.23.1 and their Tree-sitter dependencies; the isolated
probe lockfile pins the experimental dependencies. This size is **not a final
production-size pass**, because the production refresh was never built.

Before changing CSS, a local named sandbox captured uncolored permalink
screenshots for all four fixtures plus narrow and Home views under Studio.
Transient evidence is in `/tmp/pi-playwright/milestone-22-11/issue-1655/` (not a
product snapshot). In-browser CSS injection confirmed an in-code token could use
`rgb(61, 61, 59)` against base `rgb(26, 26, 25)` while a forged span outside
code remained at the base color; a simulated variable override changed only the
in-code token to `rgb(0, 90, 120)`. No after screenshots or real imported Theme
Package exist because the feature did not meet the latency gate.

The trial production patch was intentionally reverted rather than checked in;
its raw integrated timing samples cannot be rerun from this report alone. The
retained source-and-query probe can rerun candidate/fidelity checks, and the
retained benchmark reproduces the unmodified baseline. The newly authorized
production attempt must re-establish paired integrated measurements against the
revised criterion and satisfy the other gates before shipping.
