# ADR-0075: Adopt `strum` for closed string enums; retire the bespoke `StrEnum` derive

- Status: proposed
- Date: 2026-07-22
- Issue: [#607](https://github.com/jaunder-org/jaunder/issues/607)

Supersedes ADR-0074 (`StrEnum` derive — the standard string-enum trailer), which
is still `Status: proposed`.

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
- serde routes through an owned-`String` proxy
  (`#[serde(into = "String", try_from = "String")]` + `From`/`TryFrom` impls)
  where an enum crosses the `serde_qs` **form-transport** boundary (server-fn
  args), NOT a derived enum (de)serializer — a derived enum decoder is not
  guaranteed to decode a bare form value, which is why `StrEnum` hand-rolled its
  `Deserialize` as owned-`String`-through-`FromStr`. The proxy also
  single-sources the wire token in `as_str` (no `serialize_all`/`rename_all`
  duplication). Each migrated enum keeps its `serde_qs`/wire tests green and
  adds a form-transport round-trip.

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
  rather than a stringly `.as_str()` strip. This is a gap-filler, not a `strum`
  duplication — it is explicitly NOT a return to a bespoke proc-macro.
  Introduced with `PostFormat` (#572); reused for the other stored enums in
  #607.
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
- **ADR-0074 is superseded** (it never reached `accepted`). Its `str_enum!` →
  `#[derive(StrEnum)]` history remains valid background; its recommendation to
  route new string enums through `StrEnum` no longer holds — new closed string
  enums use `strum`.
