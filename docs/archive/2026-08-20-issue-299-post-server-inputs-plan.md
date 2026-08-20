# Retire post server-function arity suppression implementation plan

Spec:
[`docs/superpowers/specs/2026-08-20-issue-299-post-server-inputs.md`](../specs/2026-08-20-issue-299-post-server-inputs.md)
Issue: [#299](https://github.com/jaunder-org/jaunder/issues/299)

## Review header

**Goal:** Remove the retired #299 `clippy::too_many_arguments` wasm-clippy
allowance from both the host xtask definition and the matching Nix check, then
prove the stricter lint path.

**Scope in:** `xtask/src/steps/static_checks.rs`, `flake.nix`, this spec/plan.  
**Scope out:** post form behavior, `PostInputs`, server-function signatures,
endpoint paths, request JSON shape, storage mutation semantics.

**Tasks:**

1. Remove the xtask wasm-clippy allowance and update its argv test.
2. Mirror the stricter wasm-clippy command in `flake.nix`.
3. Run the focused post e2e proof and full check gate.
4. Commit the checked deliverable.
5. Review the completed deliverable.

**Key risks/decisions:**

- Current `PostInputs` aggregation already completed the original arity fix;
  this plan must not rename or reshape it.
- The stale allowance appears in two command definitions and one argv unit test;
  missing any one leaves issue #299 incomplete.
- `cargo xtask check` is the behavioral proof for the stricter wasm-clippy
  command because it runs xtask unit tests plus wasm clippy.
- The focused `posts.spec.ts` e2e run guards against accidental private CSR
  request-shape drift while touching only lint plumbing.

**For agentic workers:** Execute with `jaunder-iterate`; use `jaunder-dispatch`
only if delegating a whole task. Tick each task checkbox before its commit gate.

## Global constraints

- Do not change `web/src/posts/api.rs` or `web/src/posts/component.rs` unless
  verification exposes a real existing failure; the approved spec says the post
  request aggregate is already correct.
- Do not introduce any new lint suppression.
- Use `devtool run -- ...` for every command whose output or exit status
  matters.
- Keep commits focused and omit `Co-Authored-By` trailers.

## File structure

- `xtask/src/steps/static_checks.rs` — remove the temporary
  `-A clippy::too_many_arguments`, simplify the explanatory comment, and update
  `wasm_clippy_lints_web_client_and_csr`.
- `flake.nix` — remove the matching temporary `-A clippy::too_many_arguments`
  from `cargoClippyExtraArgs` and simplify the comment.
- `docs/superpowers/specs/2026-08-20-issue-299-post-server-inputs.md` — approved
  specification; no implementation edits expected.
- `docs/superpowers/plans/2026-08-20-issue-299-post-server-inputs.md` — this
  plan; tick boxes as work completes.

## Task 1: Remove the xtask wasm-clippy allowance

**Files:**

- Edit: `xtask/src/steps/static_checks.rs`

**Interfaces:**

- Consumes: current `specs(mode)` output for the `wasm-clippy` `StepSpec`.
- Produces: a `wasm-clippy` command that lints `web`, `client`, and `csr` on
  `wasm32-unknown-unknown` with `-D warnings` and no
  `-A clippy::too_many_arguments`.

- [x] **Step 1: Edit the command and comment**

Remove these two argv entries from the `StepSpec`:

```rust
"-A",
"clippy::too_many_arguments",
```

Rewrite the comment above the step so it only explains why wasm-target clippy
exists and which crates it covers. Delete the stale #299 temporary-allowance
paragraph.

- [x] **Step 2: Update the argv unit test**

Remove the same two entries from `wasm_clippy_lints_web_client_and_csr`'s
expected array. The test should still assert `-D warnings` is present and should
not assert any `-A` flag.

- [x] **Step 3: Run the focused xtask test**

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml wasm_clippy_lints_web_client_and_csr
```

Expected: pass. This proves the `specs(mode)` contract changed intentionally
before broader checks.

## Task 2: Mirror the stricter wasm-clippy command in Nix

**Files:**

- Edit: `flake.nix`

**Interfaces:**

- Consumes: the xtask wasm-clippy command from Task 1.
- Produces: a `wasm-clippy` derivation with matching package/features/target
  coverage and no temporary `too_many_arguments` allowance.

- [x] **Step 1: Edit the derivation command**

Change `cargoClippyExtraArgs` from concatenating the temporary allowance to a
single stricter string:

```nix
cargoClippyExtraArgs = "-p web -p client -p csr --features csr -- -D warnings";
```

- [x] **Step 2: Update the derivation comment**

Remove the stale paragraph saying the remaining `-A` flag is temporary and
synchronized with xtask. Keep the explanation that the Nix derivation mirrors
host `wasm-clippy` and covers wasm-only code.

## Task 3: Verify the issue contract

**Files:**

- Verify: `xtask/src/steps/static_checks.rs`
- Verify: `flake.nix`
- Verify: `web/src/posts/api.rs`
- Verify: `web/src/posts/component.rs`
- Verify: `end2end/tests/posts.spec.ts`

**Interfaces:**

- Consumes: Task 1 and Task 2 edits.
- Produces: evidence for AC1-AC4.

- [x] **Step 1: Confirm the retired suppression is gone**

Use source search, not shell grep:

```text
too_many_arguments
```

Expected: no production `#[allow(clippy::too_many_arguments)]`; no wasm-clippy
`-A clippy::too_many_arguments` in `xtask/src/steps/static_checks.rs` or
`flake.nix`. Any remaining mention must be historical/spec/plan text or an issue
reference that does not affect the command line.

- [x] **Step 2: Run focused post e2e proof**

```bash
devtool run -- cargo xtask e2e-local posts.spec.ts
```

Expected: pass. This proves post create/edit flows still work through the
existing Leptos-generated client and `PostInputs` request shape.

- [x] **Step 3: Run full check gate**

```bash
devtool run -- cargo xtask check
```

Expected: pass. This proves xtask unit tests, host clippy, wasm-clippy without
the temporary allowance, formatting, static checks, and coverage are green.

## Task 4: Commit the checked deliverable

**Files:**

- Commit: `xtask/src/steps/static_checks.rs`
- Commit: `flake.nix`
- Commit: `docs/superpowers/specs/2026-08-20-issue-299-post-server-inputs.md`
- Commit: `docs/superpowers/plans/2026-08-20-issue-299-post-server-inputs.md`

**Interfaces:**

- Consumes: Task 3's passing evidence.
- Produces: a focused commit for issue #299.

- [x] **Step 1: Tick completed implementation tasks in this plan**

Mark Task 1 through Task 3 checkboxes complete before staging.

- [x] **Step 2: Stage the checked files**

```bash
devtool run -- git status --short
```

Then stage the checked files explicitly:

```bash
devtool run -- git add xtask/src/steps/static_checks.rs flake.nix docs/superpowers/specs/2026-08-20-issue-299-post-server-inputs.md docs/superpowers/plans/2026-08-20-issue-299-post-server-inputs.md
```

Stage only the issue #299 files listed above, plus any formatter output from
`cargo xtask check` that belongs to those files.

- [x] **Step 3: Commit**

```bash
devtool run -- git commit -m "fix(xtask): retire post arity clippy allowance"
```

Expected: commit succeeds. If the pre-commit hook reformats, stage its owned
output and retry the same commit.

## Task 5: Review the completed deliverable

**Files:**

- Review: whole branch diff against `origin/main`

**Interfaces:**

- Consumes: committed issue #299 branch.
- Produces: final review packet for `jaunder-ship`.

- [x] **Step 1: Capture the branch diff**

```bash
devtool run -- git diff origin/main...HEAD
```

Expected: diff includes only the stricter wasm-clippy configuration and issue
planning artifacts before archive.

- [x] **Step 2: Run `jaunder-review`**

Run the standards/specification review against `origin/main`, with the approved
spec and this plan in scope. Resolve every finding before final ship validation.
