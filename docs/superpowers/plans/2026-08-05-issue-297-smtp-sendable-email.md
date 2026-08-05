# Plan — issue #297: an accepted address must be a sendable address

**Spec:** `docs/superpowers/specs/2026-08-05-issue-297-smtp-sendable-email.md`
(the "what" and "why" live there; this plan is the "how"). **Issue:**
[#297](https://github.com/jaunder-org/jaunder/issues/297). **Branch:**
`worktree-issue-297-smtp-sendable-email`. **Fork-point tag:**
`wt-base-issue-297`.

**For agentic workers:** drive execution with **`jaunder-iterate`**, delegating
a task to a subagent via **`jaunder-dispatch`** where useful. Tick checkboxes in
real time.

## Review header

**Goal.** Stop the transport converting addresses through a string. Mail to any
address `Email` accepts — including RFC-legal domain-literals and quoted local
parts — must build and send, instead of failing at conversion.

**Scope — in:** the three conversion sites in `server/src/mailer/smtp.rs`; a
`build_message` seam to make them testable; unquoting the sender's display name;
the corpus test asserting `Email ⊆ lettre::Address`; the three tests that
currently encode the bug; one stale doc comment.

**Scope — out:** `Email`'s grammar (not the defect); `common::Mailbox` and
`DisplayName` (no production coupling once the round-trip goes); replacing
`lettre`; EAI/SMTPUTF8; backup/restore.

**Tasks.**

1. File the separable concern (`Mailbox` stores a wire-encoded display name) as
   an issue — AC11.
2. Corpus test: everything `Email` accepts, `lettre::Address` accepts — AC5.
   Lands **before** anything depends on the invariant.
3. Extract `build_message` from `send_email` — pure refactor. This is what makes
   AC2/AC3 observable at all.
4. Recipients and `from`: construct from parts, **and** make the arms
   `unreachable!` in the same commit — AC1 (two of three sites), AC2, `from`
   half of AC3, two thirds of AC7, part of AC6.
5. Sender: construct from parts, unquoting the display name, same-commit
   `unreachable!`, and retire `BuildMailerError::InvalidSender` — AC1 (third
   site), sender half of AC3, AC4, rest of AC7, rest of AC6.
6. Stale doc comment — AC10.
7. Full gate, plus an explicit check that the human-facing boundaries are
   untouched — AC8, AC12.

**Key risks / decisions.**

- **The `unreachable!` must land with the conversion, not after it.** Those
  `map_err` arms are driven _only_ by the three tests tasks 4-5 retire
  (`divergent_address()`'s own doc says so, `smtp.rs:213-215`). Convert the arm
  and remove its driver in one commit, or `cargo xtask check` fails on uncovered
  lines — the coverage gate is stateless and an uncovered line is a hard failure
  (`CONTRIBUTING.md:543-544`). This is why there is no separate "make it
  unreachable" task.
- **`BuildMailerError::InvalidSender` ≠ `SmtpConfigError::InvalidSender`.**
  Different enums, different crates. Task 5 removes **only** the former
  (`server/src/mailer/smtp.rs:17`, sole producer `:42-44`). The latter
  (`storage/src/smtp.rs:106`) is live — produced at `:175`, asserted at `:305` —
  and must not be touched.
- **Task 5's unquoting is the subtle one.** `display_part()` returns the name
  with quotes intact and inner `\"` / `\\` escaped; lettre re-quotes on render.
  Strip the outer quotes **only** when the value both starts and ends with `"`,
  then unescape exactly those two sequences — no RFC 2047 decoding. Getting it
  wrong shows up as a doubled or mangled `From:` header, which AC4 catches.
- **Do not "fix" `smtp.sender` validation.** The spec considered and rejected
  rejecting quoting-requiring display names: `build_mailer` collapses a config
  error into `NoopMailSender` (`server/src/mailer/mod.rs:48-54`), so a rejection
  is a silent mail outage, not an error message.
- **Two of the three bug-encoding tests cannot simply be inverted.** The
  `send_email_rejects_*` pair runs against `mail.example.com:587`; with no
  server (Nix checks are network-sandboxed) `send_email` only ever returns
  `Err`. They must be re-pointed at `build_message`. Only the `from_config` one
  inverts to `is_ok()`.

## Global constraints

- **Every commit is gated.** Run
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-297-smtp-sendable-email -- cargo xtask check`
  green **before** `git commit` (the pre-commit hook runs the same thing).
  Stage, then commit — never `git commit -- <paths>`. See **`jaunder-commit`**.
- **No `Co-Authored-By` trailer.**
- **No `#[allow]` / `#[expect]` / `cov:ignore`.** The `unreachable!`s are a
  documented structural exemption (`CONTRIBUTING.md:555-565`) and are
  message-required.
- Unit tests here live in `server/src/mailer/smtp.rs`'s `#[cfg(test)]` module.
  These are not storage tests, so the dual-backend template does not apply — but
  do not add a bare `#[tokio::test]` that touches a DB. **`build_message` is
  sync: its tests are `#[test]`, not `#[tokio::test]`.**
- Commit messages: `fix(...)`, `refactor(...)`, `test(...)`, `docs(...)`, each
  referencing `(#297)`.

---

## Task 1 — file the separable concern

**Steps**

- [ ] **Step 1.** Via **`jaunder-issues`**, open an issue: _`common::Mailbox`
      stores the display name in its wire-encoded form_. Body must state: (a)
      `email_address::display_part()` returns the name with quotes intact, so
      `DisplayName` may hold `"Smith, John"`; (b) #297 copes with this by
      unquoting at the transport seam; (c) **the blast radius** — `DisplayName`
      is also the user profile display name (`storage/src/users.rs`,
      `web/src/profile/`, `common/src/site.rs`,
      `end2end/tests/profile.spec.ts`), so a character rule on it is a
      user-facing validation change; (d) scope the issue to `Mailbox`'s use of
      `display_part`, not to `DisplayName` globally. Link it from #297.
- [ ] **Step 2.** No commit (tracker-only).

---

## Task 2 — the invariant, asserted

Lands first: tasks 4 and 5 depend on it holding.

**Files:** `server/src/mailer/smtp.rs` (`#[cfg(test)]`).

**Steps**

- [ ] **Step 1.** Add a corpus test including every address in the spec's table
      (three domain-literals, three quoted local parts), ordinary addresses, and
      the EAI case `user@İ.com`:

  ```rust
  /// The invariant the `unreachable!`s in this module rest on: `Email`'s
  /// grammar is a subset of lettre's `Address`. Both delegate to the same
  /// `email_address` crate, and `Email` parses with the strictly narrower
  /// `without_display_text()` option — but that is a structural argument, and
  /// this test is what makes it a checked one (#297).
  #[test]
  fn every_email_is_a_lettre_address() {
      for raw in CORPUS {
          let Ok(email) = raw.parse::<Email>() else { continue };
          assert!(
              lettre::Address::from_str(&email.to_string()).is_ok(),
              "Email accepted {raw:?} but lettre::Address rejected it — \
               the subset invariant has broken; the unreachable!s in this \
               module are no longer sound",
          );
      }
  }
  ```

- [ ] **Step 2.** `cargo nextest run -p jaunder --lib mailer::smtp` → **PASS**
      (a lock-in guard, not a red/green — say so rather than manufacturing a
      red). **If `user@İ.com` fails**, stop and report: lettre's IDNA fallback
      (`address/types.rs:160-166`) was expected to accept it, and a failure here
      blocks tasks 4-5.
- [ ] **Step 3.** `cargo xtask check`; commit
      `test(mailer): assert Email's grammar is a subset of lettre's Address (#297)`.

---

## Task 3 — extract `build_message`

Pure refactor. No behaviour change, no test changes beyond compilation.

**Files:** `server/src/mailer/smtp.rs` (`:85-121`).

**Interfaces**

```rust
impl LettreMailSender {
    /// Builds the lettre `Message` for an [`EmailMessage`]. Split out from
    /// `send_email` so the address conversion can be asserted without a live
    /// SMTP server (#297).
    fn build_message(&self, message: &EmailMessage) -> Result<Message, MailError> { … }
}
```

`Message` is already imported (`smtp.rs:7`) and is owned, so no lifetime is
needed; `&self` is required for `self.sender.clone()` (`:92`). `send_email`
becomes `self.build_message(message)?` then the existing
`self.mailer.send(email).await.map_err(…)`.

**Steps**

- [ ] **Step 1.** Move `smtp.rs:86-113` verbatim into `build_message` — the
      `from`/recipient conversion, the subject/body build, and the existing
      `unreachable!` (`:104-113`). Leave the conversion logic **unchanged**;
      this task must not alter behaviour.
- [ ] **Step 2.** `cargo nextest run -p jaunder --lib mailer::smtp` → **PASS**,
      all existing tests unmodified, including `send_email_maps_transport_error`
      (AC9).
- [ ] **Step 3.** `cargo xtask check`; commit
      `refactor(mailer): extract build_message from send_email (#297)`.

---

## Task 4 — recipients and `from` convert from parts

One commit: the `unreachable!` lands with the conversion (see Key risks).

**Files:** `server/src/mailer/smtp.rs`.

**Steps**

- [ ] **Step 1 (red).** Add a test driving `build_message` with the three
      divergent recipients, asserting on **`msg.envelope().to()`**
      (`lettre .../message/mod.rs:518`, `address/envelope.rs:128`) — `Message`
      has no `recipients()` accessor:

  ```rust
  #[test]
  fn build_message_accepts_addresses_the_display_parser_rejects() {
      // All three are RFC-legal and `Email` accepts them; before #297 the
      // `.to_string().parse::<Mailbox>()` round-trip rejected each one.
      for raw in [
          "user@[192.0.2.1]",
          "\"has space\"@example.com",
          "\"has@at\"@example.com",
      ] { … }
  }
  ```

  Add a sibling asserting the same for `EmailMessage.from`.
  `cargo nextest run -p jaunder --lib mailer::smtp` → **FAIL** on each address.

- [ ] **Step 2 (green).** Replace both conversions with parts-based construction
      **and** the `unreachable!` together:

  ```rust
  let Ok(address) = lettre::Address::from_str(to_addr) else {
      // `Email` and lettre's `Address` both validate via `email_address`, and
      // `Email` parses with the strictly narrower option set — so an `Email`
      // lettre rejects cannot exist. Guarded by
      // `every_email_is_a_lettre_address` (#297).
      unreachable!("an Email is always a valid lettre Address")
  };
  builder = builder.to(Mailbox::new(None, address));
  ```

  (`Email` derefs to `str`, so no `to_string()` allocation is needed.) Re-run →
  **PASS**.

- [ ] **Step 3.** Re-point `send_email_rejects_from_lettre_cannot_parse`
      (`:242-260`) and `send_email_rejects_recipient_lettre_cannot_parse`
      (`:262-280`) at `build_message`, asserting success. Three things go with
      it: **rename** them (a `rejects_*` test asserting `is_ok()` is a trap —
      e.g. `build_message_accepts_a_from_the_display_parser_rejects`), change
      `#[tokio::test]` → `#[test]`, and stop them calling `divergent_address()`
      (task 5 deletes it). Do **not** leave them calling `send_email`: against a
      dead endpoint that only ever errors, they would pass for the wrong reason.
- [ ] **Step 4.** `cargo xtask check` — **including the coverage step** — then
      commit
      `fix(mailer): build recipient and from addresses from parts (#297)`.

---

## Task 5 — the sender, with its display name unquoted

Same-commit `unreachable!`, as task 4.

**Files:** `server/src/mailer/smtp.rs` (`from_config`, `:36-44`; the error enum
at `:12-21`).

**Steps**

- [ ] **Step 1 (red).** Two tests: (a) `from_config` succeeds for an
      `smtp.sender` whose address is `noreply@[192.0.2.1]`, and for one with a
      quoted local part → **FAIL**; (b) **AC4** — build a `Message` and assert
      on the **wire bytes**, `String::from_utf8(msg.formatted())`, that the
      `From:` line is `"Smith, John" <j@example.com>` for
      `smtp.sender = "\"Smith, John\" <j@example.com>"`, and that
      `Acme Inc     <a@example.com>` renders its name unquoted. Use
      `formatted()`, not `headers().get_raw("From")`: the latter returns the
      unencoded `Mailbox::Display` form, while `formatted()` is what actually
      goes on the wire and is where doubling would appear.
- [ ] **Step 2 (green).** Build the sender from parts:

  ```rust
  // `display_part()` hands back the name with its RFC quoting intact, and
  // lettre re-quotes on render — so strip ours first or the header doubles up.
  let name = config.sender.display_name().map(|n| unquote_display_name(n.as_ref()));
  let Ok(address) = lettre::Address::from_str(config.sender.address()) else {
      unreachable!("an Email is always a valid lettre Address")
  };
  let sender = Mailbox::new(name, address);
  ```

  `unquote_display_name` strips the surrounding quotes **only** when the value
  both starts and ends with `"`, then unescapes `\"` and `\\` — nothing else.
  Unit-test it: unquoted passes through; quoted is stripped; `\"` and `\\` are
  unescaped; a lone `"` at one end is left alone; **`"\"\""` (unquotes to
  empty)** — note lettre's `Display` silently drops an empty name
  (`mailbox/types.rs:90-99`), so record what we do rather than leaving it
  accidental.

- [ ] **Step 3.** Invert `from_config_rejects_sender_lettre_cannot_parse`
      (`:222-240`) to assert success, rename it accordingly, and delete
      `divergent_address()` with its doc comment (`:213-220`) — the divergence
      no longer exists. Confirm no test still references it (task 4 step 3
      already detached the other two).
- [ ] **Step 4.** Remove **`BuildMailerError::InvalidSender`** (`smtp.rs:17`),
      now unproduced. Leave `BuildMailerError::Transport` so `from_config` still
      returns `Result`. **Do not touch `SmtpConfigError::InvalidSender`** in
      `storage/src/smtp.rs` — a different enum in a different crate, still live.
- [ ] **Step 5.** `cargo nextest run -p jaunder --lib mailer::smtp` → **PASS**.
      `cargo xtask check`; commit
      `fix(mailer): build the sender from parts, unquoting its display name (#297)`.

---

## Task 6 — the stale doc comment

- [ ] **Step 1.** `common/src/mailer.rs:24` — `EmailMessage.from` is
      `Option<Email>` and `Email` rejects the `Name <addr>` form, so drop the
      `"Jaunder <noreply@example.com>"` example and say what the field actually
      holds: a bare address, defaulting to `SmtpConfig::sender` when `None`.
- [ ] **Step 2.** `cargo xtask check`; commit
      `docs(mailer): EmailMessage.from holds a bare address (#297)`.

---

## Task 7 — full gate

- [ ] **Step 1 (AC8).** Confirm the human-facing boundaries are untouched:
      `git diff wt-base-issue-297..HEAD -- common/src/email.rs     server/tests/web/web_email.rs server/tests/web/web_account.rs     server/tests/misc/commands.rs`
      is **empty**, and those suites pass. `Email` must still reject the
      `Name <addr>` form — this branch narrows nothing.
- [ ] **Step 2.**
      `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-297-smtp-sendable-email -- cargo xtask validate`
      (Bash background mode — long/cold) → green, including all four
      `{sqlite,postgres}×{chromium,firefox}` e2e combos.
- [ ] **Step 3.** If `docs/coverage/server-fns.json` moved, regenerate **after**
      that run (`regenerate` needs an existing e2e capture) and re-run
      `validate`. No movement expected: no `#[server]` fn changes here.
- [ ] **Step 4.** Review the whole branch: `git diff wt-base-issue-297..HEAD`.
      Re-read the spec's AC1-AC12 against it before handing to `jaunder-ship`.
      The ship step also posts the #297 re-scope comment explaining why the
      issue's proposed fix was not taken.
