# Plan — issue #297: an accepted address must be a sendable address

**Spec:** `docs/superpowers/specs/2026-08-05-issue-297-smtp-sendable-email.md`.
**Issue:** [#297](https://github.com/jaunder-org/jaunder/issues/297).
**Branch:** `worktree-issue-297-smtp-sendable-email`. **Fork-point tag:**
`wt-base-issue-297`.

> **Revised 2026-08-05** alongside the spec. The original tasks 4-5 described
> building lettre's `Mailbox` from structured parts; that cannot fix the defect
> — the round-trip is inside lettre — and is dropped. The fix is upstream. What
> remains locally is smaller than what has already landed.

**For agentic workers:** drive execution with **`jaunder-iterate`**. Tick
checkboxes in real time.

## Review header

**Goal.** Carry the upstream lettre fix, and retire the local machinery that
existed only to cope with the bug it fixes.

**Already landed on this branch:**

- `1206d2a0` — corpus guard asserting `Email ⊆ lettre::Address` (widened by task
  2 below).
- `eb9a5b91` — `build_message` extracted from `send_email`, so the conversion is
  assertable without a live SMTP server.
- Uncommitted: the `[patch.crates-io]` entry and the re-resolved `Cargo.lock`.
- Upstream: `jaunder-org/lettre` branch
  `fix/mailbox-quoted-local-part-and-domain-literal`, two commits (red then
  green), PR open.
- #837 filed (`common::Mailbox`'s wire-encoded display name).

**Scope — in:** the patch stanza and lockfile; widening the corpus guard to the
invariant the send path actually rests on; inverting the three tests that encode
the bug; retiring the now-dead error arms and `BuildMailerError::InvalidSender`;
two comment corrections.

**Scope — out:** `Email`'s grammar; `common::Mailbox` / `DisplayName` (#837);
replacing lettre; EAI/SMTPUTF8; backup/restore.

**Tasks.** Two commits, not four: the patch and the tests that describe it
cannot be separated (see Key risks), and the guard must precede what relies on
it.

1. **The fix, atomically** — patch stanza + lockfile + `deny.toml` exception;
   invert the three bug-encoding tests; widen the guard; add the
   divergent-address tests. Green on landing — AC1, AC2, AC3, AC4, AC6.
2. **Retire what the bug required** — the dead error arms become `unreachable!`,
   `BuildMailerError::InvalidSender` and `divergent_address()` go, empty-`to` is
   rejected explicitly, both misleading comments corrected — AC5, AC7, AC9.
3. Full gate — AC8, AC11.

**Key risks / decisions.**

- **The patch and the test inversions are one atomic change, and cannot be
  ordered.** The patch alone leaves
  `from_config_rejects_sender_lettre_cannot_parse` red (it asserts lettre
  rejects what lettre now accepts); the inversions alone are red without the
  patch, and worse — three failures instead of one. Neither half can land first.
  `CONTRIBUTING.md:85-92` requires history to stay green commit-by-commit, and
  `:104-105` asks for atomic changes; a dependency fix together with the
  assertions describing it satisfies both. **Do not bypass the gate to split
  them.**
- **The guard lands with the patch, not after it.** Task 2 makes the conversion
  arms `unreachable!`, which is safe only because the widened guard trips if the
  patch is dropped. Guard first, then rely on it — which task 1 delivers.
- **The guard is the patch's tripwire.** Its failure message must say so — the
  likeliest cause of it failing is someone removing `[patch.crates-io]` before
  upstream releases, and the next reader should not have to re-derive that.
- **`nix-coverage` already passed with the git dependency**, so crane vendors it
  without flake changes. Do not "fix" the flake.
- **Only one test currently fails**
  (`from_config_rejects_sender_lettre_cannot_parse`) — that is task 3's red,
  already in hand.
- **Do not touch `SmtpConfigError::InvalidSender`** (`storage/src/smtp.rs:106`):
  a different enum in a different crate, still live. Task 3 removes only
  `BuildMailerError::InvalidSender` (`server/src/mailer/smtp.rs:17`).

## Global constraints

- **Every commit is gated.** `cargo xtask check` green before `git commit`.
  **Always run it in Bash background mode** — a cold Nix rebuild exceeds any
  sensible foreground timeout, and killing it wastes the build.
- `git add -A` then `git commit`. Never `git commit -- <paths>`, never
  `--no-verify`.
- **No `Co-Authored-By` trailer.**
- **No `#[allow]` / `#[expect]` / `cov:ignore`.** `unreachable!` is a
  message-required structural exemption (`CONTRIBUTING.md:555-565`).
- Commit messages reference `(#297)`.

---

## Task 1 — the fix, atomically

**Files:** `Cargo.toml`, `Cargo.lock`, `deny.toml`, `server/src/mailer/smtp.rs`.

**Steps**

- [x] **Step 1.** Patch stanza pinned by rev, with the reasoning and the
      re-resolve instruction recorded; `Cargo.lock` re-resolved to that rev,
      exactly one lettre entry. **Done** — rev `2e7d29b`.
- [ ] **Step 2.** Add `jaunder-org` to `[sources.allow-org] github` in
      `deny.toml`, so the git source is a deliberate entry rather than a
      tolerated warning, and its removal is the signal the patch went away.
- [ ] **Step 3.** Invert the three bug-encoding tests (see task 2's old step 1,
      now here): `from_config_rejects_sender_lettre_cannot_parse` asserts
      success; the two `send_email_rejects_*` are re-pointed at `build_message`,
      assert success, renamed off `rejects_*`, and switched `#[tokio::test]` →
      `#[test]`. Left calling `send_email` they would pass for the wrong reason:
      against `mail.example.com:587` with no server it only ever errors.
- [ ] **Step 4.** Widen `every_email_is_a_lettre_address` to assert what the
      send path actually depends on: for every corpus address, `Email` →
      `lettre::Mailbox::from_str` succeeds **and** a `Message` builds. `Address`
      alone is not the invariant — the header round-trip is.

  ```rust
  // The send path goes through lettre's *display-form* parser, because
  // `Headers` stores each header as a string and re-parses it on `get`. This
  // guard is the tripwire for the `[patch.crates-io]` entry: if the patch is
  // dropped before the fix is released upstream, this fails loudly instead of
  // a user's mail failing silently (#297).
  ```

  The failure message must name the patch as the likely cause.

- [ ] **Step 2.** Add the divergent-address tests: `build_message` with
      recipients `user@[192.0.2.1]`, `"has space"@example.com`,
      `"has@at"@example.com`, asserting the envelope carries them (AC2); the
      same for `EmailMessage.from`, and `from_config` for a divergent
      `smtp.sender` (AC3). These pass immediately with the patch — lock-ins, not
      red/greens; say so rather than manufacturing a red.
- [ ] **Step 6.** `cargo nextest run -p jaunder --lib mailer::smtp` → **all**
      pass, including the inverted ones. Then `cargo xtask check` (background);
      commit `fix(mailer): carry lettre's mailbox parser fix (#297)`. The title
      says _fix_, not `build(deps)`: the patch is the fix.

---

## Task 2 — retire the machinery the bug required

**Files:** `server/src/mailer/smtp.rs`, `common/src/mailer.rs`.

**Steps**

- [ ] **Step 1.** Delete `divergent_address()` and its comment — no address
      diverges now. (Task 1 step 3 already detached its callers.)
- [ ] **Step 2.** Reject an empty `EmailMessage.to` in `build_message` with a
      real `MailError`, before building, and test that path. `to` is a public
      `Vec<Email>` whose non-emptiness is only a doc comment
      (`common/src/mailer.rs:27-28`); every call site passes one recipient
      today, so `MissingTo` is unreachable **by accident**. This makes it
      unreachable by construction — without it, step 4's comment would document
      a live panic.
- [ ] **Step 3.** Convert the three conversion `map_err` arms to
      message-required `unreachable!`, citing the task-1 guard by name. Remove
      `BuildMailerError::InvalidSender`, now unproduced; `BuildMailerError`
      keeps `Transport`, so `from_config` still returns `Result`. **Leave
      `SmtpConfigError::InvalidSender` alone** — different enum, different
      crate, still live.
- [ ] **Step 4.** Correct `.body()`'s `unreachable!` comment. It claims encoding
      is the only failure mode; `.body()` also fails for `MissingFrom` /
      `MissingTo` — the mis-comment that disguised this bug as an encoding
      concern. State the preconditions that make it unreachable (a `From` that
      round-trips, per the guard; a non-empty `to`, per step 2) rather than
      listing more ways to fail.
- [ ] **Step 5.** Fix `common/src/mailer.rs:24` — `EmailMessage.from` is
      `Option<Email>` and cannot hold the advertised `Name <addr>` form. While
      there, check whether `docs/adr/0017`'s description of `MailError::Send` as
      covering "lettre address/SMTP" still holds once address failures are gone;
      touch it up or note explicitly that no change is needed.
- [ ] **Step 6.** `cargo xtask check` (background); commit
      `refactor(mailer): the address conversions cannot fail (#297)`.

---

## Task 3 — full gate

- [ ] **Step 1 (AC8).** Confirm the human-facing boundaries are untouched:
      `git diff wt-base-issue-297..HEAD -- common/src/email.rs     server/tests/web/web_email.rs server/tests/web/web_account.rs     server/tests/misc/commands.rs`
      is empty.
- [ ] **Step 2.** `cargo xtask validate` (background) → green, including the e2e
      combos, **with the git dependency vendored**. `nix-coverage` already
      passed with it, so a vendoring failure here would be new information —
      report it, do not work around it.
- [ ] **Step 3.** Review the branch: `git diff wt-base-issue-297..HEAD`, re-read
      AC1-AC11. Ship notes for `jaunder-ship`: the #297 comment must record that
      the issue's proposed fix was rejected on evidence and why, link the
      upstream PR, and state that the `[patch.crates-io]` entry is temporary.
