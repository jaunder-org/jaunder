# ADR-0106: The wasm size budget is on raw bytes, not compressed

- Status: accepted
- Date: 2026-08-06
- Issue: [#836](https://github.com/jaunder-org/jaunder/issues/836)

## Context

#818 decomposed the firefox/chromium boot gap and found that firefox's wasm
compile+instantiate accounts for **80.5–87.6%** of it, at roughly **88 ms of
compile per MiB of raw wasm**. SpiderMonkey's compile throughput is not ours to
change; the volume it must compile is.

#836 acted on that: `wasm-opt -Oz` entered `devtool csr-bundle`, cutting the
shipped `pkg/jaunder.wasm` from **5 350 591** to **2 267 063** raw bytes.
Nothing stopped it growing straight back. A dependency bump, a new feature
reaching the client, or a silent weakening of the optimisation level would all
restore the cost without anyone noticing — the previous state of affairs was
precisely that `audit-wasm` existed, was accurate, and was never run.

Two facts shape what the budget must measure:

- The artifact is served **brotli-compressed** (617 871 bytes), so the number a
  reader's instinct reaches for — "what users download" — is not the number that
  governs boot.
- The wasm compiler's input is the **decompressed** artifact. Compression
  happens on the wire; compilation happens on the bytes.

## Decision

`cargo xtask validate` fails when the **raw** byte count of `pkg/jaunder.wasm`
exceeds `WASM_RAW_CEILING_BYTES`, a committed constant in
`xtask/src/wasm_budget.rs`.

**The budget is on raw bytes, and must stay that way.** A budget on the
compressed figure would be satisfied by a change that compresses better while
compiling slower — it would measure the wrong cost and pass. Anyone whose
instinct says "surely we should measure what users download" is reading the
artifact as a download; here it is a _compiler input_. This ADR exists mainly to
say so, because the change looks like an obvious improvement right up until you
ask what the number is for.

**The ceiling carries explicit headroom, currently 3.2%.** A zero-headroom
ratchet turns red on any innocent dependency bump, and since the only available
fix is to raise the number, the gate would train people to raise it reflexively
and so lose its authority. Headroom makes the tolerance reviewable instead of
implied.

The headroom is bounded on both sides. It sits **below** what the next weaker
optimisation level produces (`-Os`: 2 357 119; `-O2`: 2 390 164), so the
likeliest way to lose this win — a downgrade of the level — lands above the
ceiling and fails. A unit test asserts that relationship, so widening the
headroom past it is a deliberate act with a red test, not an accident.

The ceiling is lowered **deliberately, in the same commit as the win that earns
it**. `validate` additionally reports drift from the size #836 achieved, so
erosion _inside_ the headroom is visible before it reaches the ceiling.

The step reads the same measurement `cargo xtask audit-wasm` produces, so the
gate and the tool cannot disagree about what the bundle weighs.

## Consequences

- `cargo xtask validate` gains a `nix build .#site`. It is deliberately **not**
  added to `cargo xtask check`, the pre-commit gate, which must stay fast.
- The ceiling becomes a real review surface: raising it is a diff someone must
  justify.
- `audit-wasm`'s "this is a manual tool, not part of `check`/`validate`" note is
  now false for the totals path and has been corrected. `--breakdown` remains
  manual.
- The budget covers **size only**. It cannot tell whether the bundle still
  _works_ — that is the e2e suite's job, and a size gate passing on a broken
  bundle is a real failure mode to keep in mind.
- This does not constrain `ADR-0028`'s devtool/xtask boundary: the budget is
  host-side analysis and lives in xtask for exactly the reason that ADR gives.
- Raw size is a proxy. #836's own measurement showed **1.28 MiB of the 3.08 MiB
  saved was the wasm name section** — a custom section engines skip rather than
  compile — so raw bytes and compile time are not perfectly proportional. The
  budget is still the right guard, because a regression in either would show up
  here; but a future reader should not read "raw bytes fell by X%" as "compile
  time fell by X%".
