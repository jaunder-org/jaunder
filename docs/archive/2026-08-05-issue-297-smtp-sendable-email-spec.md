# Issue #297 — an address we accept must be an address we can send

**Issue:** [#297](https://github.com/jaunder-org/jaunder/issues/297) — _email:
validate addresses are SMTP-sendable at the boundary (unify the two parsers)_.
Milestone: Correctness & data integrity. Type: Task.

> **Revised 2026-08-05, twice.** The first version accepted #297's diagnosis and
> proposed narrowing `Email`'s grammar; that was wrong. The second located the
> defect at our transport seam and proposed building lettre's `Mailbox` from
> structured parts; **that was also wrong** — it cannot work, for the reason in
> "Why no local change can fix this". The defect is a bug in lettre, now fixed
> upstream. This version describes that. The wrong turns are kept below because
> each is a plausible-looking dead end the next reader might otherwise re-walk.

## The defect

Mail to a legal, accepted, stored email address fails at send time.

The cause is in **lettre**, not in jaunder:

1. `lettre::message::Mailbox`'s `Display` does not round-trip through its
   `FromStr`. Its RFC 2822 grammar omitted the `domain-literal` production (with
   a note that it "may never be used") and _decoded_ a quoted `local-part`, so
   lettre rendered addresses its own parser then rejected.
2. `Headers` stores every header as a **string** and re-parses it on
   `Headers::get` (`header/mod.rs:72-81`).
3. `MessageBuilder::build` therefore re-parses the `From` header
   (`message/mod.rs:424`) and, absent an explicit envelope, the `To`
   (`address/envelope.rs:170`).

So for an address lettre's own `Address` accepts, building a `Message` failed
with `MissingFrom` / `MissingTo` — errors naming the wrong problem entirely.

Measured, before the fix — all RFC-legal, all accepted by `common::email::Email`
and by `lettre::Address`:

| Address                   | `Mailbox::from_str`  | `Message::builder()…build()` |
| ------------------------- | -------------------- | ---------------------------- |
| `user@[192.0.2.1]`        | ✗ Invalid input      | ✗ MissingFrom / MissingTo    |
| `user@[127.0.0.1]`        | ✗ Invalid input      | ✗                            |
| `user@[IPv6:2001:db8::1]` | ✗ Invalid input      | ✗                            |
| `"has space"@example.com` | ✗ Invalid email user | ✗                            |
| `"has@at"@example.com`    | ✗ Invalid email user | ✗                            |
| `"a<b"@example.com`       | ✗ Invalid email user | ✗                            |

These reach the send path from real user input: the verification dialog, invites
and password resets all mail an address a user typed and we stored.

A note on the second column, since the two failure modes are easy to conflate:
in jaunder's _current_ code these addresses never reach `build()` — the
`.to_string().parse::<Mailbox>()` at `server/src/mailer/smtp.rs:92,102` fails
first and surfaces as `MailError::Send(AddressError: Invalid input)`.
`MissingFrom`/`MissingTo` is what you get once that round-trip is removed, and
is the deeper failure the patch actually fixes.

## Why #297's proposed fix is wrong

#297 proposes tightening input validation "so an accepted address is guaranteed
to be `lettre`-sendable."

1. **The addresses it would reject are legal.** RFC 5321 §4.1.2 gives
   `Local-part = Dot-string / Quoted-string`, and `address-literal` for the
   bracketed-IP domain form. Rejecting them refuses valid mail at the dialog to
   accommodate a parser bug.
2. **There were never two grammars.** `lettre::Address::check_user` calls
   `email_address::EmailAddress::is_valid_local_part` — the same crate
   `common::Email` uses. The divergence was confined to lettre's _display-form_
   parser, a separate hand-rolled nom grammar.
3. **Nothing is asymmetric the other way.** Across the corpus, zero addresses
   lettre accepts that `Email` rejects.

## Why no local change can fix this

The second draft proposed building `lettre::Mailbox` from structured parts
instead of `.to_string().parse()`. **This does not work, and the reason is worth
recording:** how the `Mailbox` is _constructed_ is irrelevant, because
`Headers::set` immediately renders it back to a string and `build()` re-parses
that string. The round-trip is inside lettre, past any API we call. Supplying an
explicit `Envelope` bypasses the check for `To` but not for `From`.

There is no workaround at our seam. Only fixing the parser fixes this.

## The fix

**Upstream, in lettre.** Two productions in
`src/message/mailbox/parsers/rfc2822.rs`:

- `domain = dot-atom / domain-literal / obs-domain` — `domain-literal` added
  (`dtext`, `dcontent`), keeping the brackets, since they are part of the domain
  `Address` stores and validates.
- `local-part = dot-atom / quoted-string / obs-local-part` — the quoted form now
  keeps its quoting (`quoted_string_raw`), because that is what makes the local
  part legal and what `Address` validates. `display-name` still decodes.

Developed test-first in `jaunder-org/lettre`, branch
`fix/mailbox-quoted-local-part-and-domain-literal`: a red commit adding the
tests (including a `Message`-level one that reproduces the misleading
`MissingFrom`), then a green commit with the parser fix. 156 lettre tests pass.
A PR is open upstream.

**Considered and reverted: extending `dtext` to non-ASCII UTF-8.** Review noted
that `Address` accepts `user@[ª]` while `Mailbox` rejects it, and `atext` /
`qcontent` both admit non-ASCII — so `dtext` looked inconsistent. It is not. The
module implements RFC 5336 §3.3, which internationalizes the _local part_ and
domain _names_; it deliberately leaves `address-literal` ASCII, because a
literal is an IP address or a standardized tag, not text. `[ª]` is not a
routable address, and RFC 5321's `dcontent` is printable US-ASCII by definition.
The residual `Address`/`Mailbox` asymmetry is leniency in `email_address`'s
domain validation, and points the safe way: the stricter side is the one
following the RFC. Nothing in jaunder can reach it — `Email` rejects all such
inputs.

**In jaunder,** carried via `[patch.crates-io]` in the workspace `Cargo.toml`
until the fix is released, **pinned by rev** so a build input changes only when
someone changes it here, with a comment recording why, which rev, and when to
drop it.

Two consequences of the patch to state plainly rather than discover later:

- **It carries a minor bump, not just the fix.** The lock moves lettre 0.11.22 →
  0.11.23, so everything else in that release rides along (e.g. multiline SMTP
  error replies). The fork branches from `v0.11.23`, not from the version we
  were on.
- **`cargo-deny` tolerates the git source only by warning.** `deny.toml` sets
  `unknown-git = "warn"` with an empty `allow-git`/`allow-org`, so the gate does
  not fail — but the exception is implicit. `jaunder-org` is added to
  `[sources.allow-org] github` so carrying the patch is a deliberate, visible
  entry whose removal is the signal that the patch went away.

## Local changes

Small, and none of them are the fix:

1. **The corpus guard asserts what the code actually depends on.** The guard
   added earlier checks `Email` → `lettre::Address`. That is not the invariant
   the send path rests on; the send path goes through the _display-form_ parser
   via `Headers`. The guard is widened to assert `Email` → `Mailbox::from_str`
   and that a `Message` builds. **This is the tripwire for the patch**: if
   someone drops the `[patch.crates-io]` entry before upstream releases, the
   build re-resolves to crates.io lettre 0.11.23, which compiles unchanged — so
   nothing fails earlier, and this test is the first thing that does.

   Two honest limits on it: it samples a fixed corpus while the `unreachable!`s
   below assert totality over everything `Email` accepts (a 6,939-input sweep
   found no counterexample, but that sweep is not what the committed test
   proves); and it lives in the same file as the code it protects, so a single
   careless edit can remove both.

2. **The three tests that encode the bug are inverted.** They currently assert
   that a divergent address is _rejected_.
3. **`build_message` stays.** Already extracted; it is what lets the conversion
   be asserted without a live SMTP server, since the Nix checks are
   network-sandboxed.
4. **The `.body()` `unreachable!` comment is corrected, and one of its failure
   modes is made genuinely impossible.** The comment claims `.body()` only fails
   when no transfer-encoding fits; it also fails for `MissingFrom` /
   `MissingTo`, which is what disguised this bug as an encoding concern.

   `MissingTo` is not hypothetical: `build_message` loops over `message.to`, and
   `EmailMessage.to` is a public `Vec<Email>` whose non-emptiness is documented
   but unenforced (`common/src/mailer.rs:27-28`). An empty vec would panic. All
   eight construction sites pass exactly one recipient today, so it is
   unreachable by accident, not by construction. `build_message` therefore
   returns a real `MailError` for an empty `to` **before** building, so the
   remaining arm covers only the genuinely impossible, and the corrected comment
   states its preconditions rather than merely naming more ways to fail.

5. **The conversion arms become `unreachable!`**, justified by (1) — safe
   precisely because the guard trips if the invariant lapses.
6. **`EmailMessage.from`'s doc comment** no longer advertises a display-name
   form the field cannot hold.

## Acceptance criteria

**AC1 — the patch is wired and pinned by rev.** The workspace `Cargo.toml`
carries a `[patch.crates-io]` entry for lettre pinned to an explicit `rev`, with
a comment stating why it exists, that the lockfile must be re-resolved to move
it, and that it should be dropped when the fix is released. `Cargo.lock`
resolves lettre to that rev, and to exactly one entry. `deny.toml` lists
`jaunder-org` under `[sources.allow-org] github`.

**AC2 — divergent addresses now send.** A test drives `build_message` with
recipients `user@[192.0.2.1]`, `"has space"@example.com` and
`"has@at"@example.com` and asserts a `Message` is returned whose envelope
carries them. Each fails before the patch.

**AC3 — the same for the sender and `from`.** `from_config` succeeds for an
`smtp.sender` whose address is a domain-literal or has a quoted local part, and
`build_message` succeeds for an `EmailMessage.from` of the same shapes.

**AC4 — the guard covers the real invariant.** A test asserts, over the corpus,
that every address `Email` accepts round-trips through
`lettre::Mailbox::from_str` and yields a buildable `Message`. Its failure
message says the patch may have been dropped.

**AC5 — the conversions that take an `Email` are `unreachable!`, not error
returns**, and no `cov:ignore` is added anywhere.

**The sender conversion is deliberately excluded and stays fallible.** It takes
a `common::mailbox::Mailbox`, not an `Email`: `DisplayName` admits any character
and `Mailbox`'s `Display` emits it unquoted, so an ordinary
`smtp.sender = "Acme, Inc <noreply@example.com>"` renders as something lettre
cannot read back as an RFC 5322 `phrase`. `BuildMailerError::InvalidSender`
therefore survives, with a test driving it. Making that arm `unreachable!` on
the strength of the `Email` guard would turn an admin's typo into a startup
panic — the root cause is #837, and until it is fixed this reports rather than
crashes.

**AC6 — the bug-encoding tests are inverted.** The three tests at
`server/src/mailer/smtp.rs` asserting rejection now assert success;
`divergent_address()` and its comment go. The two that call `send_email` are
re-pointed at `build_message` — against a dead endpoint `send_email` only ever
errors, so left as-is they would pass for the wrong reason.

**AC7 — the misleading comment is fixed, and `MissingTo` is made impossible.**
`build_message` returns a `MailError` for an empty `EmailMessage.to` before
building, and a test drives that path. `.body()`'s `unreachable!` no longer
claims encoding is its only failure mode; it states the preconditions that make
it unreachable — a `From` that round-trips (AC4's guard) and a non-empty `to`
(this criterion) — rather than merely listing more ways to fail.

**AC8 — human-facing boundaries are unchanged.** `Email` still rejects the
`Name <addr>` form; `common/src/email.rs:74-77` and the malformed-input
rejection tests are untouched and pass.

**AC9 — the stale doc is fixed** (`common/src/mailer.rs:24`).

**AC10 — the separable concern is filed** — done, #837. (A process step, not
code-observable; verified in the tracker rather than in a diff.)

**AC11 — the gate is green.** `cargo xtask validate` passes, including coverage
and all four `{sqlite,postgres}×{chromium,firefox}` e2e combos, **with the git
dependency vendored by the Nix build**. If crane cannot vendor it, that is a
blocking finding to report, not to work around.

## Out of scope

- Changing `Email`'s grammar — not the defect, and narrowing it was the first
  rejected fix.
- Replacing lettre. It validates with the same crate we do; one parser was wrong
  and is now fixed.
- `common::Mailbox`'s wire-encoded display name — #837.
- **EAI / SMTPUTF8.** `Email` accepts internationalized addresses; sending one
  additionally requires the server to advertise `SMTPUTF8`. That is capability
  negotiation, not parsing, and no boundary validation can decide it.
- Backup/restore, which copies `email` columns at table granularity and can
  restore a value no boundary approved.
