# Spec — issue #945: five stale in-source comments

**Issue:** [#945](https://github.com/jaunder-org/jaunder/issues/945) **Branch:**
`issue-945-stale-comments`

## Problem

Five in-source comments describe machinery the code no longer has. Each would
give the next reader a wrong mental model, and three of them justify current
code by reference to a design that was deliberately replaced — the defect class
`CONTRIBUTING.md` now names. One of the five (item E) is not only a comment
defect: the code it documents genuinely misbehaves.

They surfaced during the #927 replay of `docs/ARCHITECTURE.md` against source.
`docs/ARCHITECTURE.md` and several ADRs were corrected there; this spec corrects
the code side, plus two prose copies of item B's error that #927 did not reach.

## Scope

All five items land in this cycle (decided in interview). No ADR is warranted:
nothing here is a novel architectural decision — item E restores the behaviour
its own docstring already promises.

## House rules that constrain the fixes

- **`CONTRIBUTING.md:898-907` — comment against the present, not the past.** No
  replacement comment may read "the gate used to write the name" or "no longer
  writes". State what is true now (`#[macros::server]` derives it) and keep the
  issue number; drop the narrative.
  - **Carve-out that applies here:** `extract.rs`'s module doc argues _forward
    matching is right because the naming regime can change_. That is a
    present-tense claim about robustness, not archaeology, and stays.
- **ADR annotation precedent (#927).** A decision record is annotated with a
  dated blockquote, not rewritten — see
  `docs/adr/0052-devtool-unifies-static-checks.md:55-61`. Item B's ADR-0081 copy
  follows that form. Non-ADR prose (`docs/observability.md`) is edited directly.

## Items

### A. `host/src/metrics.rs` module doc — the cardinality mechanism

`host/src/metrics.rs:4` says "Helper arguments are bounded enums, so a call site
can never emit an unbounded attribute." Not every helper is enum-derived:
`atompub_request` (`host/src/metrics.rs:232`) takes `op: &'static str`, supplied
by `atompub_op` (defined `server/src/atompub/mod.rs:72`, called at `:56`) — a
matched-route-plus-method lookup.

The **invariant** (no caller-supplied text reaches a label) holds; the stated
**mechanism** does not. `docs/ARCHITECTURE.md:1175-1179` already carries the
accurate wording; the module doc adopts it.

### B. The `server-fn-tracing` attribution — six live copies

Comments and prose in **six** places attribute the `web.<vertical>.<ident>` span
name to the `server-fn-tracing` gate. Since #714 the name is derived by
`#[macros::server]` (`macros/src/server_fn.rs:132`, emitted at `:177`).

| Site                                                    | Kind            | Treatment        |
| ------------------------------------------------------- | --------------- | ---------------- |
| `xtask/src/server_fn_coverage/extract.rs:28`            | module doc      | edit             |
| `xtask/src/server_fn_coverage/extract.rs:71`            | const doc       | edit             |
| `xtask/src/server_fn_coverage/extract.rs:109`           | fn doc          | edit             |
| `xtask/src/server_fn_coverage/extract.rs:455`           | inline, in test | edit             |
| `docs/observability.md:246-247`                         | live prose doc  | edit             |
| `docs/adr/0081-empirical-server-fn-flow-coverage.md:64` | decision record | dated annotation |

> **Two corrections to the issue.** (1) It cites three `extract.rs` sites at
> `:72`, `:110`, `:457`; the tree at fork point has **four**, at `:28`, `:71`,
> `:109`, `:455` — the module doc carries it too. (2) It scopes the item to
> `extract.rs`; the identical wrong attribution is also live in
> `docs/observability.md` and ADR-0081, which #927 did not correct. Leaving them
> would keep the defect in the two documents a reader is most likely to consult,
> so they are folded in.

**What the gate actually does today**
(`xtask/src/steps/server_fn_tracing_check.rs:17-30`) — any replacement wording
must match this, and three rules, not one:

1. PII discipline as a default-deny type allowlist (`RECORDABLE_TYPES`): every
   parameter is skipped by name, covered by `skip_all`, or has a listed type.
2. A parameter bound by a pattern cannot be skipped by name, so it is refused
   unless `skip_all` covers it.
3. An unmodelled attribute argument is refused.

**Citations.** The four `extract.rs` sites currently cite `#511, ADR-0011`.
ADR-0011's span-name half is **partly superseded**
(`docs/adr/0011-unified-observability.md:202`), so a bare ADR-0011 pointer for
the name is itself misleading. Replacement comments cite **#714** for the
derivation; where an ADR pointer is kept it must be to ADR-0011's #714 addendum
(`:316`), not the superseded body.

### C. `web/src/audiences/component.rs:48` — names a method that does not exist

The comment block at `:47-50` credits `Invalidator::patched`. `Invalidator`
(`web/src/reactive/mod.rs:33`) has no such method. The real symbol is the free
function `client::reactive::patched` (`client/src/reactive.rs:52`), called at
`:54`.

### D. `flake.nix:1146` — a count that has gone stale three times

Says "The 7 non-compiling static checks (#188)". There are eight
(`tools/devtool/src/check.rs:17`). ADR-0052 said seven twice (annotated in
#927); `CONTRIBUTING.md` said eight and was right.

**Decided in interview:** drop the number rather than update it. A count in
prose that no gate checks is the drift source; this is its third home.

`tools/devtool/src/check.rs:1` and `:12` also state "8", and are **left alone**
— not because they cannot drift, but because they are correct today and sit in
the file that owns `ALL`, where a reader can check them against the list without
leaving the buffer. `CONTRIBUTING.md` is likewise already correct and untouched.

### E. `elisp/jaunder-publish.el` — the retry handler is wider than its docstring

`jaunder--create-with-retry` (`:151`) documents retrying "a signalled transport
error or a 5xx status". The handler at `:168` catches bare `(error …)`, so it
retries **any** signalled error — including the configuration error
`jaunder--auth-secret` raises when no auth-source entry exists
(`elisp/jaunder-transport.el:81`). A user with no stored credential waits
through ~3s of backoff for a message that was available immediately.

**Decided in interview:** narrow the handler, not the docstring — retrying a
configuration error cannot succeed.

The only **transport** condition `jaunder--http-request` signals is `plz-error`,
re-signalled when a failure carries no response
(`elisp/jaunder-transport.el:118-123`). Everything else it can signal is a
configuration error — `jaunder--auth-secret`, or the header prelude at
`:107-113` (`jaunder--active-username` / `jaunder--active-base-url`), which sit
outside the `condition-case` entirely. So catching `plz-error` is exactly the
set the docstring names, and the other conditions are correctly no longer
retried. `condition-case` matches by hierarchy, so `plz-error` also catches
`plz`'s `plz-curl-error` / `plz-http-error` subtypes — do **not** enumerate
them.

**Test debt this exposes:** `jaunder-create-retry-exhausts-on-transport-error`
(`elisp/test/jaunder-test.el:1201`) simulates a transport error with
`(error "boom")` — it encodes the wrong model and will fail once the handler
narrows. It must be rewritten to signal a real `plz-error`. (`make-plz-response`
is already used elsewhere in that file.)

## Acceptance criteria

- **AC-1** `host/src/metrics.rs`'s module doc no longer claims all helper
  arguments are bounded enums. It states the two admissible forms (bounded enum,
  or `&'static str` from a closed set the call site cannot widen) and names
  `atompub_request`'s `op` as the second case, consistent with
  `docs/ARCHITECTURE.md:1175-1179`.
- **AC-2** All six sites in item B's table stop crediting the gate with the
  `web.` span name. Concretely: a search for `server-fn-tracing` across
  `xtask/src/server_fn_coverage/extract.rs` and `docs/observability.md` yields
  no statement that the gate writes or derives a span name; every surviving
  mention describes only the three rules listed above, and each corrected site
  attributes the name to `#[macros::server]` citing #714.
  - **Carve-out — ADR-0081's original bullet (`:64`) still says the gate
    _writes_ the name, and must.** AC-4 preserves decision prose; the
    annotation, not an edit, is what corrects it. That one hit does not fail
    this AC. It is the same shape as AC-6's ADR-0052 carve-out.
- **AC-3** No replacement comment from AC-2 narrates the change ("used to", "no
  longer writes", "since #714 the gate stopped…"), per
  `CONTRIBUTING.md:898-907`. `extract.rs`'s forward-matching rationale survives,
  stated as a present-tense claim.
- **AC-4** ADR-0081's correction is a dated annotation blockquote in the form of
  `docs/adr/0052-devtool-unifies-static-checks.md:55-61`; its original decision
  prose is not rewritten.
- **AC-5** `web/src/audiences/component.rs`'s comment names
  `client::reactive::patched`. The named path resolves to a symbol that exists.
- **AC-6** `flake.nix:1146` states no count of the non-compiling static checks.
  A search for `7 non-compiling` under `flake.nix`, `tools/`, `xtask/`, and
  `CONTRIBUTING.md` returns nothing. (Hits remain in `docs/adr/0052-…:41` —
  annotated by #927, prose deliberately preserved — and in `docs/archive/`; both
  are out of scope and do not fail this AC.)
- **AC-7** `jaunder--create-with-retry` retries only `plz-error`. **Proved by a
  new test:** given a `jaunder--http-request` that signals a non-`plz-error`
  (the missing auth-source entry), the call errors after **exactly one** attempt
  and calls `sleep-for` zero times.
- **AC-8** Given a `jaunder--http-request` that signals `plz-error` every time,
  the call still errors after **exactly three** attempts — proved by rewriting
  `jaunder-create-retry-exhausts-on-transport-error` to signal a real
  `plz-error` rather than a bare `error`.
- **AC-9** The 5xx-retry and 4xx-no-retry tests
  (`elisp/test/jaunder-test.el:1171`, `:1189`) still pass unmodified —
  status-driven retry is untouched.
- **AC-10** `jaunder--create-with-retry`'s docstring is unchanged, because
  narrowing the handler makes it true.
- **AC-11** `devtool run -- cargo xtask validate --no-e2e` is green, including
  `ert`, `byte-compile`, and `prettier` (the two docs edits).

## Out of scope

- `docs/ARCHITECTURE.md` and the ADR-0011 / ADR-0052 annotations — already
  corrected by #927.
- `CONTRIBUTING.md`'s check count and `tools/devtool/src/check.rs`'s — correct
  today (see item D).
- `docs/archive/` — a frozen record.
- A gate enforcing the check count in prose (considered, declined in interview:
  dropping the number removes the drift without new machinery).
- The wider ~335-site stale-comment population #930 measured. This issue is the
  five found by hand, plus the two extra copies of item B's error.
