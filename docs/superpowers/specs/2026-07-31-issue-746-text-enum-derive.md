# Spec — `#[text_enum]`: one attribute owns the closed-string-enum convention

- Issue: [#746](https://github.com/jaunder-org/jaunder/issues/746)
- Date: 2026-07-31
- Status: **approved** 2026-07-31
- Soundness-reviewed: 2026-07-31 (cold read, 11 findings + delta read, 6
  findings — all folded)
- **Respecced after review**: the derive became an attribute macro (D1).
- Third review: 2026-07-31 (D1/D1a/D1b/D5/D7/D8/D11/D12 + AC set; 11 findings,
  all folded). D1a's mechanism was **disproved by an executed proc-macro
  reproduction** and is now stated correctly.

## Problem

Two unrelated ceremonies produce the same outcomes.

A **newtype** declares everything in one derive: `#[derive(StrNewtype)]` emits
the trailer, serde, and the sqlx bridge, with the storage decision expressed as
an attribute option.

A **closed string enum** assembles the same result from five separate pieces — a
strum derive list, `#[strum(serialize_all)]`, a
`#[strum(parse_err_ty, parse_err_fn)]` pair, a `parse_error!` invocation below
the type, a serde proxy attribute plus `impl_string_serde_proxy!`, and (if
stored) `impl_text_column_enum!`. Nothing at the type says what it is; the
reader assembles it from six sites.

Underneath, the sqlx bridge exists in **three** copies:
`macros/src/sqlx_bridge.rs` (newtypes), `common/src/db_enum.rs` (enums), and
`common/src/render.rs:276-323` (`RenderedHtml` — the copy `db_enum.rs`'s doc
says the enum macro was lifted from). The enum copy has already drifted: no
`size_hint`, no `#[automatically_derived]`, and a different `where` bound
(`&'r str: Decode` at `db_enum.rs:51` vs `#inner: Decode` at
`sqlx_bridge.rs:63`).

### Population

| enum                                                              | serde                  | sqlx | named parse error |
| ----------------------------------------------------------------- | ---------------------- | ---- | ----------------- |
| `PostFormat` (`render.rs`)                                        | proxy                  | ✓    | ✓                 |
| `MediaSource` (`media.rs`)                                        | proxy                  | ✓    | ✓                 |
| `AudienceBase` (`visibility.rs`)                                  | proxy                  | ✗    | ✓                 |
| `RegistrationPolicy` (`registration.rs`)                          | proxy                  | ✗    | ✓                 |
| `BackupMode` (`backup.rs`)                                        | derived + `rename_all` | ✗    | ✗ (strum default) |
| `Channel` / `SubscriptionStatus` / `TargetKind` (`visibility.rs`) | ✗                      | ✗    | ✓                 |

## Decisions

**D1 — `#[text_enum(…)]` is an attribute macro that owns the whole convention.**

```rust
#[text_enum(
    sqlx,
    error = InvalidPostFormat,
    message = "post format must be \"markdown\", \"org\", or \"html\"",
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default,
         strum::VariantArray, strum::EnumMessage)]
#[strum(serialize_all = "snake_case")]
pub enum PostFormat { … }
```

It **injects** the uniform strum derives (`AsRefStr`, `Display`, `EnumString`,
`IntoStaticStr`) and the `#[strum(parse_err_ty, parse_err_fn)]` pair;
**generates** the named error (a unit struct with hand-written `Display` +
`Error` — **not** `thiserror`, per AC-20), its private parse fn, and
`Serialize`/`Deserialize`; and with `sqlx`, the storage bridge. The author keeps
the non-uniform derives (`VariantArray`, `EnumMessage`, `Default`, `Copy`, …)
and `serialize_all` in their own attributes.

Why an attribute and not a derive: a derive cannot add attributes to its item,
so a derive-based version would still require the author to hand-write
`#[strum(parse_err_ty = …, parse_err_fn = …)]` naming a fn the macro generated.
The attribute form removes that, which is the difference between "a helper" and
"the convention."

Options: `sqlx` (opt-in, storage), `error = <Ident>`, `message = <str literal>`.
`error` and `message` come as a pair — one without the other is a spanned error,
as is any unrecognized option.

**D1a — `#[text_enum]` must be the item's first attribute, because attributes
above it are invisible to it.** Established empirically with a minimal
proc-macro reproduction, not by reasoning about the expansion order:

- A `#[derive(…)]` placed **above** the attribute expands first, is stripped,
  and the attribute macro then runs and injects normally. Derive expansion is
  _additive_ — misplacement does **not** suppress injection. (An earlier draft
  of this spec claimed the opposite; it was wrong.)
- The macro receives only the attributes **below** it. With `#[derive(Debug)]`
  above, the macro's input was exactly
  `[#[derive(Clone)] #[some_helper(below = 1)] pub]` — `Debug` already gone.

So the hazard is the inverse of what it looks like: a **uniform derive written
above `#[text_enum]` is invisible, gets injected a second time, and collides** —
a hard error (`E0119` conflicting implementations, or `E0592` duplicate
definitions). First position is what makes D1's duplicate-suppression (AC-5)
total rather than partial.

The macro cannot detect its own position, so this is enforced by documentation
and by the fact that the failure is a loud, immediate compile error naming the
duplicated trait.

**D1b — injected derives must be path-qualified, making `strum` a named
dependency.** An unqualified `#[derive(AsRefStr)]` fails with "cannot find
derive macro in this scope"; the expansion must emit `::strum::AsRefStr` and
friends. Consequently every crate adopting `#[text_enum]` must depend on `strum`
**under that name**. This matches `StrNewtype`'s existing `::serde`/`::sqlx`
assumption but is stated here because nothing else in the attribute's surface
reveals it. Helper attributes cannot be qualified — `#[strum(…)]` resolves only
because an injected strum derive registers it.

**D2 — `bridge()` takes a `BridgeSpec`.** The three inner types are no longer
one value:

```rust
struct BridgeSpec {
    type_inner,    // the Type impl's delegate and where-bound
    encode_inner,  // the Encode impl's delegate and where-bound
    to_inner,      // expression evaluating to `&#encode_inner`; may use `self`
    decode_inner,  // the Decode impl's delegate and where-bound
    convert,       // tail using local `v: #decode_inner` -> Result<Self, BoxDynError>
}
```

Both `Encode` methods route through one annotated local, so the enum's
`&'static str` coerces to the buffer's `'q` at the `let` (an extending borrow of
an rvalue, so temporary lifetime extension covers it):

```rust
fn encode_by_ref(&self, buf: …) -> … {
    let inner: &#encode_inner = #to_inner;
    <#encode_inner as ::sqlx::Encode<'q, DB>>::encode_by_ref(inner, buf)
}
fn size_hint(&self) -> usize {
    let inner: &#encode_inner = #to_inner;
    <#encode_inner as ::sqlx::Encode<'q, DB>>::size_hint(inner)
}
```

`size_hint` is explicitly part of the mechanism — it consumes the inner-value
expression too (`sqlx_bridge.rs:55-57`), and its absence from `db_enum.rs` is
one of the three drifts being closed.

**D2a — `type_inner` is `String` for enums, deliberately not `str`.** Pairing it
with the new `&'q str`/`&'r str` looks natural and is **wrong**:
`storage/src/posts.rs:847` and `storage/src/media.rs:178` bind the generic
backend with `String: sqlx::Type<DB>`, not `str: sqlx::Type<DB>`, and the enums'
names never appear in those `where` clauses. sqlx's blanket
`impl<T: ?Sized + Type<DB>> Type<DB> for &'_ T` (sqlx-core-0.8.6
`src/types/mod.rs:234`) runs the wrong way to rescue it. `String` is also
today's bound (`db_enum.rs:23`), so **no `where` clause under `storage/`
changes**. The `Encode` side needs none either:
`storage/src/posts.rs:836,1897,1982,2020,2200` and `storage/src/media.rs:173`
already carry `for<'q> &'q str: Encode<'q, DB> + Type<DB>`.

**D3 — every `FromStr`-based decode borrows `&'r str`; the rest keep `String`.**
The waste is _decode an owned `String`, borrow it to parse, drop it_. The rule
is per-conversion:

| bridge user                                                | decode conversion                         | `decode_inner`                  |
| ---------------------------------------------------------- | ----------------------------------------- | ------------------------------- |
| `text_enum` (stored enums)                                 | `FromStr::from_str(v)`                    | `&'r str` — as today            |
| `StrNewtype` default (22 types)                            | `from_str(&v)` — allocated then discarded | `&'r str` — **waste removed**   |
| `StrNewtype` `secret, sqlx` (`SmtpPassword`, `InviteCode`) | same path                                 | `&'r str` — **waste removed**   |
| `StrNewtype` `infallible` (`PostTitle`, `PostBody`)        | `From<String>(v)`                         | `String` — unchanged, see below |
| `IdNewtype` / `NumNewtype`                                 | `i64` / declared inner                    | unchanged                       |
| `SqlxBridge` (`RenderedHtml`, D10)                         | `Ok(Self(v))` — genuine move              | `String` — already optimal      |

**`infallible` stays on `String`, and _not_ because it is always optimal.**
`PostBody::from` is a genuine move (`post_body.rs:27-31`). `PostTitle::from` is
not — `Self(s.trim().to_owned())` (`post_title.rs:19-23`) allocates a second
`String` and drops the decoded one, the exact waste pattern, twice per decode.
The bridge cannot fix it: ADR-0063 makes the hand-written `From<String>` the
type's single construction door, so removing the allocation means changing that
contract. Filed as **#758**, blocked by this issue.

**24 types move** — 22 `default` plus 2 `secret, sqlx`. The six that don't are
`PostTitle`/`PostBody` (infallible), `RawToken` (`no_sqlx`), `Password`
(`secret`), `ProfferedPassword`/`ProfferedInviteCode` (`secret, serde` — no
bridge at all). `InviteCode` lives in **`host/`** (`host/src/invite.rs:26-27`),
not `common/`; that crate has no generic-over-`DB` code and no `Decode` bound,
so it needs no change.

`&'r str: Decode<'r, DB>` holds on both backends (sqlx-sqlite-0.8.6
`src/types/str.rs:27`, sqlx-postgres-0.8.6 `src/types/str.rs:126`) and is
already the live bound at `db_enum.rs:51`. On both backends `String`'s `Decode`
is implemented _through_ `&str`'s, so the accepted-value set is identical — not
a narrowing.

**D4 — enums encode `&'static str`.** `AsRef<str>` ties the borrow to `&self`,
not to `'q`, which is why the current macro must `.to_owned()`. `IntoStaticStr`
gives a real `&'static str`. Verified: strum 0.28 emits both
`From<X> for &'static str` and `From<&'a X> for &'static str`
(`strum_macros-0.28.0/src/macros/strings/as_ref_str.rs:127-146`, headers at 129
and 138), matching on `*x` over unit variants — no `Copy` needed.
`&'q str: Encode<'q, DB>` holds for every `'q` on both backends (sqlx-sqlite
`src/types/str.rs:16`, sqlx-postgres `src/types/str.rs:97`). The `From` pair is
suppressed under `#[strum(const_into_str)]` (as_ref_str.rs:147) — the macro must
reject that option rather than emit a broken bridge.

**D5 — the contract is satisfied by construction, not required of the author.**
Both bridges read the token via `for<'a> &'a Self: Into<&'static str>` and write
it back via `FromStr`. Because D1 _injects_ `IntoStaticStr` and `EnumString`, no
enum can fail the contract by forgetting a derive — the failure mode the
derive-based design had. All eight enums get `AsRefStr` (also injected), which
several rely on: `registration.rs:97`, `media.rs:1769`, `visibility.rs:315`.

**D6 — the name is `text_enum`.** `str_enum` would parallel `StrNewtype`, but
ADR-0074 introduced and ADR-0075 retired a macro by that name for a different
job. "Token" is overloaded (`RawToken`, session tokens, `InviteCode`).

**D7 — this needs a new ADR, not only an amendment to ADR-0075.** The earlier
plan was an in-place amendment, on the reasoning that only the _form_ of a
gap-filler changed. That is no longer true. ADR-0075's Decision states the
standard shape for a closed string enum is a specific list of strum derives
written on the type; D1 replaces that with one attribute that owns the
convention. That is a new architectural decision.

It is **not** a return to `StrEnum`, and the ADR must say why concretely —
including where the two genuinely do overlap, or the argument reads as special
pleading against ADR-0075:99-100, which this also reverses.

The line is **engine vs. periphery**. `StrEnum` generated the wire-token mapping
(`as_str`), `Display`, `FromStr`, and `TryFrom<&str>` (ADR-0074:46-47;
ADR-0075:30-38's capability table) — the engine, and the reason it ran to ~300
lines (ADR-0075:12). `#[text_enum]` generates **none** of those, nor
`VariantArray` nor `EnumMessage`; strum does all of it, and the attribute merely
writes the derives the author would have written by hand.

Two pieces **do** come back: `StrEnum` also generated the named error struct and
an opt-in serde bridge (ADR-0074:47, 52-56), and `#[text_enum]` generates both.
The ADR must concede this plainly. The claim is not "zero overlap" — it is that
the duplicated engine, the whole reason `StrEnum` was deleted, stays deleted.

ADR-0075 also anticipated a step like this: its Decision closes with
_"Reconsider a small shared helper only if the completed migration shows the
pattern repeated and grating"_ (84-85) — a clause already exercised once, by
`parse_error!` (`common/src/strum_enum.rs:6-9` cites it by name and asserts the
helpers are "**not** a return to `StrEnum`"). This is the second application of
that clause, not a defiance of it. The new ADR must note the irony that D8
deletes the very file it cites as precedent, and carry the precedent forward in
its own text rather than by reference.

Mechanics: a numberless draft in `docs/adr/drafts/` per `jaunder-adr`, numbered
at ship by `cargo xtask adr promote`. ADR-0075 gets amendment markers pointing
at it on four passages (AC-24).

**D8 — `parse_error!` retires** (reversing the earlier decision). With the
attribute injecting `#[strum(parse_err_ty, parse_err_fn)]`, the objection that
killed this is gone: the author no longer names a generated ident.
`error = InvalidPostFormat` states the **public** type's name explicitly at the
declaration site — better co-located than today's separate `parse_error!` call,
and still greppable, which matters because `host/src/error.rs:380-387`
(`validation_from!`, re-checked 665-671) registers these by name across a crate
boundary. Only the private parse fn's name is generated, and it appears in no
hand-written source. `common/src/strum_enum.rs` is deleted.

**D9 — `From<Enum> for String` / `TryFrom<String> for Enum` are dropped
intentionally.** `impl_string_serde_proxy!` emits both (`strum_enum.rs:34-46`);
they are public API on `common`. A whole-worktree search found no caller of
either, on any of the **four** types that have them. `FromStr` and `Display`
remain the doors.

**D10 — a bridge-only `#[derive(SqlxBridge)]`, adopted by `RenderedHtml`.**
Emits the three impls via `bridge()` and nothing else: all three inners
`= <the tuple field's type>`, `to_inner = &self.0`, `convert = Ok(Self(v))`. No
options.

Why a separate derive rather than a `#[str_newtype(bridge_only)]` mode:
`RenderedHtml` must never gain an inbound constructor from a raw `String`
(`render.rs:98-104` has `compile_fail` doctests enforcing it). A new arm in
`str_newtype::expand` would share a function with the arms emitting
`FromStr`/`TryFrom`/`From<String>`/`Deserialize`, making "did a constructor
leak?" live on every future edit. A separate derive cannot leak one. `Self(v)`
stays reachable because a derive expands in the type's own module, as the
hand-written impl does today (`render.rs:301-304`).

**Its security rationale is documented in both places:** on the type, the
existing prose (rejected sanitizing decode and why, the revisit condition,
sanitize-vs-`from_trusted`, the #701 pointer, and the `compatible`-delegation
note currently _inside_ the deleted impl at `render.rs:285-291`); on the derive,
a standing warning that its `Decode` is an **inbound door re-establishing no
invariant**.

**D11 — `BackupMode` adopts `#[text_enum]` and gains a named error.** It is the
only enum already deriving `IntoStaticStr` (`backup.rs:30-33`). Adopting removes
`Serialize`/`Deserialize` and `#[serde(rename_all)]`, retiring the dual-token
duplication ADR-0075:110-113 books as an accepted cost. Not stored — no `sqlx`.
Naively it would **regress** error quality: it has no `parse_err_ty`, so derived
serde's ``unknown variant `x`, expected `directory` or `archive` `` would become
strum's `Matching variant not found`. Under D1 `error`/`message` are mandatory,
so it gains `InvalidBackupMode` and a better message than either. It also
silently **gains `Display`**, which it does not have today (`backup.rs:21-34`) —
additive and harmless (nothing formats one, and the inherent `label()` at 46-53
does not collide), but worth naming since D11 otherwise lists only what it
loses. This costs ADR-0075 its `BackupMode`-as-precedent paragraph (72-77) — the
precedent is now `#[text_enum]`.

No test asserts the current derived-serde error text, and all three parse sites
discard the error type (`storage/src/site_config.rs:103`,
`web/src/backup/component.rs:144`, `common/src/backup.rs:226`), so swapping
`FromStr::Err` breaks nothing. `BackupMode` does reach a `#[server]` boundary
via `BackupConfig` (`backup.rs:166`, `web/src/backup/api.rs:48`), which is why
AC-21a pins its JSON bytes.

**D12 — `Channel`, `SubscriptionStatus`, `TargetKind` adopt it too, accepting a
real if modest cost.** They have named parse errors but no serde and no sqlx, so
under D1 they gain `Serialize`/`Deserialize` they do not need. The alternative
is a `no_serde` option carried by three of eight types — not an exception but a
second convention, which defeats rung 2's point. They adopt; the population is
**eight of eight**.

Verified mechanically harmless: none has an existing serde impl to collide with
(`visibility.rs:13-15, 28-39, 54-65`); no containing struct silently becomes
serializable, since Rust requires an explicit derive on the container and the
only one holding any of them, `SubscriptionRecord`, is `#[derive(Clone, Debug)]`
(`storage/src/subscriptions.rs:20-21`); and no gate reacts —
`server_fn_tracing_check`'s `RECORDABLE_TYPES` (lines 62-89) is a
server-fn-argument allowlist keyed on type names, and none of the three enters a
server fn by gaining serde.

**The cost is real and this spec should not pretend otherwise.** An earlier
draft argued "they are already `Display` + `FromStr` + `AsRef<str>`, so nothing
is newly reachable." That conflates read-out traits with
`Serialize`/`Deserialize`, which are exactly what a derive on a _containing_
struct consumes — a different kind of reachability.

The genuine cost: `TargetKind` and `SubscriptionStatus` tokens are a **storage
encoding**, bound as `&'static str` into lookup columns and parsed back
(`storage/src/posts.rs:1844-1846, 1860`). Today the absence of serde is a
compile-time barrier that forces a deliberate decision before those tokens can
become a wire contract. After D12 that barrier is gone, and a future rename of a
lookup token becomes silently wire-breaking with nothing flagging it. Accepted
as the price of one convention; the fallback if it ever bites is `no_serde` on
those two, not an exemption from the attribute.

### Retracted from the filed issue

The issue body claimed this work gives `sqlx-newtype-decode` "a structural
population to key on." **False** — that gate reads `is_i64_family` only (lines
234-254) and never inspects TEXT decodes; `sqlx_newtype_bind_check` has no
allowlist entry touching these enums. Filed separately as **#759** (blocked by
#716), with the false connection recorded so it is not re-invented.

## Migration

All eight enums converge on the same five-line shape. Deleted outright:
`common/src/db_enum.rs`, `common/src/strum_enum.rs`, `render.rs`'s hand-written
bridge, seven `parse_error!` invocations (`visibility.rs:22,48,74,108`,
`media.rs:561`, `registration.rs:40`, `render.rs:59` — `BackupMode` has none
today and gains one under D11), four `impl_string_serde_proxy!` invocations, two
`impl_text_column_enum!` invocations, and every `#[serde(into, try_from)]` /
`#[serde(rename_all)]` on these types — leaving `macros/src/sqlx_bridge.rs` as
the **only** bridge implementation in the repo.

Beyond the enums:

| target                                               | change                                                                 |
| ---------------------------------------------------- | ---------------------------------------------------------------------- |
| `StrNewtype` default + `secret, sqlx` (24 types)     | `decode_inner` → `&'r str`; `convert` drops the `&` (D3)               |
| `StrNewtype` `infallible`, `IdNewtype`, `NumNewtype` | unchanged (D3)                                                         |
| `RenderedHtml`                                       | `+SqlxBridge`; hand-written block at `render.rs:276-323` deleted (D10) |

**Nine prose sites** narrate the retired mechanisms and must be rewritten.
Enum-staled: `render.rs:20-26`, `media.rs:530-537`, `registration.rs:13-14`,
`visibility.rs:7-12`, `backup.rs:17-20`, `storage/src/media.rs:464`. Staled by
D2/D3/D10, each asserting a "three newtype derives, one inner type each" model
that no longer holds: `macros/src/sqlx_bridge.rs:1-13`,
`xtask/src/steps/sqlx_newtype_bind_check.rs:4`,
`xtask/src/steps/rendered_html_from_trusted_check.rs:80-84`.

**Wire and column representation are unchanged for every enum.**
`into = "String"` expands to `serialize_str(&Into::into(self.clone()))`; the
replacement calls `serialize_str` on the token — same bytes.
`try_from = "String"` expands to
`String::deserialize(d).and_then(|v| TryFrom::try_from(v).map_err(de::Error::custom))`;
the replacement is `String::deserialize` → `FromStr` → `de::Error::custom` — the
same owned-`String` path `serde_qs` form transport needs (ADR-0075 §Decision),
same error text. For `BackupMode` the JSON bytes are unchanged and the error
text **improves** (D11).

## Acceptance criteria

1. `macros::text_enum` exists as an attribute macro, exported from
   `macros/src/lib.rs`.
2. Applied to a non-enum, or an enum with any non-unit variant, it produces a
   spanned `compile_error!` naming `text_enum` — unit-tested on a
   `require_enum_shape` helper. Mirrors `require_newtype_shape`
   (`macros/src/lib.rs:237-257`, tests at 269-285) but is **deliberately
   stronger**: those assert only `is_err()`/`contains("compile_error")`.
3. `error` and `message` are mandatory and paired; either alone, an unrecognized
   option, or `#[strum(const_into_str)]` on the item (D4) is a spanned error.
   Unit-tested.
4. The expansion **injects** `strum::AsRefStr`, `Display`, `EnumString`,
   `IntoStaticStr` and
   `#[strum(parse_err_ty = <error>, parse_err_fn = <generated>)]`, and
   **preserves** the author's own derives and `#[strum(serialize_all)]`
   unchanged. Asserted on rendered output. 4a. The injected derives are
   **path-qualified** (`::strum::AsRefStr`, …), and the attribute's docs state
   that an adopting crate must depend on `strum` under that name (D1b). Asserted
   on rendered output.
5. Injection is idempotent-safe for every attribute **below** `#[text_enum]`: if
   the author already derives one of the four there, the expansion emits no
   duplicate. Unit-tested. This guard is necessarily partial — a uniform derive
   written _above_ the attribute is invisible to it (D1a) and will collide with
   `E0119`/`E0592`. That is why first position is required, and the docs must
   say so in those terms.
6. The attribute's doc comment states the first-position rule and names the
   actual failure — a duplicate-impl compile error from an invisible derive
   above it — not the incorrect "injection will not run."
7. Rendered-output test: with `sqlx`, `Type` delegates to `String`, `Encode` to
   `&'q str`, `Decode` to `&'r str`, and no `to_owned` appears. **Compare
   whitespace-normalized** — `TokenStream::to_string()` renders
   `< & 'q str as :: sqlx :: Encode < 'q , DB > >`, so a source-form needle will
   not match. Without `sqlx`, no sqlx impls are emitted.
8. No `where` clause under `storage/` is edited — covering D2a and D3. The only
   decode bound there is `for<'r> UserId: Decode<'r, DB>`
   (`storage/src/users.rs:229`), an `IdNewtype` D3 does not touch. If this
   fails, D2a or D3 is wrong; do **not** fix it by editing `storage/`.
9. Unit tests assert `decode_inner` per D3's table — `&'r str` for `StrNewtype`
   default and `secret, sqlx`; `String`, unchanged, for `infallible`; declared
   inner for `IdNewtype`/`NumNewtype`. A flip applied family-wide fails this,
   which is the point.
10. No `FromStr`-based `convert` allocates: it calls `from_str(v)`, not
    `from_str(&v)`.
11. `StrNewtype`'s `type_inner` and `encode_inner` stay `String` for every kind.
    Without this, a stray `encode_inner` flip passes AC-9 and AC-10 and might
    not trip AC-22 either, since most `storage/` clauses already carry
    `for<'q> &'q str: Encode<'q, DB>`.
12. `IdNewtype`/`NumNewtype` impls are **semantically** unchanged: same
    delegate, bound, and conversion. Their `Encode` bodies change shape (D2
    rewrites it for every family), so byte-identical output is not the criterion
    and must not be asserted.
13. `macros::SqlxBridge` exists and is exported; a rendered-output test asserts
    it emits `Type`/`Encode`/`Decode` and **none** of `FromStr`, `TryFrom`,
    `From`, `Deserialize`, `Serialize`, `Display`, `Deref`, and that for
    `RenderedHtml` all three inners are `String` with `to_inner = &self.0`.
14. `SqlxBridge` on a wrong shape produces a spanned `compile_error!` naming it
    — **unit-tested, not doctested**: `macros/src/lib.rs:259-263` records that
    `compile_fail` doctests are invisible to coverage instrumentation, so an
    untested error path is both a correctness gap and a plausible AC-22 coverage
    failure.
15. `rg 'impl.*sqlx::(Type|Encode|Decode)' common/` matches nothing. It
    currently matches six lines (`db_enum.rs:21,33,49`,
    `render.rs:278,294,309`), all in deleted blocks.
16. `RenderedHtml`'s `compile_fail` doctests (`render.rs:98-104`) still pass.
    They cover two spellings and would not catch a hypothetical
    `FromStr`/`TryFrom` — AC-13 is what proves no constructor is emitted; this
    guards the two documented spellings.
17. The `RenderedHtml` security rationale appears in both places (D10),
    including the `compatible`-delegation note from `render.rs:285-291`.
18. `common/src/db_enum.rs` and `common/src/strum_enum.rs` are gone;
    `rg 'impl_text_column_enum|impl_string_serde_proxy|parse_error!'` matches
    nothing outside archived docs.
19. All **eight** enums use `#[text_enum]` as their first attribute with
    `error`/`message`, carry `sqlx` iff stored (`PostFormat`, `MediaSource`
    only), retain their non-uniform derives, and have no
    `#[serde(into, try_from)]` or `#[serde(rename_all)]`. 19a. The four uniform
    derives (`AsRefStr`, `Display`, `EnumString`, `IntoStaticStr`) are
    **removed** from all eight author-written derive lists. Without this, a
    delivery that leaves them in place satisfies AC-4, AC-5 and AC-19 while
    missing D1's entire point.
20. Each of the eight named error types still exists as a **public unit struct**
    that is
    `Debug + Clone + Copy + PartialEq + Eq + Display + std::error::Error`,
    constructible as a bare unit expression — the observable shape
    `parse_error!` guarantees today (`strum_enum.rs:17-19`, per ADR-0074:47-49).
    It need **not** use `thiserror`: `num_newtype.rs:112-115` hand-writes
    `Display` + `Error` precisely so an adopting crate needs no extra
    dependency, and `text_enum` follows that in-crate precedent (D1b already
    makes `strum` a required dep; `thiserror` should not become one too). Shape,
    not mere existence, is the criterion: `host/src/error.rs:670-671` constructs
    these as bare unit-struct expressions in `check!`, which a generated
    `pub struct Invalid…(());` would break while still satisfying "exists and is
    public". Note only **two** of the eight are registered in `host` —
    `InvalidPostFormat` (386) and `InvalidMediaSource` (387); the other five
    entries in that `validation_from!` list are unrelated types.
21. `BackupMode` deserialization rejects an unknown token with the named
    `InvalidBackupMode` message, not strum's `Matching variant not found` (D11).
    New test asserts the text; today's derived-serde message is the baseline it
    must beat. 21a. `BackupMode`'s serialized JSON bytes are unchanged — this is
    the one type where `rename_all` is swapped for `serialize_all`, and it
    crosses a `#[server]` boundary via `BackupConfig`. A round-trip test pins
    the token for every variant. 21b. `Channel`, `SubscriptionStatus`, and
    `TargetKind` each gain a serde round-trip test (D12). They have none today,
    so without this a broken `Serialize` on any of the three passes every other
    criterion in this set.
22. Every existing test passes **unmodified** — `visibility.rs:335-365`,
    `render.rs:772-792`, `media.rs:1783`, and the form-transport test at
    `web/src/profile/api.rs:121-123`. A test needing edits means representation
    changed and the change is wrong.
23. `cargo xtask validate` green (both backends, all four e2e combos). This is
    the gate for D3's 24-type bound change, not a spot check.
24. A new ADR draft exists in `docs/adr/drafts/` (D7), and ADR-0075 carries
    `- Amended: 2026-07-31` plus `_Amended by #746._` on four passages: the
    Decision sentence (81-83), the `impl_text_column_enum!` Consequences bullet
    (99-100) with its "explicitly NOT a return to a bespoke proc-macro" clause
    **rewritten**, the `BackupMode`-as-precedent paragraph (72-77), and the
    accepted-minor-cost bullet (110-113).
25. Nine prose sites updated per Migration.
26. Issue #746's body is updated to this design.

## Out of scope

- **`PostTitle`'s second decode allocation** — **#758**, blocked by this issue.
- **Extending the sqlx gates to TEXT** — **#759**, blocked by #716.
- **`ProfferedPassword`/`ProfferedInviteCode`/`Password`/`RawToken`** — secrets
  and must-not-store types; no bridge, not string enums.

## Verification

`devtool run -- cargo xtask validate` from the worktree.
