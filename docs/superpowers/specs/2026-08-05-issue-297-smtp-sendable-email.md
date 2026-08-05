# Issue #297 — an address we accept must be an address we can send

**Issue:** [#297](https://github.com/jaunder-org/jaunder/issues/297) — _email:
validate addresses are SMTP-sendable at the boundary (unify the two parsers)_.
Milestone: Correctness & data integrity. Type: Task.

> **This spec deliberately does not implement the fix #297 proposes.** The issue
> is right that a real defect exists and right about its symptom; its diagnosis
> is wrong, and following it would narrow the domain to fit a parser we do not
> have to satisfy. The evidence is in "Why not the issue's fix" below. #297 is
> re-scoped in place, with a comment recording why.

## The defect

Mail to a legal, accepted, stored email address can fail at send time.

`server/src/mailer/smtp.rs` converts our address types into `lettre`'s by
round-tripping them through a **string**: `.to_string().parse()`, at three sites
— the sender (`:37-44`), the message `from` (`:86-92`), and every recipient
(`:96-102`). That `parse()` targets `lettre::message::Mailbox`, whose `FromStr`
parses the RFC 5322 **display form** (`Name <addr>`, a nom grammar at
`lettre-0.11.22/src/message/mailbox/types.rs:112-125`). It chokes on any address
whose own text contains the characters that grammar uses as structure.

Measured, by running a corpus through both parsers:

| Address (all RFC-legal)   | `Mailbox::from_str`  | `Address::from_str` |
| ------------------------- | -------------------- | ------------------- |
| `user@[127.0.0.1]`        | ✗ Invalid input      | ✓                   |
| `user@[192.0.2.1]`        | ✗ Invalid input      | ✓                   |
| `user@[IPv6:2001:db8::1]` | ✗ Invalid input      | ✓                   |
| `"has space"@example.com` | ✗ Invalid email user | ✓                   |
| `"a<b"@example.com`       | ✗ Invalid email user | ✓                   |
| `"has@at"@example.com`    | ✗ Invalid email user | ✓                   |

Every one is accepted by `common::email::Email`, persists happily, and then
fails only when mail is attempted — the "store-but-can't-send correctness gap"
#297 names.

## Why not the issue's fix

#297 proposes tightening input validation "so an accepted address is guaranteed
to be `lettre`-sendable." Three findings say otherwise.

1. **The addresses it would reject are legal.** RFC 5321 §4.1.2 gives
   `Local-part = Dot-string / Quoted-string` and provides `address-literal` for
   the bracketed-IP domain form. Rejecting them refuses valid mail at the dialog
   instead of delivering it.
2. **There are not two grammars.** `lettre::Address::check_user` calls
   `email_address::EmailAddress::is_valid_local_part`
   (`lettre-0.11.22/src/address/types.rs:152-158`) — the same crate
   `common::Email` parses with — and `check_domain` accepts bracketed IP
   literals via that same crate's `is_valid_domain` → `parse_literal_domain`
   (`types.rs:170`; `email_address-0.2.9/src/lib.rs:1063-1068`). The premise
   "different grammars with different strictness" holds only for
   `lettre::Mailbox`'s **display-form string parser**, which is not a grammar we
   have to satisfy at all.
3. **Nothing is asymmetric in the other direction.** Across the corpus there
   were **zero** addresses lettre accepts that `Email` rejects.

The issue's stated payoff — retiring the re-parse error arms — is preserved
here, earned by construction rather than by narrowing what we accept.

## Why the types stay as they are

Considered and rejected: making `lettre`'s type the domain type, or a thin
newtype over `lettre::Address`.

`common` is compiled to **wasm32** (`common/Cargo.toml:33` carries a
`cfg(target_arch = "wasm32")` dependency block). `Email` must live there because
the browser validates the field before submit (`web/src/email/component.rs:17`,
`Field::<Email>::new()`, per ADR-0065). `lettre` is a native SMTP stack and
cannot follow; `common/src/mailer.rs:1-10` already records this as the reason
the concrete senders live in `server::mailer` (ADR-0016). Wrapping
`lettre::Address` in `common` would drag the transport crate into the browser
bundle and cost client-side validation.

The split is correct. Only the conversion between the halves is wrong.

## Where display names actually come from

**`common::mailbox::Mailbox` has exactly one consumer** — `SmtpConfig::sender`
(`storage/src/smtp.rs:72`, parsed at `:174`), from the `smtp.sender` site-config
key, default `Jaunder <noreply@localhost>`.

Every human-facing input takes a bare `Email`, which **already rejects** the
`Name <addr>` form (`common/src/email.rs:36-39`, test at `:74-77`): the
verification dialog, the invite dialog, and `jaunder smtp-test --to`. That
behaviour is correct and this spec does not change it.

## Decisions

1. **The transport converts via structured parts, never via a string.**
   `smtp.rs` stops calling `.to_string().parse::<lettre::Mailbox>()` at all
   three sites and builds
   `lettre::Mailbox::new(name, lettre::Address::from_str(…))` instead.
   `Address`'s `FromStr` is the addr-spec parser (`address/types.rs:200-210`),
   not the display-form one, and accepts everything `Email` does. `name` is
   `None` for `from`/recipients (an `EmailMessage` carries bare `Email`s) and
   `Some(…)` only for the sender.

2. **The sender's display name is unquoted at the seam.** `email_address`'s
   `display_part()` returns the display name **with its RFC quotes intact**, so
   a stored `DisplayName` may be `"Smith, John"` — quote characters included.
   lettre quotes the name itself when rendering
   (`message/mailbox/types.rs:361-374`, `Mailbox::encode` at `:71-85`), so
   passing the stored form through would double-quote it. `smtp.rs` therefore
   strips the surrounding quotes and unescapes `\"` and `\\` before handing the
   name to `lettre::Mailbox::new`.

   **Rejected: refusing quoted display names at the config boundary.** It looked
   tidier, but `"Smith, John" <j@example.com>` is accepted _today_ and lettre
   sends it correctly, and `build_mailer` collapses a config error into
   `NoopMailSender` (`server/src/mailer/mod.rs:48-54`) — so rejecting it would
   silently stop all outgoing mail for that admin. A fix for a send bug must not
   introduce a send outage.

3. **`common::Mailbox` and `DisplayName` are not modified.** Their only
   production coupling to the transport is the string round-trip decision 1
   deletes: `Mailbox::to_string()`'s sole call site is `smtp.rs:40`, and
   `Mailbox::new` has no production caller at all. Changing their parsing,
   `Display`, or character rules would be churn against the defect, not a fix
   for it.

4. **A `build_message` seam makes the conversion observable.** `send_email`
   currently builds and sends in one function (`smtp.rs:85-121`), so no test can
   watch a message being built without a live SMTP server — and the Nix check
   derivations are network-sandboxed. Extracting
   `fn build_message(&self, &EmailMessage) -> Message` lets the conversion be
   asserted directly, with `send_email` reduced to `build_message` + transport.
   This is what makes AC2/AC3 checkable at all.

5. **The conversion arms become `unreachable!`, guarded by a corpus test.**
   Decision 1 makes "anything `Email` accepts, `lettre::Address` accepts" the
   load-bearing invariant. It holds for a structural reason, not a coincidence:
   both call the _same_ `email_address` functions (`parse_local_part` /
   `parse_domain`), `Email` uses `Options::default().without_display_text()`
   against lettre's `Options::default()` — a strictly **narrowing** option — and
   `Cargo.lock` unifies both on a single `email_address 0.2.9`. A corpus test in
   **`server`** (the only crate that sees both types) asserts it over legal and
   adversarial addresses. With that guard the arms are justified
   `unreachable!`s, matching the existing one at `smtp.rs:104-113` and the
   message-required structural exemption in `CONTRIBUTING.md:555-565`.

   Rejected: leaving a live `MailError::Send` arm — undrivable in practice, so
   it becomes exactly the uncovered line #297 set out to remove, needing a
   `cov:ignore`.

6. **`EmailMessage.from`'s doc comment is corrected.** `common/src/mailer.rs:24`
   advertises `"Jaunder <noreply@example.com>"` as the field's form, but the
   field is `Option<Email>` and `Email` rejects that form.

### Separable concern — filed, not fixed here

`common::Mailbox` stores the display name in its **wire-encoded** form (quotes
included), because that is what `display_part()` returns. Decision 2 copes with
it at the one seam that cares. Whether the domain type should hold the literal
label and encode on render is a real modelling question — but note the blast
radius: `DisplayName` is also the **user profile display name**
(`storage/src/users.rs`, `web/src/profile/`, `common/src/site.rs`,
`end2end/tests/profile.spec.ts`), so any character rule added to it is a
user-facing validation change. The filed issue must scope itself to `Mailbox`'s
use of `display_part`, not to `DisplayName` globally.

## Acceptance criteria

**AC1 — no string round-trip remains in the transport.**
`server/src/mailer/smtp.rs` contains no `.to_string().parse()` producing a
`lettre::message::Mailbox`, at any of the three sites.

**AC2 — conversion no longer fails for any address `Email` accepts.** A test
drives `build_message` with recipients `user@[192.0.2.1]`,
`"has space"@example.com` and `"has@at"@example.com` and asserts it returns a
`Message` carrying those recipients. (Today each fails at conversion.) Scope
note: this is about _conversion_, not end-to-end deliverability — see out of
scope on EAI.

**AC3 — the same holds for the sender and for `from`.** `from_config` succeeds
for an `smtp.sender` whose address is a domain-literal or has a quoted local
part; `build_message` succeeds for an `EmailMessage.from` of the same shapes.

**AC4 — a quoted sender display name is neither doubled nor lost.** For
`smtp.sender = "\"Smith, John\" <j@example.com>"`, the built `Message`'s `From`
header renders the name quoted exactly once — `"Smith, John" <j@example.com>` —
and for `Acme Inc <a@example.com>` renders it unquoted. Both configs still build
a working mailer, as they do today.

**AC5 — the subset invariant is asserted.** A test in `server` runs a corpus of
at least the addresses tabulated above, plus ordinary ones, through
`Email::from_str`; for every address `Email` accepts,
`lettre::Address::from_str` accepts it too. The test names the invariant and
fails with the offending address.

**AC6 — the conversion arms are `unreachable!`, not error returns.** No
conversion site in `smtp.rs` returns `MailError`/`BuildMailerError` for a parse
failure, and no `cov:ignore` is added.

**AC7 — the tests that encoded the bug are replaced.** Of the three at
`server/src/mailer/smtp.rs:222-280`:
`from_config_rejects_sender_lettre_cannot_parse` is inverted to assert success;
the two `send_email_rejects_*` tests are re-pointed at `build_message` and
assert success there, since against `mail.example.com:587` with no server
`send_email` can only ever return `Err`. `divergent_address()` and its comment
go — the divergence it names no longer exists.

**AC8 — human-facing boundaries are unchanged.** `Email` still rejects the
`Name <addr>` form; `common/src/email.rs:74-77` (the display-form rejection) is
untouched and still passes, as do the generic-malformed rejection tests at
`server/tests/web/web_email.rs:188-205`, `server/tests/web/web_account.rs:314`
and `server/tests/misc/commands.rs:748-752`.

**AC9 — `send_email` still sends.** `send_email_maps_transport_error`
(`smtp.rs:181-211`) still passes, proving the transport path survives the
`build_message` extraction.

**AC10 — the stale doc is fixed.** `common/src/mailer.rs:24` no longer
advertises a display-name form for a field that cannot hold one.

**AC11 — the separable concern is filed**, scoped to `Mailbox`'s use of
`display_part` and explicitly noting `DisplayName`'s profile-side blast radius.
Linked from #297.

**AC12 — the gate is green.** `cargo xtask validate` passes, including coverage
and all four `{sqlite,postgres}×{chromium,firefox}` e2e combos.

## Out of scope

- Changing `Email`'s grammar. It is not the defect, and narrowing it was the
  rejected fix.
- Replacing `lettre`. It delegates validation to the same crate we use.
- `common::Mailbox`'s wire-encoded display name (filed separately).
- **EAI / SMTPUTF8.** `Email` accepts internationalized addresses
  (`common/src/email.rs:131-139` asserts `user@İ.com` parses), and lettre will
  refuse to send one unless the server advertises `SMTPUTF8`/`8BITMIME`
  (`transport/smtp/client/async_connection.rs:164-182`). That is a genuine
  remaining accepted-but-maybe-unsendable class, but it is a _server capability_
  negotiation, not a parser divergence, and no boundary validation can decide
  it.
- Backup/restore, which copies `email` columns at table granularity and can
  restore a value no boundary approved — a pre-existing gap unrelated to
  sendability.
