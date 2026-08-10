# #767 — a static gate for cross-language literal agreement

Issue: [#767](https://github.com/jaunder-org/jaunder/issues/767). Milestone:
Developer tooling & DX. Provenance: #251 (the mount-marker rename that created
the duplicated literal), #794 (recorded the same drift risk for the mark
prefix), ADR-0085 (static gates enumerate, they do not search).

## Summary

Two constants are spelled once in Rust and once in TypeScript, with nothing but
a comment on each side holding them together:

| key            | Rust site                                               | TypeScript site                                      | value          |
| -------------- | ------------------------------------------------------- | ---------------------------------------------------- | -------------- |
| `mount-marker` | `csr/src/lib.rs:18` — `mark_ready`'s `inline_js` string | `end2end/tests/mount.ts:10` — `MOUNTED_ATTR`         | `data-mounted` |
| `mark-prefix`  | `client/src/perf/mod.rs:25` — `MARK_PREFIX`             | `end2end/tests/capture-trace.ts:189` — `MARK_PREFIX` | `jaunder.`     |

Neither pair can be collapsed into a single declaration: `mount-marker` crosses
the wasm boundary as an opaque JS string, and `mark-prefix` is read by the
Playwright harness in Node, which cannot import from Rust. #251's D6 decided the
duplication is necessary; it did not give the two sides anything that checks
them.

Drift is silent at build time and maximally expensive at test time. A one-word
typo in an `inline_js` string turns the whole
`{sqlite,postgres}×{chromium,firefox}` matrix red with 60+ Playwright timeouts,
~25 minutes, and no output naming the cause. `cargo xtask check` never runs e2e,
so it cannot see it at all.

This cycle adds a host static check — **`xlang-literal`** — that reads a
declared table of cross-language literal pairs, extracts each side's literal
from source, and fails when a pair's two literals differ.

## Decisions

| ID      | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1**  | The gate is a **table of pairs**, not a bespoke check for `data-mounted`. Each entry is `{key, a: Site, b: Site}`; the gate is the loop. Adding a future pair costs a table row, not a new check.                                                                                                                                                                                                                                                        |
| **D2**  | A `Site` is `{file, anchor, quote}` (see "Site extraction" below). The anchor's job is to **locate the site**, never to characterise a violation — the violation is decided by exact string inequality between the two extracted literals.                                                                                                                                                                                                               |
| **D3**  | **Zero occurrences of the anchor is a hard failure, and so is more than one.** A renamed constant, a moved file, a reformatted line, or a second matching site all produce a loud message naming the file, the anchor, and the count. A gate that matches nothing must never be indistinguishable from a pass.                                                                                                                                           |
| **D4**  | **An unreadable file is a hard failure**, not a skip (ADR-0085 principle 6). So is an anchor found but not followed by a well-formed quoted literal.                                                                                                                                                                                                                                                                                                     |
| **D5**  | The gate asserts **agreement, not a value**. Renaming `data-mounted` to `data-ready` on both sides passes. The gate has no opinion on what the literal should be; it only refuses to let the two sides disagree.                                                                                                                                                                                                                                         |
| **D6**  | The gate ships with **both** pairs above. One entry would not exercise the table.                                                                                                                                                                                                                                                                                                                                                                        |
| **D7**  | Lives at `xtask/src/steps/xlang_literal_check.rs`, registered in **both** `Command::Check` and `Command::Validate`, alongside `e2e_scaffold_check` (`xtask/src/lib.rs:474` and `:519`). Host-only per the xtask invariant — no Nix derivation invokes it. CI covers it through `cargo xtask validate --no-e2e`.                                                                                                                                          |
| **D8**  | **No new dependency.** Extraction is ~40 lines of straight-line Rust over `&str`. `regex` is _not_ added: `xtask` is its own cargo workspace with its own `Cargo.lock`, and `regex` is in neither — adding it would compile `regex` + `regex-automata` + `regex-syntax` into every `cargo xtask` rebuild.                                                                                                                                                |
| **D9**  | Extraction is a **pure function of source text**, unit-tested directly on inline fixtures (the `e2e_scaffold_check::problems` shape).                                                                                                                                                                                                                                                                                                                    |
| **D10** | Site paths are resolved against a **root parameter**, never cwd-relative constants. `run()` passes `Path::new(".")` (xtask always runs from the repo root); the AC9 real-tree test passes `Path::new(env!("CARGO_MANIFEST_DIR")).join("..")`, the `server_fn_registrar_check.rs:677` precedent. `e2e_scaffold_check`'s cwd-relative `"flake.nix"` is deliberately **not** followed: unit tests run with cwd = `xtask/`, which would make AC9 impossible. |
| **D11** | The repo root is a **parameter** of the checking function, not a hardcoded constant, so AC7 (unreadable file) is testable by pointing the gate at a temporary directory rather than by mutating the real tree.                                                                                                                                                                                                                                           |

### Site extraction

A `Site` declares a **literal anchor string** and the **quote character** that
opens the literal following it. Extraction: find the anchor, require exactly one
occurrence, then read from the next `quote` character to the next unescaped
`quote`. The four sites in the shipped table, each verified to occur exactly
once in the real tree:

| site                             | anchor                 | quote |
| -------------------------------- | ---------------------- | ----- |
| `csr/src/lib.rs`                 | `setAttribute(`        | `'`   |
| `end2end/tests/mount.ts`         | `MOUNTED_ATTR = `      | `"`   |
| `client/src/perf/mod.rs`         | `MARK_PREFIX: &str = ` | `"`   |
| `end2end/tests/capture-trace.ts` | `MARK_PREFIX = `       | `"`   |

**Occurrences, not matching lines.** The count in D3 is of anchor occurrences in
the whole file, so two anchors on one line is a failure, not a silent
first-wins.

**Comments are not excluded, and must not need to be.** Each anchor is the
_declaration_ form, which no prose comment contains — `csr/src/lib.rs:11` says
`body[data-mounted]` and does not contain `setAttribute(`; `mount.ts:12–13`
mention `MOUNTED_ATTR` but never `MOUNTED_ATTR = `. This is load-bearing for
AC11, which rewrites exactly those adjacent comments: an anchor that a comment
could match would make prose edits flip the gate's verdict. AC9 is what keeps
this honest.

### Why an anchored locator is compatible with ADR-0085

ADR-0085's Decision is scoped to gates enforcing a **type-safety** invariant, so
it does not straightforwardly reach a cross-language string-agreement gate. Two
of its principles are adopted deliberately anyway, because they are about gate
honesty rather than about types:

- **Principle 6 (fail on unreadable input)** is adopted verbatim as D4.
- **Principle 3's spirit (nothing self-exempts)** holds trivially: there are no
  exemptions. A pair agrees or it does not.

The one principle that appears to cut against this design is **principle 5,
"parse rather than scan."** It does not bite here, and the reason is specific:
principle 5 exists because a line-based scan cannot relate a decode's type on
one line to the SQL on the next, and the workarounds for that are pattern
searches for violations. This invariant spans no lines at all — each literal is
a single-line declaration, and the comparison is exact string equality between
two extracted values, not a judgement about surrounding code. `csr/src/lib.rs`'s
`inline_js` attribute is multi-line, but the literal is not, and nothing about
the other five lines changes what `setAttribute('…'` means.

The deeper reconciliation is the failure direction. The anchor does not decide a
violation; it **locates a site**. An out-of-date locator is loud — zero
occurrences or two, both hard failures (D3) — where an incomplete violation
detector would be silent. That asymmetry is what ADR-0085 is actually about.

**What is not claimed:** that the table is complete. A third duplicated literal
nobody added is unpoliced and recorded nowhere. That limit is stated in the
module doc per ADR-0085's honesty obligation, and it is inherent to a
declared-pair design — there is no structural property distinguishing "a literal
duplicated across languages on purpose" from "two files that happen to share a
string."

## Acceptance criteria

- **AC1 — the gate exists and is wired.** `cargo xtask check` and
  `cargo xtask validate --no-e2e` each report a step named `xlang-literal` in
  `.xtask/last-result.json`'s `steps[]`.

- **AC2 — a clean tree passes.** On the branch tip with no other working-tree
  edits, the `xlang-literal` step is `ok: true`.

- **AC3 — mount-marker drift fails, with a usable message.** A unit test feeds
  fixture sources in which the Rust side says `data-mounted` and the TS side
  says anything else, and asserts the failure detail contains **all five** of:
  the key `mount-marker`, both file paths, and both differing values.

- **AC4 — mark-prefix drift fails, with a usable message.** The same assertion
  for the `mark-prefix` pair.

- **AC5 — a vanished site fails loudly.** A unit test shows that source text in
  which a site's anchor occurs **zero** times yields a failure whose message
  names the file and the anchor and says it was not found — not a pass.

- **AC6 — a duplicated site fails loudly.** A unit test shows that source text
  in which a site's anchor occurs **more than once** yields a failure naming the
  file, the anchor, and the occurrence count — not a pass that silently picks
  the first. A second occurrence **on the same line** as the first must also
  fail.

- **AC7 — an unreadable file fails.** With the repo root pointed at a temporary
  directory lacking the site files (D11), the step fails and its detail names
  the missing path — rather than passing or being skipped.

- **AC8 — agreement, not value.** A unit test shows that changing a pair's
  literal to the same new value on **both** sides passes (D5).

- **AC9 — the real table resolves.** A test runs every table entry against the
  **real repository tree** (resolved per D10) and asserts each site yields
  exactly one anchor occurrence and a well-formed literal, and each pair agrees.
  This is what makes a future refactor of `csr/src/lib.rs`, `mount.ts`, or the
  comments AC11 rewrites fail here rather than silently disarming the gate.

- **AC10 — the honesty limit is documented.** The module doc states that the
  gate polices exactly the declared pairs and cannot discover an undeclared
  duplicated literal (ADR-0085's honesty obligation).

- **AC11 — the counterpart comments point at the gate.** The comments at
  `csr/src/lib.rs:11–14`, `end2end/tests/mount.ts:4–9`,
  `client/src/perf/mod.rs`'s `MARK_PREFIX` doc, and
  `end2end/tests/capture-trace.ts:185–188` say the agreement is now **enforced**
  by `xlang-literal` and name it — so a reader who edits one side is told what
  will catch them. `capture-trace.ts:187–188` currently cites `MOUNTED_ATTR` as
  the drift-prone counterexample; that claim becomes false and must be
  corrected. AC9 must still pass after these edits.

- **AC12 — the ADR is recorded.**
  `docs/adr/0109-cross-language-literal-agreement.md` states that
  cross-language literal agreement is enforced by a declared pair table, and how
  it stands relative to ADR-0085. Written numberless in the drafts pen;
  `cargo xtask adr promote` numbers it at ship.

- **AC13 — the gate is green.** `cargo xtask validate --no-e2e` passes. The new
  module's tests run in the `xtask-tests` step
  (`xtask/src/steps/host_tests.rs`); the Nix coverage gate does **not** apply,
  because `flake.nix:1190` excludes `/xtask/` from the instrumented source.

## Out of scope

- Discovering undeclared cross-language duplicated literals (see the honesty
  limit above). No mechanism proposed.
- Collapsing either duplication into a single source of truth — #251's D6
  decided the Rust side keeps its own literal, and nothing here revisits that.
- Any change to e2e behaviour, the mount marker's value, or the boot-mark names.
