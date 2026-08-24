# Issue #827 — typed localStorage key accessors

**Issue:** [#827](https://github.com/jaunder-org/jaunder/issues/827) — _types:
LocalStorageKey registry — the untyped, transposable localStorage get/set_
**Milestone:** #13 Domain-value type safety (newtypes) **Branch:**
`issue-827-local-storage-key-registry`

## Outcome

Runtime code stops passing raw localStorage key strings to `client::storage`.
Jaunder-owned browser storage keys are named through one closed registry at the
`web`/`common` layer, while `client::storage::{get,set,remove}` remains the raw
ADR-0069 browser primitive over `&str`.

## Verified facts the issue got wrong

1. The runtime key set is not two keys. `jaunder_home_redirect` is read by the
   inline pre-paint script in both `web/src/app/render.rs` and `csr/index.html`.
   It currently has no production writer in `client/src` or `web/src`.
2. `jaunder_theme` values are not a closed Rust enum today. The current design
   treats non-empty theme identifiers as opaque CSS selector tokens so
   user-provided stylesheets can add their own `[data-theme="..."]` values.
3. The raw browser primitive's own wasm browser test uses a test-only sentinel
   key. That remains legal because it tests the primitive itself, not product
   storage policy.

## Load-bearing decisions

### D1 — `client::storage` stays raw

ADR-0069 is unchanged: `client` owns browser infrastructure and no Jaunder
domain registry. Its public signatures remain `get(key: &str)`,
`set(key: &str, value: &str)`, and `remove(key: &str)`. The typed door lives
above that primitive.

### D2 — One product registry names Jaunder-owned localStorage keys

The product registry covers the runtime Jaunder-owned keys currently present in
`client/src` and `web/src`:

- the advisory auth marker key (`jaunder_auth`), shared with the pre-paint
  script and marker codec;
- the persisted app-shell theme key (`jaunder_theme`);
- the pre-paint home-redirect preference key (`jaunder_home_redirect`), even
  though it is read only by the inline pre-paint script today.

The registry records the closed key identities. It does not make the raw
`client::storage` primitive private, because that would violate ADR-0069 and
break its own browser-level test.

### D3 — Raw storage calls are legal only inside typed product accessors

Production Rust callers outside those accessors no longer pass a string literal,
`&str`, or re-exported key constant to `client::storage::{get,set,remove}`. The
existing marker accessor shape is the model: callers ask for the domain
operation (read marker, write marker, remove marker; read/write theme), not for
a storage slot.

### D4 — Pre-paint JavaScript is a constrained exception, not a second registry

The inline pre-paint script must keep reading localStorage synchronously before
WASM boots. It cannot call Rust accessors. Its duplicated literal copy in
`csr/index.html` remains guarded by the existing render-shell drift tests, and
its key spellings must agree with the product registry. This is a bridge rule,
not permission to add more raw localStorage literals.

### D5 — Theme values remain opaque non-empty identifiers

This cycle types the key, not the theme value. A non-empty stored theme string
continues to flow into `data-theme` unchanged; empty, absent, or storage-failure
cases continue to select `DEFAULT_THEME`. That preserves the existing CSS
design: shipped selectors define `terminal`, `studio`, and `reader`, while
future user-provided stylesheets may define additional identifiers without a
Rust enum change.

## Acceptance

1. `client::storage::{get,set,remove}` signatures are unchanged and the `client`
   crate still carries no Jaunder domain key type.
2. A single product registry names `jaunder_auth`, `jaunder_theme`, and
   `jaunder_home_redirect`; registry tests prove round-trip string agreement and
   reject an unknown key.
3. Production Rust call sites outside the typed localStorage accessors do not
   pass raw keys to `client::storage::{get,set,remove}`.
4. Auth marker behavior is unchanged: malformed or absent markers remain
   anonymous control flow; storage failures are still swallowed toward the safe
   direction and reported through client telemetry.
5. Theme behavior is unchanged except for typed key access: absent, empty, or
   failed reads select `DEFAULT_THEME`; arbitrary non-empty values are preserved
   as opaque `data-theme` identifiers and written back through the typed
   accessor.
6. The pre-paint script still reads the auth marker and redirect preference
   synchronously before WASM boot, and its literal spellings are checked against
   the registry or an equivalent drift guard.
7. The raw `client::storage` browser lifecycle test remains allowed to use its
   test-only sentinel key.

## Boundaries

- No ADR-0069 amendment and no domain registry inside `client`.
- No finite Rust theme enum in this cycle.
- No theme picker UI, user stylesheet feature, or new writer for
  `jaunder_home_redirect`.
- No change to the auth marker JSON shape, advisory semantics, cookie auth, or
  server authorization.
- No general browser storage abstraction beyond localStorage keys already owned
  by Jaunder runtime code.
