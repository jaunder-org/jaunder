# ADR-0071: Transparent sqlx bridge for string newtypes

- Status: accepted
- Date: 2026-07-20
- Issue: [#438](https://github.com/jaunder-org/jaunder/issues/438)

## Context

String domain newtypes (`Username`, `Slug`, `TokenHash`, `InviteCode`, …)
crossed the `sqlx` boundary as bare strings: every bind was `.bind(x.as_ref())`
(stripping the newtype to `&str`), and every read decoded a `String` and
hand-re-parsed it back into the newtype. That hand re-parse was fallible
boilerplate — e.g. `build_invite_record` carried two `cov:ignore` lines for an
"unreachable" error arm — and a newtype was not a first-class DB column type. It
also meant a value's type-safety was lost at the storage edge and manually
reconstructed, a place for the newtype invariant to silently drift.

ADR-0063 already solved the analogous problem for numeric id newtypes with a
transparent `serde` bridge emitted by the `StrNewtype`/id derives. The DB
boundary is the same shape of problem, one layer down.

Two constraints shape the solution:

- **`common` is target-agnostic (wasm).** The value types live in `common`,
  which the CSR/wasm build compiles and which must never pull `sqlx` (a native,
  non-wasm dependency). `InviteCode` lives in `host` (server-only).
- **`Encode` is a capability, not just a conversion.** Implementing
  `sqlx::Encode` for a type means "this value may be written to the database as
  its raw string." For a few types that is precisely the wrong capability: a
  plaintext `Password` is never stored (only its hash), and a `RawToken` must be
  hashed to a `TokenHash` before it touches a table.

## Decision

The `StrNewtype` derive emits a **transparent, feature-gated sqlx bridge** —
generic `sqlx::Type` + `Encode` + `Decode`, `impl<DB: sqlx::Database>`
delegating to the inner `String` (one impl covers SQLite and Postgres, both
TEXT) — **on by default**, gated behind a `#[cfg(feature = "sqlx")]` that the
wasm build never enables. This mirrors the serde bridge's own shape (ADR-0063):
default-on, `secret` drops it, a secret re-adds it explicitly.

- **Emission rule** (parsed in the derive's `parse_opts`):
  - non-`secret` type → bridge **emitted by default** (no annotation);
  - `secret` → bridge **dropped** (a secret is not storable by default —
    `.bind(password)` will not compile);
  - `secret, sqlx` → **re-adds** the bridge to a secret that genuinely is stored
    (`InviteCode`);
  - `no_sqlx` → the one opt-**out** for a non-`secret` must-not-store type
    (`RawToken`). This is the single place sqlx diverges from serde, justified
    because `Encode` carries a storability semantic `Serialize` does not.
- **`Decode` validates.** For a validating newtype, `Decode` routes the decoded
  string through `FromStr` — keeping the integrity guard the old hand re-parse
  gave (a corrupted/migrated column is rejected, surfacing as
  `sqlx::Error::ColumnDecode`), not silently admitted. For an `infallible`
  newtype it wraps via `From<String>`.
- **Feature isolation.** `common` gains an optional `sqlx` dependency + `sqlx`
  feature; `storage` enables `common/sqlx`. Cargo feature unification is
  per-target-per-graph: `storage` is only ever in the host build, and `web`'s
  CSR/wasm build pulls `common` + `macros` without the feature, so the optional
  dep never compiles for wasm32 — the same isolation that already keeps
  `host`/`storage`/`sqlx` out of the CSR build. A
  `#[cfg(all(target_arch = "wasm32", feature = "sqlx"))] compile_error!` in
  `common` makes any future violation fail loudly.
- **An `xtask` enforcement gate (`sqlx-newtype-bind`)** scans `storage/src` and
  fails on the stringly-bind idioms (`.bind(_.as_ref())`, `.bind(&*_)`,
  `.bind(&**_)`), so a newtype cannot silently be bound as a bare string again.
  It carries a small, substring-matched allowlist for the one legitimate
  `.as_ref()` bind (a typed `Option<PostTitle>::as_ref()`). (#502 retired the
  second entry, `RenderedHtml`, by giving that type a hand-written write-only
  bridge — see Consequences.)

## Consequences

- Every derive-based string newtype is a first-class DB column type:
  `.bind(newtype)` binds directly and `query_as` decodes straight into the
  newtype. New stored string newtypes are DB-ready with no annotation and
  **cannot silently miss the bridge** — the gate enforces it.
- The fallible hand re-parses and their `cov:ignore` debt are retired; several
  record builders became infallible.
- Type-safety is preserved end-to-end across the storage edge; a corrupt column
  is a `ColumnDecode` error at read, not a silently-admitted invalid value.
- `secret` types stay bridge-less by default — the derive is now the single
  place that decides storability, and a plaintext secret cannot be bound to a
  query by accident.
- Commits us to: the `common` optional-`sqlx`-feature seam (kept off for wasm by
  the `compile_error!` guard and the wasm-clippy gate); the `sqlx-newtype-bind`
  gate and its allowlist (down to one entry after #502).
- Rules out per-type hand-written sqlx impls (orphan-rule-bound and duplicative)
  and a storage-side wrapper type (second-class, conversion at every edge) — for
  derive-eligible newtypes. The lone sanctioned exception is `RenderedHtml`
  (#502): a provenance type whose carve-outs (no `FromStr`, and a `Decode` would
  launder an untrusted column into trusted unescaped HTML) rule the derive out,
  so it gets a hand-written **write-only** bridge (`Type`+`Encode`, no
  `Decode`); its column still decodes as `String` and is rebuilt via the gated
  `from_trusted`.
