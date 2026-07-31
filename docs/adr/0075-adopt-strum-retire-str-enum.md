# ADR-0075: Adopt `strum` for closed string enums; retire the bespoke `StrEnum` derive

- Status: accepted
- Date: 2026-07-22
- Amended: 2026-07-31
- Issue: [#607](https://github.com/jaunder-org/jaunder/issues/607)

Supersedes ADR-0074 (`StrEnum` derive — the standard string-enum trailer), now
`Status: superseded`.

Amended by #746 in five places, marked inline below: the per-enum derive list is
now one `#[text_enum]` attribute
(`docs/adr/drafts/text-enum-closed-string-enum-convention.md`). The core
decision — `strum` owns the token mapping, `Display`, `FromStr`, and variant
enumeration, and `StrEnum` stays deleted — is unchanged.

## Context

ADR-0074 promoted a bespoke `#[derive(StrEnum)]` proc-macro (~300 lines in
`macros/`) as the standard trailer for closed string-backed enums (`Channel`,
`SubscriptionStatus`, `TargetKind`, `AudienceBase`, `PostFormat`,
`RegistrationPolicy`). Its central justification for a hand-rolled macro over
the `strum` crate — which is **already a dependency** and is used by
`BackupMode` (`common/src/backup.rs`) — was that each enum needs a **named,
host-registrable parse error carrying a domain message** (`InvalidPostFormat` →
`host/src/error.rs` `validation_from!` → the client-facing message), and that
the predecessor `macro_rules! str_enum` (and, it was assumed, `strum`) could not
provide one.

That assumption is **false for `strum` 0.28**.
`#[strum(parse_err_ty = …, parse_err_fn = …)]` on `EnumString` yields
`FromStr`/`TryFrom<&str>` with a **custom per-type error**, so the named-error
requirement — the whole reason `StrEnum` existed rather than deriving `strum` —
is satisfiable with the standard crate. A capability review (strum 0.28) found
**nothing `StrEnum` does that `strum` cannot**:

| Capability                                   | `StrEnum`                  | `strum` 0.28                                               |
| -------------------------------------------- | -------------------------- | ---------------------------------------------------------- |
| Wire token (snake_case + per-variant rename) | `as_str`                   | `serialize_all` + `#[strum(serialize = …)]`                |
| `Display`                                    | ✓                          | `strum::Display`                                           |
| `FromStr` / `TryFrom<&str>`                  | ✓                          | `EnumString`                                               |
| Variant enumeration                          | ✗                          | `VariantArray::VARIANTS`                                   |
| Per-variant metadata (UI labels)             | ✗                          | `EnumMessage::get_message`                                 |
| Named per-type parse error + message         | generated                  | `parse_err_ty` + `parse_err_fn` + a `thiserror` unit error |
| serde bridge                                 | generated (`serde` opt-in) | serde derive + `rename_all`                                |

`StrEnum`'s only residual edge is ergonomic bundling (one derive vs. a derive
list + a `thiserror` error + a one-line `parse_err_fn`) and a serde bridge that
single-sources the wire token (strum needs `serialize_all` **and** serde
`rename_all` to agree). These are conveniences, not capabilities — and they do
not justify maintaining a bespoke ~300-line proc-macro that duplicates a
standard crate already in the tree. `strum` also _adds_ what `StrEnum` lacks and
the codebase now needs: variant enumeration and per-variant metadata (surfaced
by #572's shared `FormatToggle`).

## Decision

**Adopt `strum` as the standard for closed string-backed enums, and retire the
`StrEnum` derive.** Each such enum uses the `BackupMode` shape:

- `strum::VariantArray` (enumeration), `strum::EnumString` +
  `#[strum(parse_err_ty = Invalid<Name>, parse_err_fn = …)]` (parse to a named
  error), `strum::Display` / `strum::AsRefStr` / `strum::IntoStaticStr` (string
  forms), `#[strum(serialize_all = "snake_case")]` (wire tokens), and
  `strum::EnumMessage` where per-variant UI metadata (labels) is needed.
- The named error is a `thiserror` unit struct
  (`#[derive(… Error)] #[error("…")]`), matching the repo's existing convention
  (`InvalidBackupSchedule`, `common/backup.rs`).

  > _Amended by #746._ Still a **unit struct**, but no longer `thiserror`:
  > `#[text_enum]` generates it with a hand-written `Display` + `Error`, as
  > `NumNewtype`'s error already did, so an adopting crate needs no dependency
  > beyond the `strum` the injected derives require. The observable shape is
  > unchanged, which is what `host`'s `validation_from!` depends on.

- serde routes through an owned-`String` proxy
  (`#[serde(into = "String", try_from = "String")]` + `From`/`TryFrom` impls)
  where an enum crosses the `serde_qs` **form-transport** boundary (server-fn
  args), NOT a derived enum (de)serializer — a derived enum decoder is not
  guaranteed to decode a bare form value, which is why `StrEnum` hand-rolled its
  `Deserialize` as owned-`String`-through-`FromStr`. The proxy also
  single-sources the wire token in `as_str` (no `serialize_all`/`rename_all`
  duplication). Each migrated enum keeps its `serde_qs`/wire tests green and
  adds a form-transport round-trip.

_Amended by #746._ `BackupMode` is no longer the precedent — `#[text_enum]` is,
and `BackupMode` itself adopted it, gaining the named `InvalidBackupMode` this
paragraph notes it lacked. The paragraph stands as history of what was
established when.

`BackupMode` is precedent for the mechanical strum derives only; it hand-writes
`label()` (not `EnumMessage`) and uses strum's default `ParseError` (not
`parse_err_ty`), and its serde is JSON `site_config` (not `serde_qs` form
transport). So `EnumMessage`-driven labels, `parse_err_ty` named errors, and the
`serde_qs`-safe `String` proxy are established fresh by this migration (verified
on compile + tests), not inherited.

The named, host-registrable error (`Invalid<Name>`, its `host/src/error.rs`
`validation_from!` registration, and client message) is **preserved** — it was
the one thing feared lost, and `parse_err_ty` keeps it. Do **not** introduce a
new bespoke macro to shrink the residual per-enum boilerplate (the `thiserror`
error + one-line `parse_err_fn`); that would recreate the very thing being
retired. Reconsider a small shared helper only if the completed migration shows
the pattern repeated and grating.

> _Amended by #746._ That last sentence has now been exercised twice. First by
> #607 (`parse_error!`, `impl_string_serde_proxy!`); then by #746, which
> replaced both with the `#[text_enum]` attribute. The prohibition's own
> parenthetical scopes it to the residual error boilerplate, and #746 does
> generate that — so this is a real, bounded reversal, argued in the amending
> ADR under "Why this is not a return to `StrEnum`". What is **not** reversed:
> `strum` still owns the token mapping, `Display`, `FromStr`, `VariantArray`,
> and `EnumMessage`, which is what made `StrEnum` worth deleting.

## Consequences

- **`StrEnum` is deleted** once all users are migrated:
  `macros/src/str_enum.rs`, its registration in `macros/src/lib.rs`, and its
  tests. Until then it coexists.
- **A reusable sqlx bridge is introduced** for the storage-backed enums. `strum`
  provides no sqlx integration and `sqlx`'s own `#[derive(Type)]` targets
  _native_ DB enums (not the TEXT tokens these are stored as, dual-backend). So
  a small declarative `impl_text_column_enum!` `macro_rules!` in `common` emits
  the `Type`/`Encode`/`Decode` bridge (delegating to `String`/`&str` via
  `AsRef<str>` + `FromStr`) — one definition, applied per enum, so each stored
  enum binds/decodes as a typed value (like the `StrNewtype` newtypes, #438)
  rather than a stringly `.as_str()` strip. Introduced with `PostFormat` (#572);
  reused for the other stored enums in #607.

  > _Amended by #746._ The final clause of this bullet used to read "This is a
  > gap-filler, not a `strum` duplication — it is explicitly NOT a return to a
  > bespoke proc-macro." **The second half is reversed**: the gap-filler is now
  > proc-macro codegen, reached through `#[text_enum(sqlx)]`, and
  > `impl_text_column_enum!` is deleted. The first half stands and is why: it
  > fills a gap `strum` does not cover (`sqlx`'s own `#[derive(Type)]` targets
  > native DB enums, not dual-backend TEXT tokens), so nothing is duplicated.
  > Only the spelling changed — and with it, the three drifting copies of that
  > bridge collapsed to one.

- **Migration is staged.** `PostFormat` migrates first, in #572 (it surfaced the
  need for enumeration + labels via `FormatToggle`). The remaining `StrEnum`
  enums (`common/src/visibility.rs` ×4, `common/src/media.rs`,
  `common/src/registration.rs` — audit `rg -n 'StrEnum' common/src` for the
  exact set) and the macro deletion are #607. Each migration is
  representation-compatible (identical wire tokens, preserved `Invalid<Name>`
  error) and gated by that enum's existing tests.
- **Accepted minor cost:** the wire token is declared in two attributes
  (`serialize_all` + serde `rename_all`) that must agree — the same duplication
  `BackupMode` already carries. The compile-time duplicate-token check `StrEnum`
  performed is lost; a per-enum round-trip test covers the same ground.

  > _Amended by #746._ This cost is **retired**. `#[text_enum]`'s serde bridge
  > reads the strum token directly, so `rename_all` is gone from every adopter
  > and the wire token is declared once, in `serialize_all`. `BackupMode` was
  > the last carrier.

- **ADR-0074 is superseded** (it never reached `accepted`). Its `str_enum!` →
  `#[derive(StrEnum)]` history remains valid background; its recommendation to
  route new string enums through `StrEnum` no longer holds — new closed string
  enums use `strum`.
