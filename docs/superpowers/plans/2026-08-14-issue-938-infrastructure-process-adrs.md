# Infrastructure and Process Decision Records Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Record and promote four durable #938 architecture decisions, then
project them accurately into the architecture view.

**Architecture:** Draft four focused ADRs in the ignored drafts pen so their
boundaries remain independently reviewable. Project each draft into
`docs/ARCHITECTURE.md` while its citation is still path-based; promotion assigns
collision-free identifiers, rewrites citations, and regenerates the ADR index.

**Tech Stack:** Markdown ADRs; `cargo xtask adr promote`; repository
architecture/documentation gates.

## Review header

**Scope — in:** process configuration; NixOS deployment/package outputs; Cargo
workspace and `host_tests` boundary; Emacs `auth-source` App Password storage;
#76 disposition; ADR/index/architecture projection.

**Scope — out:** any Rust, Nix, Elisp, database, route, browser, or CLI behavior
change; new NixOS options; client-side credential persistence/prompting.

**Tasks:**

1. Resolve the incompatible #76 self-provision proposal.
2. Draft process-configuration ADR.
3. Draft deployment/package-output ADR.
4. Draft workspace/compensating-gate ADR.
5. Draft Emacs `auth-source` ADR.
6. Project drafts, promote them, and verify documentation truth.

**Key risks/decisions:** Numberless drafts are out of git until promotion;
`promote` alone owns ADR numbers and `docs/README.md`. The architecture view
must never represent an accepted target as shipped behavior. Password-file
precedence, Nix module's deliberate secret-injection gap, port-excluded
credential identity, and the #76 conflict are explicit decisions, not incidental
details.

## Global Constraints

- Follow approved specification:
  `docs/superpowers/specs/2026-08-14-issue-938-infrastructure-process-adrs.md`.
- Preserve ADR-0008, ADR-0014, ADR-0028, ADR-0035, ADR-0038, ADR-0047, ADR-0102,
  and ADR-0127 boundaries; do not silently supersede any of them.
- Drafts use `# ADR-DRAFT: <Title>`, `- Status: proposed`, date `2026-08-14`,
  and the #938 URL. Never assign a numeric ADR identifier manually.
- `docs/ARCHITECTURE.md` cites drafts by their `docs/adr/drafts/<slug>.md` path;
  `cargo xtask adr promote` rewrites citations and regenerates `docs/README.md`.
- Before each kept commit: tick its task checkbox, run
  `devtool run -- cargo xtask check`, stage its complete changed tree, and
  commit without a `Co-Authored-By` trailer.
- Documentation-only work has no new application test. Verification is ADR
  formatting, projection/index/link parity, and `cargo xtask validate --no-e2e`.

---

## File structure

| File                                                              | Responsibility                                                                          |
| ----------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| GitHub issues #76 and new focused follow-up                       | Resolve the incompatible credential proposal and record the stale `host_tests` comment. |
| `docs/adr/drafts/process-configuration-cli-contract.md`           | D2 operator-facing process configuration contract.                                      |
| `docs/adr/drafts/declarative-nixos-deployment-package-outputs.md` | D3 NixOS module and output roles.                                                       |
| `docs/adr/drafts/cargo-workspace-execution-boundaries.md`         | D4 root/`xtask`/`tools` split and `host_tests`.                                         |
| `docs/adr/drafts/emacs-auth-source-app-password-storage.md`       | D5 App Password persistence boundary.                                                   |
| `docs/ARCHITECTURE.md`                                            | Materialized projection of all four decisions; remove the five #938 gaps.               |
| `docs/adr/NNNN-*.md`                                              | Numbered accepted records created by promotion.                                         |
| `docs/README.md`                                                  | Generated ADR table, modified only by promotion.                                        |

### Task 1: Reconcile external tracker dependencies

**Files:**

- Modify: GitHub issue #76.
- Create: one focused GitHub issue for `xtask/src/steps/host_tests.rs`.

**Interfaces:**

- Consumes: specification D4–D5 and AC4–AC5.
- Produces: a closed, superseded, or redesigned #76 with an explicit #938 link;
  one open issue to correct the stale `host_tests` comment before the D4 ADR is
  promoted.

- [x] **Step 1: Read issue #76 and confirm the credential conflict**

  Confirm its proposed flow is “login → `create_app_password` → store token” and
  conflicts with the approved `auth-source`-only persistence boundary.

- [x] **Step 2: Close #76 as superseded by #938's decision**

  Add a concise issue comment explaining that App Password persistence remains
  owned by `auth-source`, the client must neither login to mint nor store a
  token, and a future credential UX needs a new decision. Close the issue with
  the tracker’s completed/superseded state reason.

- [x] **Step 3: File the focused `host_tests` documentation defect**

  Create one issue identifying the false `xtask/src/steps/host_tests.rs` claim
  that all `tools/` code is excluded from every Nix check. Its acceptance
  criterion is to preserve the true uncovered-unit-suite rationale while
  acknowledging that `tools/devtool` is built and run by Nix static checks. Link
  #938 and ADR-0028; do not change source in this documentation cycle.

- [x] **Step 4: Verify tracker state**

  Read #76 and the new issue. Expected: #76 is closed with the #938 rationale;
  the new issue is open, focused, and records the exact stale-comment
  correction.

- [x] **Step 5: Commit**

  No repository commit: the durable artifacts for this task are tracker issues.

### Task 2: Draft the process-configuration ADR

**Files:**

- Create: `docs/adr/drafts/process-configuration-cli-contract.md`.

**Interfaces:**

- Consumes: specification D2 and AC2; `server/src/cli.rs`,
  `server/src/commands.rs`, and `storage/src/postgres/open.rs`.
- Produces: a proposed ADR path cited by the architecture projection and later
  promoted to one numbered accepted ADR.

- [x] **Step 1: Copy the ADR template into the drafts pen**

  Create the exact draft path with heading
  `# ADR-DRAFT: Process Configuration and CLI Contract`, status `proposed`, date
  `2026-08-14`, and issue #938.

- [x] **Step 2: Write the decision contract**

  State flag → environment → default precedence only for an applicable argument;
  scope `JAUNDER_VERBOSE` globally, storage/database settings to storage-using
  commands, and bind/environment/runtime-file to `serve`. State runtime-file
  default `<storage-path>/runtime.json`, verbose default false, and all six
  process-shape variables.

  State PostgreSQL password behavior exactly: password-file overrides password
  variable, either overrides a password in `--db`/`JAUNDER_DB`, file content
  trims trailing whitespace, and configured unreadable/non-Unicode values fail
  configuration. State `prod` enables secure cookies and declines initialization
  of a missing database while existing databases migrate; development
  initializes only a missing SQLite database. Reject conflating this process
  surface with ADR-0102 stored `site_config` or embedding secrets in URLs.

- [x] **Step 3: Write consequences and rejected alternatives**

  Commit to compatibility review for changes to flags, variable names, defaults,
  or precedence. Reject treating clap annotations as private implementation,
  silently falling back from malformed secret input, and merging process values
  into stored configuration.

- [x] **Step 4: Verify draft mechanics**

  Inspect the draft. Expected: template sections complete, line 1 exactly uses
  `ADR-DRAFT`, status is one-token `proposed`, and no manually chosen number or
  `docs/README.md` edit exists.

- [x] **Step 5: Commit**

  Drafts are gitignored; do not commit them. Carry this checked draft into Task
  6, where promotion creates the tracked record.

### Task 3: Draft the declarative deployment ADR

**Files:**

- Create: `docs/adr/drafts/declarative-nixos-deployment-package-outputs.md`.

**Interfaces:**

- Consumes: specification D3 and AC3; ADR-0008; `flake.nix`; Task 2's process
  contract.
- Produces: a proposed ADR path cited by the architecture projection and later
  promoted to one numbered accepted ADR.

- [x] **Step 1: Copy the ADR template into the drafts pen**

  Create the exact draft path with heading
  `# ADR-DRAFT: Declarative NixOS Deployment and Package Outputs`, proposed
  status, date, and issue #938.

- [x] **Step 2: Write the module/package contract**

  Define `packages.jaunder` as the deployable single binary. Define
  `nixosModules.jaunder` options exactly: `enable = false`,
  `bind = "127.0.0.1:3000"`, `db = "sqlite:./data/jaunder.db"`, and
  `prod = false`. State the dedicated `jaunder` user/group, home
  `/var/lib/jaunder`, `StateDirectory = "jaunder"`, and
  `WorkingDirectory = "%S/jaunder"`; mapped bind/database/prod environment;
  `jaunder init --db "$JAUNDER_DB" --skip-if-exists` as `preStart`;
  `jaunder serve` as `ExecStart`; restart `on-failure` after `2s`; and the
  ADR-0008 external reverse-proxy boundary.

  State the module intentionally does not expose PostgreSQL password channels;
  operators inject them through their service manager without putting a secret
  in `JAUNDER_DB` or its database URL. Production import guidance explicitly
  selects bind and database values and sets `prod = true`. Define
  `packages.site` as wasm-size-audit-only because the binary embeds assets;
  define both NixOS VMs as development/test only.

- [x] **Step 3: Write consequences and rejected alternatives**

  Reject treating `packages.site` as a runtime bundle, a module-managed TLS
  stack, stored configuration aliases, and undocumented production defaults.
  Record the narrow module interface as a compatibility commitment.

- [x] **Step 4: Verify draft mechanics**

  Inspect heading, status, issue link, and all option defaults. Expected: no ADR
  number, no `docs/README.md` edit, and no claim that the module injects
  database secrets.

- [x] **Step 5: Commit**

  Draft is gitignored; do not commit it before Task 6 promotion.

### Task 4: Draft the workspace and compensating-gate ADR

**Files:**

- Create: `docs/adr/drafts/cargo-workspace-execution-boundaries.md`.

**Interfaces:**

- Consumes: specification D4 and AC4; ADR-0028; `Cargo.toml`,
  `xtask/Cargo.toml`, `tools/Cargo.toml`, `flake.nix`, and
  `xtask/src/steps/host_tests.rs`.
- Produces: a proposed ADR path cited by the architecture projection and later
  promoted to one numbered accepted ADR.

- [x] **Step 1: Copy the ADR template into the drafts pen**

  Create the exact draft path with heading
  `# ADR-DRAFT: Cargo Workspace Execution Boundaries and Compensating Host Tests`,
  proposed status, date, and issue #938.

- [x] **Step 2: Write the workspace and gate contract**

  Define root application workspace ownership, `xtask` as a separate host-only
  workspace excluded from root membership and application derivation source, and
  `tools` as a separate workspace owning `devtool`, `coverage`, and `doctests`.
  State the accurate mixed execution model: `devtool` may run in derivations and
  on the host; workspace ownership is not a claim that all `tools` code runs in
  one place. Preserve ADR-0028's execution litmus.

  Define `host_tests` as required execution of
  `cargo test --manifest-path xtask/Cargo.toml` and
  `cargo test --manifest-path tools/Cargo.toml` on every reached check/validate
  ladder run because application coverage does not run those unit suites. State
  that the steps add test execution, not coverage, and link Task 1's focused
  stale-comment issue as the known current deviation.

- [x] **Step 3: Write consequences and rejected alternatives**

  Reject merging the workspaces merely for convenience, invoking `xtask` inside
  derivations, and treating host tests as an optional fast loop. Keep source
  filtering precise: do not claim all Nix derivations exclude all `tools` code.

- [x] **Step 4: Verify draft mechanics**

  Inspect the two exact host-test commands and source-filter wording. Expected:
  commands/name match `host_tests.rs`; draft remains numberless and untracked.

- [x] **Step 5: Commit**

  Draft is gitignored; do not commit it before Task 6 promotion.

### Task 5: Draft the Emacs App Password storage ADR

**Files:**

- Create: `docs/adr/drafts/emacs-auth-source-app-password-storage.md`.

**Interfaces:**

- Consumes: specification D5 and AC5; ADR-0014, ADR-0035, ADR-0038, ADR-0047;
  `elisp/jaunder-transport.el`; Task 1's #76 disposition.
- Produces: a proposed ADR path cited by the architecture projection and later
  promoted to one numbered accepted ADR.

- [x] **Step 1: Copy the ADR template into the drafts pen**

  Create the exact draft path with heading
  `# ADR-DRAFT: Emacs auth-source App Password Storage`, proposed status, date,
  and issue #938.

- [x] **Step 2: Write the credential boundary**

  State that Emacs retrieves an App Password from `auth-source` using active
  blog URL host plus configured username, excluding port, with `:max 1`. Absence
  is a loud non-interactive configuration error. The client neither prompts,
  writes, nor persists a secret; the request transport reads the retrieved
  secret only to construct Basic authentication. State that current retry
  behavior still retries the broad signalled-error path and link the focused
  #1062 implementation-debt issue.

  Separate the temporary ADR-0035 integration-test fixture from client
  persistence. Preserve ADR-0038 transport and ADR-0047 multi-blog boundaries.
  Cite Task 1's closed/reworked #76 as the disposition of the contradictory
  self-provision proposal.

- [x] **Step 3: Write consequences and rejected alternatives**

  Reject client-managed secret files, prompting/minting a login credential,
  port-qualified secret identity, and silently proceeding without an entry.
  Record that changing credential identity or persistence requires a new ADR.

- [x] **Step 4: Verify draft mechanics**

  Inspect the lookup identity and absence/retry claims against source. Expected:
  no claim of username normalization that the client does not perform; no
  untracked secret persistence path.

- [x] **Step 5: Commit**

  Draft is gitignored; do not commit it before Task 6 promotion.

### Task 6: Project, promote, verify, and commit the accepted decisions

**Files:**

- Modify: `docs/ARCHITECTURE.md:2626-2648` and relevant current-behavior
  sections for process configuration, deployment, workspace/gates, and Emacs
  transport.
- Create: four numbered `docs/adr/NNNN-*.md` files via promotion.
- Modify: `docs/README.md` via promotion only.
- Test: ADR/documentation gates run by `cargo xtask check` and
  `cargo xtask validate --no-e2e`.

**Interfaces:**

- Consumes: four drafts from Tasks 2–5, Task 1's #76 disposition, and Task 1's
  focused `host_tests` comment issue.
- Produces: four accepted numbered ADRs; generated ADR table; architecture view
  that cites each accepted record and removes every #938 `Un-ADR'd reality`
  bullet.

- [x] **Step 1: Refresh, rebase, and inspect drafts before tracked edits**

  Run `git fetch origin`, then rebase onto refreshed `origin/main` while only
  ignored ADR drafts exist. Inspect `docs/adr/drafts/` and continue only when it
  contains exactly this cycle's four named drafts; a stray draft would be
  promoted accidentally and must be resolved before continuing.

- [x] **Step 2: Project all four drafts into `docs/ARCHITECTURE.md`**

  Replace only the five #938 bullets in `Un-ADR'd reality` with the four
  path-based draft citations and concise current-contract prose. Update the
  existing process, deployment, workspace/gate, and Emacs transport sections so
  the view names exact current behavior and decision boundaries. Link Task 1's
  focused `host_tests` issue as the known D4 target/current deviation. Leave
  every unrelated #936/#937 and remaining un-ADR'd entry unchanged. Do not add a
  `CONTEXT.md` term: App Password and Protocol Client already cover the domain.

- [x] **Step 3: Format and promote after the clean rebase**

  Run `devtool run -- prettier -w docs/ARCHITECTURE.md`, then
  `devtool run -- cargo xtask adr promote`. Expected: each named draft is
  numbered and moved to `docs/adr/`, its status becomes `accepted`, path
  citations become numbered citations, and the generated `docs/README.md` table
  is staged. Do not manually edit table rows or ADR numbers.

- [x] **Step 4: Run the pre-commit documentation gate on the staged change**

  Stage the four promoted ADRs, `docs/ARCHITECTURE.md`, and `docs/README.md`;
  run `devtool run -- cargo xtask check`. Expected: ADR format, generated index,
  architecture projection, links, and formatting pass or apply mechanical fixes.
  Restage any gate modifications so the complete checked tree is committed.

- [x] **Step 5: Commit the promoted decision set**

  Tick this task and commit:

  ```bash
  git commit -m "docs(adr): record infrastructure and process decisions (#938)"
  ```

  The pre-commit gate must see the complete staged tree. Do not add a
  `Co-Authored-By` trailer.

- [x] **Step 6: Validate the clean committed branch**

  Run `devtool run -- cargo xtask validate --no-e2e`. Expected: ADR format,
  README/index parity, architecture-view parity, links, and documentation
  validation pass on a clean tree. Inspect the merge-base diff against
  `origin/main...HEAD`: exactly four ADRs, the generated index, and intended
  architecture projection; no production files.

## Self-review

- **Spec coverage:** Tasks 2–5 cover D2–D5 and AC2–AC5; Task 1 resolves the D4
  target/current comment deviation and D5 #76 conflict; Task 6 covers D1, D6,
  and AC1/AC6–AC8.
- **Scope:** no task changes runtime behavior, module API, workspace membership,
  or credential persistence.
- **Identifier safety:** drafts are numberless until Task 6; only promotion owns
  numbers and `docs/README.md`.
- **Placeholder scan:** all paths, commands, decision content, and tracker
  disposition are explicit; no implementation task relies on unspecified tests.

## Execution handoff

Plan complete and saved to
`/home/mdorman/src/jaunder/agent-6/docs/superpowers/plans/2026-08-14-issue-938-infrastructure-process-adrs.md`.

After plan approval, execute with `jaunder-iterate`, ticking each checkbox
before its commit gate and using `jaunder-dispatch` only for independently
verifiable tasks.
