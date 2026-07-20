# Spec — issue #438: transparent sqlx bridge for string newtypes

**Issue:** jaunder-org/jaunder#438 — _storage: transparent sqlx bridge for
string newtypes_ **Milestone:** #13 Domain-value type safety (newtypes)
**Relates:** ADR-0063 (transparent-i64 serde bridge — the numeric analogue),
#400 (introduced `InviteCode` on the `.as_ref()` convention, deferring this),
#502 (`RenderedHtml` alignment — a carve-out dependency).

## Summary

String domain newtypes cross the `sqlx` boundary via `.bind(x.as_ref())` (and a
few `.bind(&*x)`) at **~78 sites** across `storage/` — ~57 of them `TokenHash` —
and some rows are decoded as `String` then re-parsed by hand (the fallible
`helpers::build_invite_record`, carrying two `cov:ignore` lines). A newtype is
not a first-class DB column type.

This cycle makes **every derive-based (`#[derive(StrNewtype)]`) string newtype
that is stored** a first-class sqlx column type via a **transparent,
feature-gated sqlx bridge emitted by the derive**, adopts it across all such
types, removes the hand re-parse / `cov:ignore` debt, and installs an **xtask
enforcement gate** so the `.bind(_.as_ref())` idiom cannot silently return for a
newtype.

One genuinely hand-rolled type (`RenderedHtml`, blocked on #502) is **carved
out** to a tracked follow-up; `RawToken` is excluded only because it is never
stored.

## Scope

### In scope

**AC-1 — Mechanism: a derive-emitted, feature-gated sqlx bridge, on by
default.**

The bridge mirrors the **serde** bridge's own shape (ADR-0063): **on by default
for every `#[derive(StrNewtype)]` type, dropped by `secret`, re-added to a
secret with an explicit opt-in.** `Encode` means "this value may be written to
the DB as its raw string," so the default-off exception is exactly the
must-not-store class.

- Add an **optional `sqlx` dependency + `sqlx` feature** to `common` (the
  `uuid`/`chrono` idiom: impls that exist only when a feature the wasm build
  never enables is on).
- The `StrNewtype` derive (`macros/src/str_newtype.rs`) emits
  `#[cfg(feature = "sqlx")]`-gated **generic** impls delegating to `str`/`&str`,
  **by default**:
  - `impl<DB: sqlx::Database> sqlx::Type<DB> for T where str: sqlx::Type<DB>` —
    delegates to `<str as Type<DB>>` (one impl covers SQLite + Postgres; both
    bind TEXT).
  - `Encode`: `encode_by_ref` → `<&str as Encode<DB>>` (zero-copy, borrows
    `self`).
  - `Decode`: `<&str as Decode<DB>>` then convert (see AC-2).
- **Emission rule** (parallels serde), parsed in `parse_opts`:
  - **Non-secret** type → bridge **emitted by default**. Covers all 14
    non-secret derive types — `Username`, `Slug`, `Tag`, `TagLabel`,
    `AudienceName`, `FeedPath`, `Filename`, `ContentHash`, `PostBody`,
    `PostTitle`, `TokenHash`, `Email`, `DisplayName`, `BackupSchedule` — with
    **no annotation**. A future stored string newtype is DB-ready automatically
    — it cannot silently miss the bridge.
  - **`secret`** type → bridge **dropped** by default (a secret must not be
    storable by default: `Password`, `ProfferedPassword`, `ProfferedInviteCode`
    get no `Encode`, so `.bind(password)` will not compile).
  - **`secret, sqlx`** → re-adds the bridge to a secret that genuinely _is_
    stored: **`InviteCode`**.
  - **`no_sqlx`** → the one opt-_out_ for a **non-secret** must-not-store type:
    **`RawToken`** (a raw capability token that must be hashed to `TokenHash`
    before storage; keeping it bridge-less makes `.bind(raw_token)` a compile
    error). This is the single place sqlx diverges from serde — justified
    because `Encode` has a storability semantic `Serialize` does not.
  - Guards: `no_sqlx` is invalid with `secret` (a secret is already bridge-less)
    and with `sqlx`; `sqlx` is only meaningful with `secret`. Reject unknown
    options as today.
- The proc-macro crate itself gains **no** sqlx dependency — it only emits
  tokens; `#[cfg(feature = "sqlx")]` strips the items before name resolution
  when off.
- **wasm stays sqlx-free** (AC-6 verifies): `storage` enables `common/sqlx`;
  `storage` is only ever in the host build; `web`'s CSR/wasm build pulls
  `common`+`macros` and never enables the feature, so the optional dep is never
  compiled for wasm32. `host` declares an on-by-default `sqlx` feature (never
  built for wasm — ADR-0058), so `InviteCode` rides the same path.

**AC-2 — Decode validates through the type's constructor.**

- For **validating** newtypes (`Username`, `Slug`, `Tag`, `TagLabel`,
  `AudienceName`, `FeedPath`, `Filename`, `ContentHash`, `InviteCode`,
  `TokenHash`, `Email`, `DisplayName`, and `BackupSchedule` if validating):
  `Decode` routes the decoded `&str` through `FromStr` — **keeping** the
  integrity guard `build_invite_record` gives today (a corrupted/migrated column
  is rejected, not silently admitted).
- For **`infallible`** newtypes (`PostBody`, `PostTitle`): `Decode` wraps via
  the infallible `From<String>` (no validation to run). The plan classifies each
  stored type as validating vs infallible (drives which `Decode` helper it
  uses).
- The Decode error arm **is coverable** and must be covered by an
  `#[apply(backends)]` test that binds a bad raw string into the column and
  asserts a decode error — so no `cov:ignore` is introduced (retiring debt is
  the point, not relocating it).

**AC-3 — Convert the storage bind/decode sites.** With the bridge on by default
(AC-1), the only _annotations_ required are: `#[str_newtype(secret, sqlx)]` on
`InviteCode` (re-add to the stored secret) and `#[str_newtype(no_sqlx)]` on
`RawToken` (opt the must-not-store type out) — every other stored string newtype
is already covered with no edit. The bulk of the work is then **mechanical
bind-site conversion**: replace `.bind(x.as_ref())` / `.bind(&*x)` /
`.bind(&**x)` / `.map(|x| &**x)` with `.bind(x)` (or the newtype-typed `Option`)
and let `query_as` decode straight into the newtype (removing hand re-parses),
for `Username`, `Slug`, `Tag`/`TagLabel`, `AudienceName`, `FeedPath`,
`Filename`, `ContentHash`, `PostBody`, `PostTitle`, `InviteCode`, `TokenHash`,
`Email`, `DisplayName`, and `BackupSchedule`. Exact bind-site list (incl.
confirming which of `Tag`/`TagLabel` are actually stored, and the `Option`-deref
`&**` forms at e.g. `users.rs:264,429`, `email.rs:140`) is enumerated in the
plan; every backend pair (`sqlite/` + `postgres/` + shared) is updated in
lockstep (backend parity).

**Note on size:** `TokenHash` alone accounts for **~57** of the ~78 policed bind
sites (sessions/email/password-reset token columns across
`storage/src/{sqlite,postgres,}/…`) — the bulk of the mechanical conversion. It
gets the bridge automatically (non-secret), and the enforcement gate (AC-5)
_cannot_ allowlist a derive-based stored newtype, so it is converted, not
deferred.

**AC-4 — Remove the re-parse / `cov:ignore` debt this unblocks.** The
`helpers::build_invite_record` hand re-parse, the `cov:ignore-start` block at
`invites.rs:103-107` (`create_invite`) and the inline `cov:ignore` at
`invites.rs:164` (`list_invites`) are removed; `create_invite`'s raw-`String`
bind from `generate_token()` becomes typed end-to-end.

**AC-5 — Enforcement gate (new xtask step).** A source-scan gate (modelled on
`proffered_secret_check.rs`) over `storage/src` that **fails on the _newtype_
stringly-bind idioms** — `.bind(<expr>.as_ref())`, `.bind(&*<expr>)`,
`.bind(&**<expr>)`, and the `Option` form `.bind(<opt>.map(|x| &**x))` /
`.map(|x| &*x)` (a string newtype exposes its inner via
`AsRef<str>`/`Deref<str>`) — so a newtype can no longer be bound as a bare
string. It deliberately does **not** police `.as_str()`, which is
`String::as_str` on a genuine owned `String` (e.g. `format.as_str()` for
`PostFormat`) — those are not newtypes and stay as-is.

- **Allowlist:** a small, explicit, annotated list for the legitimate remainders
  — the carved-out `rendered_html` binds (until **#502** lands its bridge), plus
  any genuine non-newtype `.as_ref()`/`&*` bind the plan's enumeration surfaces.
  Each entry is tagged with its reason/tracking issue and is expected to shrink
  to zero. The gate must **bite**: a unit test (like the existing gate's tests)
  demonstrating it rejects a reintroduced `.bind(x.as_ref())` on a converted
  newtype.

**AC-6 — wasm build stays sqlx-free.** The CSR/wasm build compiles with no
`sqlx` in its dependency graph (verified by the existing wasm build/clippy
gate + an explicit check that `common`'s `sqlx` feature is off for wasm
targets).

**AC-7 — ADR.** Record the decision as a new ADR (the sqlx analogue of
ADR-0063's serde bridge): the derive-emitted feature-gated bridge, the
**on-by-default, `secret`-excluded, `no_sqlx` opt-out** shape (and why it
mirrors the serde bridge — `Encode` = storability, so must-not-store types are
excluded), the Decode-validates rationale, the
`common`-sqlx-feature/wasm-isolation argument, and the enforcement gate. Drafted
numberless in `docs/adr/drafts/`, promoted at ship.

### Out of scope

- **`RenderedHtml`** (`common/src/render.rs`): the one genuinely **hand-rolled**
  string newtype (no `#[derive(StrNewtype)]`, no `FromStr` constructor), so the
  derive-emitted bridge can't reach it — it won't get the default bridge either.
  Blocked on **#502** (which aligns it to the trailer). Its sqlx bridge is a
  follow-up noted on #502 (the plan's first task); its storage binds stay
  allowlisted (AC-5) meanwhile.
- `Proffered*` wire types (`ProfferedInviteCode`, `ProfferedPassword`) and
  `Password`: `secret` inbound/wire values, never stored — the default-off
  `secret` rule (AC-1) already keeps them bridge-less; no opt-in.
- `RawToken`: **handled in-cycle** via `no_sqlx` (see AC-1/AC-3), not deferred.
- Numeric-id serde bridge (ADR-0063, already shipped); numeric-value newtypes
  (#535–537/#464 cluster).
- Trait-signature changes; any newtype's validation rules.

## Acceptance (roll-up)

- The derive emits feature-gated generic `Type`/`Encode`/`Decode` **on by
  default**; `secret` drops it, `secret, sqlx` re-adds it (`InviteCode`),
  `no_sqlx` opts a non-secret type out (`RawToken`) (AC-1).
- Decode validates through `FromStr` for validating types, infallible-wrap for
  infallible types, with a covered error-arm test — no new `cov:ignore` (AC-2).
- Every derive-based stored string newtype is bound/decoded as itself; no
  `.bind(_.as_ref())` on them remains (AC-3).
- `build_invite_record` re-parse + the three `invites.rs` `cov:ignore` markers
  gone (AC-4).
- The enforcement gate rejects a reintroduced stringly bind and ships green with
  an annotated, shrinking allowlist (AC-5).
- wasm build carries no sqlx (AC-6); ADR drafted (AC-7).
- Dual-backend (`#[apply(backends)]`) coverage for bind + decode + decode-error,
  per backend parity.
- `cargo xtask validate` green (with e2e — this touches the live create/read
  paths).
