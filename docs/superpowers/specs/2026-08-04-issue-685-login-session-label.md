# Spec — #685: `login` takes `Option<SessionLabel>`

- Issue: [#685](https://github.com/jaunder-org/jaunder/issues/685)
- Milestone: Code quality ratchet
- Governing ADRs: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (string-newtype convention, §4 boundary rule, §5 mandatory adoption),
  [ADR-0065](../../adr/0065-client-side-domain-validation.md) (typed wire args)
- Date: 2026-08-04

## Problem

`web/src/auth/api.rs:40-45` takes the session label as a bare `Option<String>`:

```rust
#[macros::server(skip(password, label))]
pub async fn login(
    username: Username,
    password: ProfferedPassword,
    label: Option<String>,
) -> WebResult<LoginResponse>
```

while `web/src/sessions/api.rs:61` takes the same concept as the newtype that
already exists for it: `create_app_password(label: SessionLabel)`.

`SessionLabel` (`common/src/session_label.rs:28`) is a full ADR-0063 string
newtype — a validating `FromStr` chokepoint plus the generated
`Display`/`AsRef`/`Borrow`/`Deref` trailer and the validating serde and sqlx
bridges. `login` bypasses all of it. Three concrete consequences:

1. **No validation of client input.** A client-supplied label reaches
   `SessionLabel::from_lossy` (`api.rs:91`), which _coerces_ — trims, truncates
   to 255, defaults empty to `"Unknown device"`. `from_lossy`'s own doc
   (`session_label.rs:58-59`) says the opposite is required: "Untrusted _wire_
   input must go through the validating `FromStr` instead (which rejects rather
   than silently truncates)." The login path is the violation that doc warns of.
2. **Two inconsistent bounds.** The User-Agent branch hand-rolls a 200-char
   truncation (`api.rs:83-87`) alongside the newtype's
   `MAX_SESSION_LABEL_CHARS = 255`, and hand-rolls the default literal
   `"Unknown device"` (`api.rs:82`) which is already `SessionLabel::DEFAULT`.
3. **ADR-0063 §5 non-compliance.** Once a domain newtype exists, every argument
   is typed as it; flattening to a primitive requires express owner approval.
   `label` is the last non-secret stringly-typed wire arg on this path. ADR-0065
   carves out an exception only for secrets, and a free-form label is not one.
   (The ADR's wording — that a secret's arg "stays `String`" — is itself stale
   against the code, which passes a typed `ProfferedPassword`; the conclusion
   that a label gets no exemption is unaffected. Realigning ADR-0065's text is
   out of scope here.)

Surfaced during #511 (server-fn tracing). This does **not** change that issue's
tracing gate: `Option<String>` and `Option<SessionLabel>` are both
non-recordable, so `label` stays in the `skip(...)` list either way.

## Context that shapes the design

The login form **does not collect a label**. `web/src/auth/component.rs:38-75`
renders an `ActionForm` with exactly two inputs (`Username`,
`ProfferedPassword`); there is no label field, so a browser submission omits
`label` entirely and the server always takes the User-Agent branch. The
explicit-label branch is reachable only by a non-browser client posting the
form-encoded body directly, plus one integration test
(`server/tests/web/web_auth.rs:385`).

The e2e suite never sends a label, and its seeded-auth path (ADR-0098) bypasses
the `login` server fn entirely, calling `create_session` with a validating
`.parse()`.

## Decisions

### D1 — Keep the parameter; retype it to `Option<SessionLabel>`

The capability (a non-browser client naming its own session) is preserved. The
alternative — deleting the parameter because no first-party client uses it — is
a wire-contract capability removal beyond this issue's scope.

`Option` keeps its "means absence" reading: an omitted label is `None`, never
`Some("")`.

### D2 — The newtype's serde bridge is the sole chokepoint; no custom shim

No hand-written deserializer or pre-parse shim is added. Whatever the
`Option<SessionLabel>` bridge does is the contract — a shim would re-implement
the validation rule, which ADR-0065 forbids ("never a re-implemented rule").

**What that actually yields** (verified against the code, not assumed — see the
falsified-premise note below):

| Wire input          | Today                        | After                          |
| ------------------- | ---------------------------- | ------------------------------ |
| `label` omitted     | `None` → UA branch           | `None` → UA branch (unchanged) |
| `label=` (empty)    | `None` → UA branch           | `None` → UA branch (unchanged) |
| `label=%20%20` (ws) | trimmed to empty → UA branch | **rejected at decode**         |
| `label=my-device`   | used as-is                   | used as-is (unchanged)         |
| over-long (>255)    | silently truncated to 255    | **rejected at decode**         |

The `Option<T>` form layer absorbs a present-but-empty field as `None` _before_
the newtype's deserializer runs, so empty-means-absent comes free from the
framework — no leniency has to be written, and ADR-0065's _Optional fields_
bullet (`:52-59`) sanctions exactly this shape. Precedent, with the behavior
locked by a test: `web/src/backup/api.rs:38`'s
`destination_path: Option<DestinationPath>`, whose
`operator_can_update_backup_settings_clears_via_empty_destination`
(`server/tests/web/web_backup.rs:400-427`) posts `destination_path=` and asserts
200 + `None`.

So the wire-contract tightening is **narrow and deliberate**: whitespace-only
and over-long labels are now rejected instead of silently coerced. Truly-empty
and omitted are unchanged.

> **Falsified premise, recorded.** An earlier draft of this spec asserted that
> `label=` would be rejected at decode, and scoped an AC to rewriting the
> empty-label test to assert rejection. The cold soundness review falsified that
> against the code.
> `docs/archive/2026-07-23-issue-581-destination-path-spec.md:53-69` shows the
> identical error being made and caught on the `DestinationPath` issue. Noted
> here so it is not made a third time.

### D3 — `from_lossy` owns the User-Agent branch's bounding and defaulting

Delete the hand-rolled `.take(200)` truncation and the duplicated
`"Unknown device"` literal. The raw User-Agent (or its absence) is handed
straight to `SessionLabel::from_lossy`, which is precisely ADR-0063 §2's
sanctioned lossy door for a value _derived internally rather than submitted_.

Result: one cap (`MAX_SESSION_LABEL_CHARS` = 255) and one default, each defined
in exactly one place — inside `SessionLabel`. (`SessionLabel::DEFAULT` is a
private const in another crate, so `api.rs` cannot and must not name it; the
default arrives _via_ `from_lossy`.)

A `Some(label)` is used **directly** — it is already a validated `SessionLabel`.
It must not be round-tripped back through `from_lossy` or through `String`.

### D4 — No new ADR; record the no-client-input rationale in a code comment

Every outcome above is an _application_ of ADR-0063/ADR-0065, not a novel
decision, so no ADR draft is written.

ADR-0065 requires a typed wire arg to be client-pre-validated with the same
newtype. Here that requirement is **vacuous**: the login form has no label input
to pre-validate, and no `Field<SessionLabel>` is added to it (adding a label
field to the login form is a UX change this issue did not ask for). A doc
comment on `login` must state this, so a future reader does not read the missing
`Field<>` as an ADR-0065 violation.

### D5 — `SessionLabel`'s import must be ungated

`SessionLabel` is currently imported inside the `#[cfg(feature = "server")]`
block (`api.rs:17-26`). As a wire-arg type it is named in the `#[server]`
signature on **both** the client and server builds, so it moves to the ungated
import block alongside `Username`, `ProfferedPassword`, and `RawToken` — and the
existing comment there explaining why those are ungated must cover it.

## Acceptance criteria

Each is observable — a reviewer can tell delivered from not.

- **AC1** `web/src/auth/api.rs`'s `login` signature reads
  `label: Option<SessionLabel>`. No `Option<String>` label remains on the path.
- **AC2** `SessionLabel` is imported in the ungated import block, not the
  `#[cfg(feature = "server")]` block, and the wasm/CSR build compiles
  (`cargo xtask check` covers both builds).
- **AC3** The `login` body contains no hand-rolled `200`-char truncation and no
  `"Unknown device"` string literal; the User-Agent value reaches
  `SessionLabel::from_lossy` directly. `from_lossy` is called **only** on the
  User-Agent branch — a `Some(label)` is passed through as the already-validated
  `SessionLabel` it is, never re-derived via `from_lossy` or `String`.
- **AC4** A valid explicit label still names the session: posting
  `username=…&password=…&label=my-device` yields a session record whose label is
  `my-device`. (Existing test `login_with_label_creates_session_with_label`
  continues to pass.)
- **AC5** An empty explicit label still means _absent_: posting `…&label=`
  returns `StatusCode::OK` and creates a session labelled from the User-Agent
  (`"Unknown device"` when no UA header is sent). The existing
  `login_with_empty_label_creates_session_without_label` test passes
  **unchanged** — this behavior is preserved, not tightened.
- **AC6** A whitespace-only explicit label (`…&label=%20%20`) is rejected at
  decode: the response is `StatusCode::INTERNAL_SERVER_ERROR` and **no session
  token is returned** (decode fails before the handler body runs, so no session
  row can be minted). New test, mirroring
  `create_app_password_rejects_blank_label`
  (`server/tests/web/web_sessions.rs:188-206`), which posts the same
  whitespace-only shape.
- **AC6b** An over-long explicit label (>`MAX_SESSION_LABEL_CHARS` scalars) is
  likewise rejected at decode — `StatusCode::INTERNAL_SERVER_ERROR`, no session
  token returned — rather than silently truncated. New test.
- **AC7** A long User-Agent is bounded by `MAX_SESSION_LABEL_CHARS`, not 200:
  the existing `login_truncates_long_user_agent` test is replaced so a 250-char
  UA now yields a 250-char label, plus a companion test asserting a 300-char UA
  is truncated to `MAX_SESSION_LABEL_CHARS` (not rejected — the UA is derived,
  so it takes the lossy door).
- **AC8** A request with no `user-agent` header still produces the
  `"Unknown device"` label, and that string appears nowhere in `api.rs` — it
  arrives via `SessionLabel::from_lossy`'s own default.
- **AC9** `login`'s doc comment states why the typed `label` arg has no
  client-side `Field<SessionLabel>` counterpart **on the login form**
  specifically (that form has no label input, so the browser path always takes
  the User-Agent branch). Note a `Field::<SessionLabel>` does exist elsewhere —
  `web/src/sessions/component.rs:77`, the app-password form — so the comment
  must not claim none exists anywhere.
- **AC10** `label` remains in the `#[macros::server(skip(...))]` list — the
  tracing gate from #511 is unchanged.
- **AC11** `cargo xtask validate` is green, including the coverage gate and all
  four `{sqlite,postgres}×{chromium,firefox}` e2e combos. No e2e or seeded-auth
  change is expected, since neither sends a label.

## Out of scope

- Adding a label field to the login form (UX change; D1).
- Removing the `label` parameter (capability removal; D1).
- Any change to `create_app_password` or the app-password form.
- Any change to `SessionLabel` itself or to `MAX_SESSION_LABEL_CHARS`.
