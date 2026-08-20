# #753 ADR Draft Heading Rewrite Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scope ADR draft heading promotion to line 1 and fail malformed draft
headings before mutation.

**Architecture:** Keep the change inside `xtask/src/adr.rs`. Add a small
heading-promotion helper and call it from an all-draft preflight before Pass B
writes numbered ADRs, removes drafts, rewrites references, syncs README, or
stages paths. Tests live in the existing in-file `adr.rs` test module and pin
both the body-token preservation bug and the multi-draft no-mutation failure
path.

**Tech Stack:** Rust `xtask`; in-file unit tests; verification via targeted
`cargo test --manifest-path xtask/Cargo.toml ...` and
`devtool run -- cargo xtask check --no-test`.

**Scope:** In: `xtask/src/adr.rs` heading rewrite behavior and tests; this
approved spec/plan. Out: ADR workflow changes, numbering policy, path-form
reference rewriting, README sync, architecture-view parity, non-`proposed`
status behavior.

**Task list:**

1. Add tests for line-scoped heading promotion and malformed-heading preflight.
2. Implement heading-only promotion and all-draft validation.
3. Run gates and commit.

**Key risks/decisions:**

- Risk: validating inside the existing per-draft Pass B loop can still
  half-promote earlier drafts. Preflight every draft first.
- Risk: asserting only the promoted body can miss staging/removal mutations. The
  malformed-heading test must assert file existence, numbered-file absence, and
  clean `git diff --cached` / `git diff` state.
- Decision: preserve existing ordering after preflight: heading rewrite ->
  `strip_one_level` -> `accept_proposed_status` -> write/remove/stage -> Pass C
  path rewrite.

## Global Constraints

- Follow `CONTRIBUTING.md`: structured edits, focused commit, no lint
  suppression without explicit user approval.
- Spec: body mentions of `ADR-DRAFT` in prose or code spans remain unchanged
  after promotion.
- Spec: if any draft heading is malformed, `adr promote` returns an error before
  writing any numbered ADR, removing any draft, syncing README, rewriting
  references, or staging any path.
- Spec: existing relative-link rewrites, cross-draft reference rewrites, and
  ADR-0088 status promotion remain unchanged.
- Targeted test command:
  `devtool run -- cargo test --manifest-path xtask/Cargo.toml adr::tests::promote -- --nocapture`.
- Gate command: `devtool run -- cargo xtask check --no-test`.

---

### Task 1: Pin promote heading behavior with tests

**Files:**

- Modify: `xtask/src/adr.rs` test module near existing `promote_*` tests.
- Reference:
  `docs/superpowers/specs/2026-08-20-issue-753-adr-draft-heading-rewrite.md`.

**Interfaces:**

- Consumes: existing `promote_repo`, `write`, `run_promote`, and `git_stdout`
  helpers in `xtask/src/adr.rs` tests.
- Produces: failing tests that define the implementation contract for Task 2.

- [x] **Step 1: Add the body-token regression test**

  Add a test named `promote_rewrites_only_the_heading_token`:

  ````rust
  #[test]
  fn promote_rewrites_only_the_heading_token() {
      let tmp = promote_repo("heading-only");
      write(
          &tmp,
          "docs/adr/drafts/d.md",
          "# ADR-DRAFT: D\n\nThe literal `ADR-DRAFT` token is documented here.\n\n```text\nADR-DRAFT\n```\n",
      );

      run_promote(&tmp).unwrap();

      let body = std::fs::read_to_string(tmp.join("docs/adr/0002-d.md")).unwrap();
      assert!(body.starts_with("# ADR-0002: D\n"), "body: {body}");
      assert!(
          body.contains("The literal `ADR-DRAFT` token is documented here."),
          "body: {body}"
      );
      assert!(body.contains("```text\nADR-DRAFT\n```"), "body: {body}");
      assert!(!body.contains("`ADR-0002`"), "body: {body}");
      let _ = std::fs::remove_dir_all(&tmp);
  }
  ````

- [x] **Step 2: Add the multi-draft preflight regression test**

  Add a test named
  `promote_rejects_malformed_heading_before_mutating_any_draft`:

  ```rust
  #[test]
  fn promote_rejects_malformed_heading_before_mutating_any_draft() {
      let tmp = promote_repo("bad-heading");
      let valid = "# ADR-DRAFT: Aaa\n";
      let malformed = "# ADR-DRAFT:   \n";
      write(&tmp, "docs/adr/drafts/aaa.md", valid);
      write(&tmp, "docs/adr/drafts/bbb.md", malformed);

      let err = run_promote(&tmp).unwrap_err();
      let message = format!("{err:#}");

      assert!(
          message.contains("docs/adr/drafts/bbb.md"),
          "error should name malformed draft: {message}"
      );
      assert!(
          message.contains("non-empty title"),
          "error should require a non-empty title: {message}"
      );
      assert_eq!(
          std::fs::read_to_string(tmp.join("docs/adr/drafts/aaa.md")).unwrap(),
          valid
      );
      assert_eq!(
          std::fs::read_to_string(tmp.join("docs/adr/drafts/bbb.md")).unwrap(),
          malformed
      );
      assert!(!tmp.join("docs/adr/0002-aaa.md").exists());
      assert!(!tmp.join("docs/adr/0003-bbb.md").exists());

      let unstaged = git_stdout(&tmp, &["diff", "--name-only"]);
      let staged = git_stdout(&tmp, &["diff", "--cached", "--name-only"]);
      assert!(unstaged.trim().is_empty(), "unstaged diff: {unstaged}");
      assert!(staged.trim().is_empty(), "staged diff: {staged}");
      let _ = std::fs::remove_dir_all(&tmp);
  }
  ```

- [x] **Step 3: Run the targeted tests and verify failure**

  Run:

  ```bash
  devtool run -- cargo test --manifest-path xtask/Cargo.toml adr::tests::promote -- --nocapture
  ```

  Expected: FAIL. `promote_rewrites_only_the_heading_token` fails because body
  mentions become `ADR-0002`.
  `promote_rejects_malformed_heading_before_mutating_any_draft` fails because
  malformed headings are not preflighted.

### Task 2: Implement heading-only promotion and validation preflight

**Files:**

- Modify: `xtask/src/adr.rs` near `run_promote`.
- Test: `xtask/src/adr.rs` in-file tests from Task 1.

**Interfaces:**

- Consumes: tests from Task 1.
- Produces: helper with this contract:

  ```rust
  fn promote_heading(body: &str, number: u32, draft_rel: &str) -> Result<String>
  ```

  Behavior:
  - Requires the body to start with `# ADR-DRAFT: `.
  - Requires a non-empty title on line 1 after that prefix.
  - Rewrites only that line's `ADR-DRAFT` token to `ADR-NNNN`.
  - Leaves the remainder of `body` byte-identical.
  - Returns an error that names `draft_rel`, the required `# ADR-DRAFT: `
    prefix, and the non-empty title requirement when malformed.

- [x] **Step 1: Add the helper**

  Add `promote_heading` near `run_promote`. Implementation shape:

  ```rust
  fn promote_heading(body: &str, number: u32, draft_rel: &str) -> Result<String> {
      let required = "# ADR-DRAFT: ";
      let Some(rest) = body.strip_prefix(required) else {
          bail!("{draft_rel} must start with `{required}` and a non-empty title");
      };
      let title = rest.split_once('\n').map_or(rest, |(line, _)| line);
      if title.trim().is_empty() {
          bail!("{draft_rel} must start with `{required}` and a non-empty title");
      }
      Ok(format!("# ADR-{}: {rest}", pad(number)))
  }
  ```

  This uses `strip_prefix` against the whole body, so only the first line prefix
  is touched. The `rest` includes the title, newline, and entire body unchanged.

- [x] **Step 2: Preflight every draft before mutation**

  After Pass A assigns numbers and before Pass B starts
  writing/removing/staging, read each assigned draft and run `promote_heading`
  once. Store the heading-promoted body for Pass B so the implementation does
  not re-read and revalidate after mutation starts.

  Use a local vector shape equivalent to:

  ```rust
  let mut promoted_bodies: Vec<(String, String)> = Vec::new();
  for p in &assigned {
      let draft_rel = format!("{DRAFTS_DIR}/{}.md", p.slug);
      let body = std::fs::read_to_string(repo.join(&draft_rel))
          .with_context(|| format!("reading {draft_rel}"))?;
      let numbered = promote_heading(&body, p.num, &draft_rel)?;
      promoted_bodies.push((draft_rel, numbered));
  }
  ```

  Then Pass B iterates `assigned.iter_mut().zip(promoted_bodies)` and uses the
  stored `draft_rel` and `numbered` values. Keep the existing `strip_one_level`,
  `accept_proposed_status`, write, remove, and `git::add` order after that
  point.

- [x] **Step 3: Remove the whole-body replacement**

  Delete the existing line:

  ```rust
  let numbered = body.replace("ADR-DRAFT", &format!("ADR-{}", pad(p.num)));
  ```

  There should be no whole-body `ADR-DRAFT` replacement left in `run_promote`.

- [x] **Step 4: Run the targeted tests and verify pass**

  Run:

  ```bash
  devtool run -- cargo test --manifest-path xtask/Cargo.toml adr::tests::promote -- --nocapture
  ```

  Expected: PASS. The new tests and existing promote tests pass.

### Task 3: Gate and commit

**Files:**

- Modify: `xtask/src/adr.rs`.
- Include:
  `docs/superpowers/specs/2026-08-20-issue-753-adr-draft-heading-rewrite.md`.
- Modify:
  `docs/superpowers/plans/2026-08-20-issue-753-adr-draft-heading-rewrite.md`
  checkbox state before commit.

**Interfaces:**

- Consumes: passing Task 2 targeted tests.
- Produces: one checked commit for issue #753.

- [x] **Step 1: Inspect the diff**

  Run:

  ```bash
  git diff --stat
  git diff -- xtask/src/adr.rs docs/superpowers/specs/2026-08-20-issue-753-adr-draft-heading-rewrite.md docs/superpowers/plans/2026-08-20-issue-753-adr-draft-heading-rewrite.md
  ```

  Expected: source changes are limited to `xtask/src/adr.rs`; docs are the
  approved spec and this plan; no ADRs changed.

- [x] **Step 2: Run the fast gate**

  Run:

  ```bash
  devtool run -- cargo xtask check --no-test
  ```

  Expected: PASS — JSON summary has `ok: true` and `exit_code: 0`.

- [x] **Step 3: Commit**

  Before committing, tick all completed task checkboxes in this plan.

  Stage exactly:

  ```bash
  git add xtask/src/adr.rs docs/superpowers/specs/2026-08-20-issue-753-adr-draft-heading-rewrite.md docs/superpowers/plans/2026-08-20-issue-753-adr-draft-heading-rewrite.md
  ```

  Commit:

  ```bash
  git commit -m "fix(xtask): scope ADR draft heading promotion"
  ```

  No `Co-Authored-By` trailer.
