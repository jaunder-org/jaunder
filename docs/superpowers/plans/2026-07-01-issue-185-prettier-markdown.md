# Format Markdown with prettier — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring tracked `*.md` under prettier (`proseWrap: always`) with a flag-day reformat and a check/validate gate via the existing host prettier StepSpec.

**Architecture:** Add a root prettier config + ignore file; decouple `.md` from the coverage hash; reformat the tree in one pure-reformat commit; then widen the host `prettier` StepSpec to `**/*.md`. The nix `prettier-check` sibling is left untouched (its redundancy is routed to a separate issue).

**Tech Stack:** prettier (from `pkgs.prettier`, flake.lock-pinned), Nix (crane coverage src filter), Rust (xtask `StepSpec`).

## Global Constraints

- Prettier config: `{ "proseWrap": "always" }` — `printWidth` **omitted** (relies on prettier's default 80; don't reinforce defaults).
- Prettier binary: `pkgs.prettier`, pinned by flake.lock. No new version work; the lock is the pin.
- Emphasis marker becomes `_` tree-wide (prettier hardcodes `_italic_` / `**bold**`).
- `.prettierignore` covers: `docs/archive/`, `target/`, `result`, `result-*`, `.claude/`.
- Commit messages: conventional-commit style, **no `Co-Authored-By` trailers**.
- Per-commit gate is git-hook-enforced (`check --no-test` + `validate --no-e2e --allow-dirty`); each `git commit` runs it automatically. Run gates via the worktree (Bash tool), not `ctx_execute` (which targets the main repo).
- Prettier is invoked through the devShell: `nix develop -c prettier …`.

---

### Task 1: File the separable-concern issue (static-sibling cleanup)

No code. Capture the separable concern up front so it can be picked up independently (jaunder-start convention).

**Files:** none (GitHub issue).

- [ ] **Step 1: Create the issue**

Use the `jaunder-issues` skill conventions. Title and body:

- Title: `Remove redundant static-check nix *-check siblings (overlap with cargo xtask validate); amend ADR-0031`
- Body (summary): The nix `*-check` derivations `prettier-check`, `rustfmt`, `clippy`, `leptosfmt-check`, `ert-check`, `elisp-fmt-check`, `deny` each duplicate a host StepSpec running the identical tool. They are exercised only by `nix flake check`, which neither CI (`ci.yml` runs `cargo xtask validate --no-e2e` + the e2e matrix) nor the steered `cargo xtask` workflow runs. This is distinct from the load-bearing nix derivations xtask orchestrates (`coverage`, `e2e-*`, `elisp-integration`), which require the sandbox/VM/instrumentation. Proposal: remove the static-sibling class and amend/supersede ADR-0031 (which codified the host-StepSpec + nix-sibling convention using `prettier-check` as the exemplar). Discovered during #185.
- Label: `tooling`. Add to the Jaunder Backlog project (#1), Status `Todo`.

Run (via `gh`):

```bash
gh issue create --repo jaunder-org/jaunder \
  --title "Remove redundant static-check nix *-check siblings (overlap with cargo xtask validate); amend ADR-0031" \
  --label tooling \
  --body "<body above>"
```

Then add it to project #1 (Status defaults to no-status; set Todo) per `jaunder-issues`.

- [ ] **Step 2: Record the new issue number**

Note the issue number in the PR description later (referenced as "spun out of #185").

---

### Task 2: Plumbing — prettier config, ignore file, coverage denylist

Establish the config so the flag-day write uses `proseWrap: always`, and decouple `.md` from the coverage hash **before** the reformat so it doesn't bust the coverage cache.

**Files:**
- Create: `.prettierrc.json`
- Create: `.prettierignore`
- Modify: `flake.nix` (coverage src filter, ~`flake.nix:1177-1183`)

**Interfaces:**
- Produces: a repo-root prettier config that Task 3's `prettier --write` and Task 4's StepSpec both discover automatically.

- [ ] **Step 1: Write `.prettierrc.json`**

```json
{
  "proseWrap": "always"
}
```

- [ ] **Step 2: Write `.prettierignore`**

```
docs/archive/
target/
result
result-*
.claude/
```

- [ ] **Step 3: Add `.md` to the coverage source denylist**

In `flake.nix`, the coverage derivation's `cleanSourceWith` filter (~line 1177):

```nix
                  filter =
                    path: _type:
                    !(pkgs.lib.hasInfix "/xtask/" path)
                    && !(pkgs.lib.hasInfix "/tools/" path)
                    && !(pkgs.lib.hasInfix "/docs/" path)
                    && !(pkgs.lib.hasInfix "/.github/" path)
                    && !(pkgs.lib.hasInfix "/elisp/" path)
                    && !(pkgs.lib.hasSuffix ".md" path);
```

(Add the final `&& !(pkgs.lib.hasSuffix ".md" path)` line.)

- [ ] **Step 4: Sanity-check config discovery**

Run:

```bash
nix develop -c prettier --check .prettierrc.json
```

Expected: PASS (the config file is itself valid, prettier-clean JSON). This also confirms `prettier` is on PATH via the devShell.

- [ ] **Step 5: Commit**

The pre-commit hook runs `check --no-test` + `validate --no-e2e --allow-dirty`. The existing prettier StepSpec still checks `end2end` only (no tracked `end2end/*.md`), so the config addition is gate-neutral here. The flake.nix change triggers one coverage rebuild (expected).

```bash
git add .prettierrc.json .prettierignore flake.nix
git commit -m "build(prettier): add markdown config and exclude *.md from coverage source"
```

Expected: hook green; commit created.

---

### Task 3: Flag day — reformat all living markdown

One pure-reformat commit, reviewable as such.

**Files:** every tracked, non-ignored `*.md` (reformatted in place).

- [ ] **Step 1: Run prettier over the tree**

```bash
nix develop -c prettier --write "**/*.md"
```

Expected: prettier reports the reformatted files (the ~80 living docs; `docs/archive/`, `.claude/`, `target/` skipped per `.prettierignore`).

- [ ] **Step 2: Confirm the diff is pure reformat**

```bash
git diff --stat
```

Expected: only `*.md` files changed; changes are `_`-emphasis + `always` reflow. Spot-check one file with `git diff -- README.md` to confirm no content edits, only wrapping/emphasis.

- [ ] **Step 3: Commit**

The md gate is not wired yet (Task 4), so the pre-commit hook checks `end2end` only and passes. `.md` is now coverage-denied (Task 2), so the reformat does not bust the coverage cache.

```bash
git add -A
git commit -m "style(docs): prettier --write all Markdown (proseWrap: always flag day)"
```

Expected: hook green; commit created.

---

### Task 4: Wire the gate — widen the host prettier StepSpec

Make `cargo xtask check` auto-format markdown and `cargo xtask validate` verify it.

**Files:**
- Modify: `xtask/src/steps/static_checks.rs:34-37`
- Modify: `CONTRIBUTING.md:106`

**Interfaces:**
- Consumes: the `.prettierrc.json` / `.prettierignore` from Task 2 (prettier auto-discovers them).

- [ ] **Step 1: Widen the prettier StepSpec args**

In `xtask/src/steps/static_checks.rs`, replace the `prettier_args` block:

```rust
    // prettier — end2end/ frontend assets + all tracked Markdown (**/*.md,
    // scoped by .prettierignore); proseWrap: always from .prettierrc.json.
    let prettier_args = match mode {
        Mode::Check => vec!["--check", "end2end", "**/*.md"],
        Mode::Fix => vec!["-w", "end2end", "**/*.md"],
    };
```

- [ ] **Step 2: Update CONTRIBUTING.md**

`CONTRIBUTING.md:106` currently:

```
- `prettier --check end2end` checks Playwright and other frontend test assets.
```

Replace with:

```
- `prettier --check end2end '**/*.md'` checks Playwright/frontend test assets and all tracked Markdown (`proseWrap: always`; scoped by `.prettierignore`, which excludes `docs/archive/`).
```

- [ ] **Step 3: Positive check — gate passes on the clean tree**

Run (in the worktree):

```bash
cargo xtask check --no-test
```

Expected: green — `prettier` step now covers `**/*.md` and the tree is already formatted (Task 3), so `-w` produces no changes.

- [ ] **Step 4: Negative check — gate fails on misformatted markdown**

Introduce a deliberate violation, confirm the gate catches it, then revert:

```bash
printf '\n\nThis  is   a deliberately mis-formatted   line with an *emphasis* to force a prettier violation and reflow past the wrap width so proseWrap always has something to do.\n' >> README.md
nix develop -c prettier --check "**/*.md"
```

Expected: prettier **exits non-zero** and lists `README.md` as needing formatting (proving the same args the StepSpec uses reject bad input).

Revert:

```bash
git checkout -- README.md
```

- [ ] **Step 5: Full pre-push gate**

```bash
cargo xtask validate --no-e2e
```

Expected: green (static checks incl. the widened prettier + coverage).

- [ ] **Step 6: Commit**

```bash
git add xtask/src/steps/static_checks.rs CONTRIBUTING.md
git commit -m "build(xtask): gate Markdown with prettier in check/validate"
```

Expected: hook green; commit created.

---

## Self-Review

**Spec coverage:**
- Config files (`.prettierrc.json`, `.prettierignore`) → Task 2. ✓
- Host StepSpec gate (check auto-fix / validate verify) → Task 4. ✓
- `prettier-check` left untouched → no task modifies it (explicit). ✓
- Coverage denylist `.md` exclusion → Task 2 Step 3. ✓
- Version pin → Global Constraints (already satisfied; no task needed). ✓
- Flag-day pure-reformat commit → Task 3. ✓
- Negative-check acceptance → Task 4 Step 4. ✓
- Separable concern filed → Task 1. ✓
- Sequencing (plumbing → flag day → gate) → task order. ✓

**Placeholder scan:** none (all commands and file contents are concrete).

**Type consistency:** the `prettier_args` `Mode::Check`/`Mode::Fix` shape matches the existing `static_checks.rs` pattern (verified against `fmt_args`/`leptos_args`). CONTRIBUTING line target verified at `:106`.
