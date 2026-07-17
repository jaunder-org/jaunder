# AudienceBase enum Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating individual tasks to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Replace `AudienceSelection.base: String` with a typed
`AudienceBase { Public, Subscribers, Private }` enum so an invalid audience base
is unrepresentable past the DOM edge.

**Architecture:** Add an additive `serde` arm to the `str_enum!` macro
(`common/src/visibility.rs`) whose `Serialize`/`Deserialize` route through the
existing `as_str`/`TryFrom<&str>` — one source of truth for the wire strings.
Retype the field and turn the server-side string match into an exhaustive enum
match. The cross-crate field retype (Task 2) lands atomically because it does
not compile piecewise.

**Tech Stack:** Rust, Leptos (web crate, host+wasm dual-target),
serde/serde_json, `cargo xtask` gate, nextest.

Spec:
[`docs/superpowers/specs/2026-07-17-issue-499-audiencebase-enum.md`](../specs/2026-07-17-issue-499-audiencebase-enum.md).
Refer to the spec for the _what/why_ and acceptance criteria; this plan is the
_how_.

## Global Constraints

- **No wire-shape change.** `AudienceBase` must serialize to exactly `"public"`,
  `"subscribers"`, `"private"` (serde routes through `as_str`). The DOM
  `<option value>` strings and integration-test wire bytes stay byte-identical.
  (Spec AC#1, AC#6)
- **Single source of truth for the strings.** The three literals live only in
  the `str_enum!(serde AudienceBase {…})` definition; no other production site
  holds an audience-base literal. (Spec AC#3)
- **Keep `Default`.** `AudienceSelection` keeps `#[derive(Default)]`;
  `AudienceBase: Default = Private` (safe, non-widening). (Spec §1)
- **No xtask gate, no ADR** — the exhaustive `match` is the enforcement. (Spec
  Non-goals)
- **Coverage:** `common/src/visibility.rs` is gate-measured; the new `Default`,
  the macro `Display`/`as_str`/`TryFrom`, and both serde paths (happy + reject)
  must be exercised by tests, not `cov:ignore`'d. (Spec §5, AC#7)
- **No `Co-Authored-By` trailer.** Run `cargo xtask check` before each commit
  (**jaunder-commit**).

## Review header

**Scope — in:** `common/src/visibility.rs` (macro arm + enum + tests);
`web/src/posts/mod.rs` (field + two mapping fns + unit tests);
`web/src/pages/ui.rs` and `web/src/pages/posts.rs` (DOM edge + comparisons +
constructions); `server/tests/web/web_posts.rs` (two assertions). **Scope —
out:** `AudienceTarget`, storage, wire endpoint shapes, union semantics; no new
files; no separable follow-up issues surfaced.

**Tasks:**

1. `AudienceBase` enum via a new `serde` arm of `str_enum!`,
   `Default = Private`, full coverage tests — self-contained in `common`, green
   commit.
2. Retype `AudienceSelection.base` to `AudienceBase` and update every consumer
   (web mapping + unit tests, DOM edge, integration assertions) — one atomic
   green commit.

**Key risks / decisions:**

- **Task 2 is atomic by necessity.** Retyping the field breaks string-literal
  comparisons across `web` and the `jaunder` integration tests simultaneously;
  the gate builds the whole workspace, so an intermediate commit would leave the
  tree uncompilable. All consumers change in one commit.
- **Coverage of new `common` code** is the most likely gate surprise — Task 1's
  tests target it directly (`default()`, `Display`, serde reject path).
- The DOM edge / view wiring is not host-unit-testable; it is covered by
  `end2end/tests/visibility.spec.ts` (unchanged) and the workspace compile.

---

### Task 1: `AudienceBase` enum + `serde` arm of `str_enum!`

**Files:**

- Modify: `common/src/visibility.rs` (macro at `:7-26`; invocations at `:28-30`;
  tests at `:113-202`)

**Interfaces:**

- Consumes: nothing (leaf).
- Produces: `common::visibility::AudienceBase` —
  `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]` enum with variants
  `Public`, `Subscribers`, `Private`;
  `AudienceBase::as_str(&self) -> &'static str`;
  `TryFrom<&str> for AudienceBase (Error = ())`; `Display`; `serde::Serialize`
  (→ `"public"`/`"subscribers"`/ `"private"`); `serde::Deserialize`;
  `impl Default for AudienceBase` → `Private`.

- [x] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `common/src/visibility.rs`:

```rust
#[test]
fn audience_base_serializes_to_lowercase_literal() {
    assert_eq!(serde_json::to_string(&AudienceBase::Public).unwrap(), "\"public\"");
    assert_eq!(
        serde_json::to_string(&AudienceBase::Subscribers).unwrap(),
        "\"subscribers\""
    );
    assert_eq!(serde_json::to_string(&AudienceBase::Private).unwrap(), "\"private\"");
}

#[test]
fn audience_base_deserializes_from_literal() {
    for v in [AudienceBase::Public, AudienceBase::Subscribers, AudienceBase::Private] {
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<AudienceBase>(&json).unwrap(), v);
    }
}

#[test]
fn audience_base_deserialize_rejects_unknown() {
    assert!(serde_json::from_str::<AudienceBase>("\"bogus\"").is_err());
}

#[test]
fn audience_base_default_is_private() {
    assert_eq!(AudienceBase::default(), AudienceBase::Private);
}
```

Also extend the existing coverage tests to include `AudienceBase`:

- In `display_matches_as_str` (`:129`), add a loop over
  `[AudienceBase::Public, AudienceBase::Subscribers, AudienceBase::Private]`
  asserting `v.to_string() == v.as_str()` and
  `AudienceBase::try_from(v.as_str()) == Ok(v)`.

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p common audience_base` Expected: FAIL — `AudienceBase`
not defined.

- [x] **Step 3: Implement against the tests**

Add a second arm to the `str_enum!` macro (do not touch the existing arm).
Signature contract: `str_enum!(serde $name { $($variant => $s:literal),+ })`
expands the normal arm and additionally emits `Serialize`/`Deserialize`. The
tests pin every branch (serialize per variant, deserialize round-trip,
reject-unknown, default); write the body to satisfy them:

```rust
macro_rules! str_enum {
    // ... existing arm unchanged ...

    (serde $name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        str_enum!($name { $($variant => $s),+ });

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> ::core::result::Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }
        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> ::core::result::Result<Self, D::Error> {
                // Fully-qualify: the `Deserialize` trait is not `use`d in this module,
                // so the bare `String::deserialize` method path would not resolve.
                let s = <::std::string::String as ::serde::Deserialize>::deserialize(d)?;
                Self::try_from(s.as_str())
                    .map_err(|()| <D::Error as ::serde::de::Error>::unknown_variant(&s, &[$($s),+]))
            }
        }
    };
}
```

Add the invocation after the existing three (`:30`):

```rust
str_enum!(serde AudienceBase { Public => "public", Subscribers => "subscribers", Private => "private" });

impl Default for AudienceBase {
    fn default() -> Self {
        // Author-only is the safe, non-widening default (faithful to the prior
        // empty-string -> author-only fall-through). See #499.
        Self::Private
    }
}
```

Update the module doc comment (`:1-2`) to mention audience-base if appropriate
(the list of "audience targeting" types).

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p common visibility` Expected: PASS (new
`audience_base_*` tests + the extended `display_matches_as_str`).

- [x] **Step 5: Commit**

Run `cargo xtask check` first (fmt + clippy + coverage — this is where the
`common`-crate coverage of the new code is enforced); fix any uncovered region
by adding a test (not `cov:ignore`).

```bash
git add common/src/visibility.rs
git commit -m "feat(common): add AudienceBase str_enum with serde (#499)"
```

---

### Task 2: Retype `AudienceSelection.base` and update all consumers

**Files:**

- Modify: `web/src/posts/mod.rs` (field `:84`; `audience_selection_to_targets`
  `:96-113`; `targets_to_audience_selection` `:136-154`; imports; unit tests
  `:704-804`)
- Modify: `web/src/pages/ui.rs` (`base_options` `:463`; `on:change` `:477`;
  `selected` `:489`; `disabled` `:532`; construction `:588`)
- Modify: `web/src/pages/posts.rs` (construction `:642-645`)
- Modify: `server/tests/web/web_posts.rs` (import `:10`; assertions `:3479`,
  `:3528`)

**Interfaces:**

- Consumes: `common::visibility::AudienceBase` (Task 1).
- Produces: `AudienceSelection { base: AudienceBase, named: Vec<AudienceId> }` —
  same wire shape (serde unchanged externally); public API of the two
  `#[server]` fns returning `AudienceSelection` is byte-identical on the wire.

**Why one commit:** the field retype breaks the string-literal comparisons in
`web/src/posts/mod.rs`, `ui.rs`, and `server/tests/web/web_posts.rs` at once;
the workspace does not compile until all are updated, and the commit gate builds
the whole workspace. All edits below land in a single commit.

- [x] **Step 1: Update `web/src/posts/mod.rs` source + unit tests**

Field (`:84`): `pub base: AudienceBase,` (add
`use common::visibility::AudienceBase;` to the top-level `use common::{...}`
block; `AudienceTarget` is already imported locally inside the two fns — extend
those `use common::visibility::{...}` lines to `{AudienceBase, AudienceTarget}`
or import `AudienceBase` at module scope).

`audience_selection_to_targets` (`:100-109`) — exhaustive match, no `_`:

```rust
let base = match selection.base {
    AudienceBase::Public => Some(AudienceTarget::Public),
    AudienceBase::Subscribers => Some(AudienceTarget::Subscribers),
    AudienceBase::Private => None, // author-only; named ignored below
};
let Some(base) = base else {
    return Vec::new();
};
std::iter::once(base)
    .chain(selection.named.iter().copied().map(AudienceTarget::Named))
    .collect()
```

`targets_to_audience_selection` (`:140-153`) — enum accumulator:

```rust
let mut base = AudienceBase::Private;
let mut named = Vec::new();
for target in targets {
    match target {
        AudienceTarget::Public => base = AudienceBase::Public,
        AudienceTarget::Subscribers => base = AudienceBase::Subscribers,
        AudienceTarget::Named(id) => named.push(*id),
        AudienceTarget::Private => {}
    }
}
AudienceSelection { base, named }
```

Unit tests: the `selection` helper (`:704-709`) takes `base: AudienceBase`:

```rust
fn selection(base: AudienceBase, named: &[AudienceId]) -> AudienceSelection {
    AudienceSelection { base, named: named.to_vec() }
}
```

Update each caller: `selection("public", …)` →
`selection(AudienceBase::Public, …)`, etc. In
`private_selection_is_empty_and_ignores_named` (`:784-791`) **remove** the
`"nonsense"` sub-assertion (an invalid base is no longer constructible); keep
the `AudienceBase::Private` case. In `targets_round_trip_through_selection`
(`:807-836`) replace the `"public"/"subscribers"/"private"` string args with the
enum variants. Add `use common::visibility::AudienceBase;` to the test module's
`use` block (`:695-702`).

- [x] **Step 2: Run the web unit tests, verify they pass**

Run: `cargo nextest run -p web posts::tests` (module-path filter — a bare
`selection` substring filter would skip `public_plus_named_unions`, which has no
"selection" in its name) Expected: PASS —
`public_selection_maps_to_public_target`,
`subscribers_selection_maps_to_subscribers_target`, `public_plus_named_unions`,
`private_selection_is_empty_and_ignores_named`,
`absent_selection_defaults_to_public`, `targets_round_trip_through_selection`
all green against the enum.

- [x] **Step 3: Update the DOM edge — `web/src/pages/ui.rs`**

`base_options` (`:463`) → variants (drop the string literals); `base_labels`
(`:464`) unchanged:

```rust
let base_options = [
    AudienceBase::Public,
    AudienceBase::Subscribers,
    AudienceBase::Private,
];
```

Option construction (`:481-495`) — use `as_str()` for `value=`, compare variants
for `selected=` (`AudienceBase` is `Copy + Eq`):

```rust
{base_options
    .iter()
    .zip(base_labels)
    .map(|(base, label)| {
        let base = *base;
        view! {
            <option
                value=base.as_str()
                selected=move || selection.get().base == base
            >
                {label}
            </option>
        }
    })
    .collect_view()}
```

`on:change` (`:474-479`) — parse once, ignore the unreachable `Err`:

```rust
on:change=move |ev| {
    if let Ok(base) = AudienceBase::try_from(event_target_value(&ev).as_str()) {
        selection.update(|sel| sel.base = base);
    }
}
```

`disabled` in `audience_checkbox` (`:532`):
`let disabled = move || selection.get().base == AudienceBase::Private;`

Construction (`:588-591`): `base: AudienceBase::Public,`.

Add `use common::visibility::AudienceBase;` to `ui.rs` imports. (Verify the
existing `use common::...` import block; `AudienceId` is already imported there
per `:529`.)

- [x] **Step 4: Update `web/src/pages/posts.rs` construction**

`:642-645`: `base: AudienceBase::Public,`; add
`use common::visibility::AudienceBase;` to `posts.rs` imports.

- [x] **Step 5: Update the integration assertions —
      `server/tests/web/web_posts.rs`**

Add `use common::visibility::AudienceBase;` to the test module imports (near
`:10`). At `:3479-3480` and `:3528-3530`, change
`assert_eq!(selection.base, "public");` →
`assert_eq!(selection.base, AudienceBase::Public);`.

- [x] **Step 6: Run the full gate, verify it passes**

Run: `cargo xtask check` (foreground; coverage rebuild ~2min — see the
run-slow-gates-foreground note). This compiles the whole workspace (host + wasm
web), runs clippy, the instrumented tests including the `jaunder` integration
tests (PostgreSQL), and the coverage check. Expected: PASS. If clippy flags a
now-redundant `use` or a `to_string` on a `Copy` enum, fix it. Confirm no
`"public"/"subscribers"/"private"` audience-base literal remains in production
code:

Run: `rg -n '"(public|subscribers|private)"' web/src common/src/visibility.rs`
Expected: matches only inside `str_enum!(serde AudienceBase {…})` in
`common/src/visibility.rs` (and `base_labels`-style display strings, which are
not the base values).

- [x] **Step 7: Commit**

```bash
git add web/src/posts/mod.rs web/src/pages/ui.rs web/src/pages/posts.rs server/tests/web/web_posts.rs
git commit -m "refactor(web): type AudienceSelection.base as AudienceBase (#499)"
```

---

## Self-review

- **Spec coverage:** AC#1 → Task 1 (serde/default/Display tests). AC#2 → Task 2
  Step 1 (field type). AC#3 → Task 2 Steps 1/3/4 + Step 6 grep. AC#4 → Task 2
  Step 1 (exhaustive match). AC#5 → Task 1 (serde reject) + Task 2 Step 3 (DOM
  edge `if let Ok`). AC#6 → Task 2 Step 2 (round-trip unit tests) + unchanged
  e2e. AC#7 → Task 1 Step 5 + Task 2 Step 6 (`cargo xtask check`). All covered.
- **Placeholders:** none — every step has concrete code and commands.
- **Type consistency:** `AudienceBase` variants
  (`Public`/`Subscribers`/`Private`), `as_str`, `TryFrom<&str>`,
  `Default = Private`, and `AudienceSelection { base: AudienceBase, named }` are
  used identically across Tasks 1 and 2.
