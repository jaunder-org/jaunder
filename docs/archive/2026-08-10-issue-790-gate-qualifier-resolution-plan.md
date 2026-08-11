# Gate qualifier resolution — implementation plan (#790)

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
`docs/superpowers/specs/2026-08-10-issue-790-gate-qualifier-resolution.md` —
referenced by decision (D1–D6) and acceptance criterion (AC1–AC19). Not restated
here.

**Goal:** Teach `rendered-html-from-trusted` to resolve a `from_trusted` site's
qualifier, so the population holds only `RenderedHtml`'s door and the four
`ContentType` markers go.

**Architecture:** Two pure additions to `xtask/src/steps/ident_gate.rs` — a
tree-wide owner-alias harvest and a per-file qualifier resolver — wired into the
existing `syn` walker behind one optional `Gate` field. **Resolution only ever
suppresses; `visit_ident` stays the sole recorder** (AC1a), so nothing it
catches today is lost and no site can be double-counted.

**Tech Stack:** Rust, `syn` 2 / `proc-macro2`, `cargo xtask`.

**Running the tests:** `xtask` is **not** a workspace member — it has its own
manifest and the flake excludes it. So the tests below run via
`cargo test --manifest-path xtask/Cargo.toml <filter>`, which is exactly what
the `xtask-tests` gate step does (`xtask/src/steps/host_tests.rs:14-17`).
`cargo nextest run -p xtask` fails with "package ID specification `xtask` did
not match any packages".

## Review header

**Scope — in:** `xtask/src/steps/ident_gate.rs`,
`xtask/src/steps/rendered_html_from_trusted_check.rs`, `common/src/media.rs`,
`common/src/feed/feed_path.rs`, `common/src/render.rs` (one stale sentence), a
new `docs/adr/drafts/` entry, stale sentences in ADR-0079 and ADR-0094.

**Scope — out:** renaming either door; behavioural change to
`raw_html_door_check.rs` or `html_sink_check.rs` (they gain `owner: None` and
nothing else); reading qualifiers in macro bodies; resolving a
rename-of-a-rename; moving any gate to `ast-grep` (**#893**, **#894**).

**Tasks:**

1. File the two ast-grep concerns; commit spec and plan.
2. Owner-alias harvest — pure function, TDD.
3. Per-file qualifier resolver — pure struct, TDD.
4. Wire suppression into the walker behind `Gate::owner`.
5. Point the gate at `RenderedHtml` **and** remove the four markers — one
   commit.
6. ADR draft plus the six stale-prose sites.

**Key risks / decisions:**

- **`visit_ident` must remain the only recorder.** A `fn from_trusted`
  definition is not a `syn::Path` — it is caught today because the fn visitors
  recurse into `sig.ident`. So is a method-call ident (`ExprMethodCall.method`)
  and every macro token. If `visit_path` became the recorder,
  `common/src/render.rs:112`'s definition would leave the population and its
  marker at `:111` would fail as a stale orphan. Resolution therefore populates
  a **suppression set** that `visit_ident` consults; it never records. This also
  makes `owner: None` byte-identical by construction (the set is simply empty)
  and makes double-counting impossible.
- **Tasks 5's two halves cannot be split.** The moment `GATE.owner` is set, the
  four `ContentType` markers become orphans and `cargo xtask check` fails — and
  the pre-commit hook runs that gate. Setting the owner and deleting the markers
  is one commit.
- **Multi-segment paths resolve the type name, they do not exempt it.**
  `crate::render::RenderedHtml::from_trusted` must stay in the population. An
  implementation reading ">2 segments ⇒ not the door" passes a naive test suite
  and fails open, so Task 3 tests the positive case explicitly.
- **The one real nested-`use` site.** `common/src/feed/feed_path.rs:7` is
  `use crate::{media::ContentType, tag::Tag, username::Username};`. A resolver
  that only handles flat `use` items leaves `ContentType` unbound there,
  flagging `:98`. Task 3 tests the nested-group form.
- **The owner set is deliberately over-approximated** (Task 2, spec D2):
  widening it only moves sites _into_ the population.
- **Tasks 2–4 land as ONE commit** — found the hard way. `xtask-clippy` runs
  with `-D warnings`, so `-D dead-code` rejects `owner_aliases` / `Resolver`
  while only `#[cfg(test)]` code calls them; a `#[cfg(test)]` caller does not
  count as a use for the non-test build. Tasks 2 and 3 therefore cannot be
  committed before Task 4 wires them into the walker. They stay separate
  **development** units — write and green each one's tests in turn — but the
  commit comes once, at the end of Task 4. The alternative, an
  `#[expect(dead_code)]` bridging two commits, needs explicit user approval per
  CONTRIBUTING and would be a suppression that exists purely to satisfy commit
  granularity.
- Unlike #863 this is host-compiled, so **every criterion is a real test**.

## Global Constraints

- Gate steps live under `POLICED_ROOTS`; xtask is host-only, never inside a Nix
  derivation.
- `ast-grep` is **not** in the flake — do not reach for it (Task 1).
- No `Co-Authored-By` trailer. Stage, then commit. Never
  `git commit -- <paths>`.
- The pre-commit hook runs the full `cargo xtask check`; run it first so it
  passes clean.
- Coverage policy applies to xtask: new functions need tests in the same commit.
- Test-module imports: `mod tests` currently has only `use super::scan;` and
  `mod marker_tests` only `use super::{Classified, Why, classify, scan};`. Every
  task below must extend those `use` lists for the items it introduces.

---

### Task 1: File the ast-grep concerns; commit spec and plan

**Files:** `docs/superpowers/{specs,plans}/2026-08-10-issue-790-*.md` (commit
only).

- [x] **Step 1: File two issues** via `jaunder-issues`, label `tooling`:
  1. **"no-full-reload: move to an ast-grep rule — per-line matching admits a
     documented evasion"** — `xtask/src/steps/no_full_reload_check.rs:9-11`
     states matching is per-line, so a formatter splitting the chain across
     lines slips through. The one gate with no allowlist, no markers and no
     counting, so it ports cleanly and the pattern fixes the evasion as a side
     effect. Blocked on adding `ast-grep` to the devShell **and** to any Nix
     check derivation that runs it, plus a rule dir inside the flake `src`
     filter.
  2. **"Consider ast-grep as the matching layer for the ident gates, keeping
     policy in xtask"** — the match half is a trivial rule; all the value is in
     the policy layer (`markers.rs`, required reasons, `Why::Shared` per-line
     counting, orphan detection). ast-grep's only suppression is
     `# ast-grep-ignore`: no required reason and **no orphan detection**, the
     staleness class ADR-0094/#778 exist to catch. Record the hybrid shape
     (ast-grep replaces `scan`, `classify` unchanged) and the two caveats:
     `Mention.function` must be reconstructed from an `inside` capture, and
     `test_ranges` needs a second rule.
- [x] **Step 2: Record the numbers** in this plan's Review header — **#893**
      (`no-full-reload`, the one fully-movable gate) and **#894** (the hybrid
      matching layer for the ident gates).
- [x] **Step 3: Commit the spec and plan** — `cargo xtask validate` refuses a
      dirty tree.

```bash
git add docs/superpowers/specs/2026-08-10-issue-790-gate-qualifier-resolution.md docs/superpowers/plans/2026-08-10-issue-790-gate-qualifier-resolution.md
git commit -m "docs(plan): spec and plan for gate qualifier resolution (#790)"
```

---

### Task 2: Owner-alias harvest

**Files:** Modify `xtask/src/steps/ident_gate.rs` — new pure fn plus tests in
`mod tests`.

**Interfaces:**

- Produces
  `pub fn owner_aliases(sources: &[(String, String)], owner: &str) -> BTreeSet<String>`
  — every ident that can denote `owner` anywhere in the tree; always contains
  `owner`. Tasks 4 and 5 consume it.

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn the_owner_is_always_in_its_own_alias_set() {
    let set = owner_aliases(&[], "Owner");
    assert_eq!(set.len(), 1, "an empty tree yields the owner alone: {set:?}");
    assert!(set.contains("Owner"));
}

#[test]
fn a_renaming_use_of_the_owner_contributes_its_new_name() {
    let src = ("a.rs".to_string(), "use crate::render::Owner as Doc;\n".to_string());
    assert!(owner_aliases(&[src], "Owner").contains("Doc"));
}

#[test]
fn a_type_alias_to_the_owner_contributes_its_name() {
    let src = ("a.rs".to_string(), "type Html = Owner;\n".to_string());
    assert!(owner_aliases(&[src], "Owner").contains("Html"));
}

#[test]
fn a_nested_use_group_still_yields_the_rename() {
    let src = (
        "a.rs".to_string(),
        "use crate::render::{Sanitizer, Owner as Doc};\n".to_string(),
    );
    assert!(owner_aliases(&[src], "Owner").contains("Doc"));
}

#[test]
fn unrelated_renames_and_aliases_are_ignored() {
    let src = (
        "a.rs".to_string(),
        "use crate::media::ContentType as Ct;\ntype Bytes = Vec<u8>;\n".to_string(),
    );
    assert_eq!(owner_aliases(&[src], "Owner").len(), 1);
}

#[test]
fn a_plain_non_renaming_import_of_the_owner_adds_nothing() {
    let src = ("a.rs".to_string(), "use crate::render::Owner;\n".to_string());
    assert_eq!(owner_aliases(&[src], "Owner").len(), 1, "already the owner's own name");
}

#[test]
fn the_harvest_spans_files_and_is_order_independent() {
    let a = ("a.rs".to_string(), "use crate::render::Owner as Doc;\n".to_string());
    let b = ("b.rs".to_string(), "type Html = Owner;\n".to_string());
    let forward = owner_aliases(&[a.clone(), b.clone()], "Owner");
    let backward = owner_aliases(&[b, a], "Owner");
    assert_eq!(forward, backward);
    assert!(forward.contains("Doc") && forward.contains("Html"));
}

#[test]
fn an_unparseable_file_is_skipped_rather_than_panicking() {
    let src = ("a.rs".to_string(), "fn (((".to_string());
    assert_eq!(owner_aliases(&[src], "Owner").len(), 1);
}
```

- [x] **Step 2: Run them, verify they fail**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo test --manifest-path xtask/Cargo.toml steps::ident_gate::tests
```

Expected: FAIL — `owner_aliases` not defined.

- [x] **Step 3: Implement against the tests**

The tests pin every branch. Signature and doc:

```rust
/// Every ident that can denote `owner` anywhere in the scanned tree.
///
/// A renaming re-export in one module (`pub use crate::render::RenderedHtml as Doc;`)
/// makes `Doc::from_trusted` in *another* module a site on the owner's door; per-file
/// resolution alone would miss it (#790, spec D2).
///
/// Deliberately **over-approximates** — an ident lands here on a name match alone, so a
/// `type ContentType = RenderedHtml;` anywhere would pull genuine `ContentType` sites into
/// the population. That is the fail-closed direction: an over-large owner set costs a
/// marker, an under-large one loses an XSS door.
///
/// A `syn` parse failure is skipped rather than fatal. This is a widening pass, and
/// `scan` already hard-errors on an unparseable file, so a second error path here would
/// only duplicate that one.
pub fn owner_aliases(sources: &[(String, String)], owner: &str) -> BTreeSet<String>
```

- [x] **Step 4: Run the tests, verify they pass** — same command. Expected:
      PASS.
- [x] **Step 5: Gate and commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo xtask check
git add xtask/src/steps/ident_gate.rs
git commit -m "feat(xtask): harvest owner aliases across policed roots (#790)"
```

---

### Task 3: Per-file qualifier resolver

**Files:** Modify `xtask/src/steps/ident_gate.rs` — new pure types plus tests.

**Interfaces:**

- Consumes `owner_aliases` (Task 2) — the caller passes the set in.
- Produces:

```rust
/// Whose door a policed site belongs to.
///
/// Named for the question it answers, not for the gate's `owner` field: `Gate::owner` is
/// a type *name*, this is a *verdict*.
#[derive(Debug, PartialEq, Eq)]
pub enum Membership {
    /// The qualifier denotes the gate's owner type — the real door.
    Door,
    /// The qualifier denotes some other, named type — not this door.
    OtherType,
    /// The qualifier could not be determined; the site stays in the population (D1).
    Unknown,
}

pub struct Resolver {
    /* per-file: non-glob `use` bindings, in-file type definitions */
}

impl Resolver {
    /// Build from one file's parsed items.
    pub fn for_file(file: &syn::File) -> Self;

    /// Classify a path whose leaf is a policed ident.
    /// `impl_self` is the enclosing `impl`'s self-type name, for `Self::`.
    pub fn membership(
        &self,
        path: &syn::Path,
        owners: &BTreeSet<String>,
        impl_self: Option<&str>,
    ) -> Membership;
}
```

- [x] **Step 1: Write the test helper, precisely**

Eleven tests depend on it, and a wrong helper silently inverts assertions:

```rust
/// The first path in `file` whose **last** segment is `leaf`, in visit order.
///
/// Must return a single-segment path for an unqualified `from_trusted(x)` call (not
/// `None` — "unqualified" is a verdict the resolver produces, not an absence), and the
/// full four-segment path for `crate::media::ContentType::from_trusted(x)`. Must ignore
/// `use` items, or a fixture's own import would be found before its call site.
fn first_policed_path(file: &syn::File, leaf: &str) -> Option<syn::Path>
```

- [x] **Step 2: Write the failing tests**

```rust
fn resolve(src: &str, owners: &[&str], impl_self: Option<&str>) -> Membership {
    let file: syn::File = syn::parse_str(src).expect("fixture parses");
    let set: BTreeSet<String> = owners.iter().map(|s| (*s).to_string()).collect();
    let path = first_policed_path(&file, "from_trusted").expect("fixture has a site");
    Resolver::for_file(&file).membership(&path, &set, impl_self)
}

#[test]
fn a_bare_owner_qualifier_is_the_door() {
    assert_eq!(resolve("fn f() { Owner::from_trusted(x); }\n", &["Owner"], None), Membership::Door);
}

#[test]
fn a_renamed_owner_qualifier_is_the_door() {
    // The #778 hole, closed by resolution rather than over-approximation (AC5).
    let src = "use crate::render::Owner as Doc;\nfn f() { Doc::from_trusted(x); }\n";
    assert_eq!(resolve(src, &["Owner", "Doc"], None), Membership::Door);
}

#[test]
fn a_fully_qualified_owner_path_is_still_the_door() {
    // Fails open if ">2 segments" is read as "not the door".
    let src = "fn f() { crate::render::Owner::from_trusted(x); }\n";
    assert_eq!(resolve(src, &["Owner"], None), Membership::Door);
}

#[test]
fn a_multi_segment_path_names_its_type_and_needs_no_import() {
    let src = "fn f() { crate::media::ContentType::from_trusted(x); }\n";
    assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
}

#[test]
fn self_inside_the_owners_impl_is_the_door() {
    assert_eq!(
        resolve("fn f() { Self::from_trusted(x); }\n", &["Owner"], Some("Owner")),
        Membership::Door
    );
}

#[test]
fn self_inside_another_impl_is_not_the_door() {
    assert_eq!(
        resolve("fn f() { Self::from_trusted(x); }\n", &["Owner"], Some("ContentType")),
        Membership::OtherType
    );
}

#[test]
fn a_qualifier_defined_in_this_file_resolves_to_itself() {
    let src = "struct ContentType(String);\nfn f() { ContentType::from_trusted(x); }\n";
    assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
}

#[test]
fn a_qualifier_imported_by_a_flat_use_resolves() {
    let src = "use crate::media::ContentType;\nfn f() { ContentType::from_trusted(x); }\n";
    assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
}

#[test]
fn a_qualifier_imported_by_a_nested_use_group_resolves() {
    // The form `common/src/feed/feed_path.rs:7` actually uses.
    let src = "use crate::{media::ContentType, tag::Tag};\nfn f() { ContentType::from_trusted(x); }\n";
    assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
}

#[test]
fn an_in_file_type_alias_resolves_without_the_owner_set() {
    // The alias is NOT pre-seeded into `owners`, so this exercises the in-file branch
    // rather than short-circuiting on the owner set.
    let src = "type Ct = ContentType;\nfn f() { Ct::from_trusted(x); }\n";
    assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
}

#[test]
fn an_unbound_bare_qualifier_is_unknown() {
    let src = "use foo::*;\nfn f() { Mystery::from_trusted(x); }\n";
    assert_eq!(resolve(src, &["Owner"], None), Membership::Unknown);
}

#[test]
fn a_generic_parameter_qualifier_is_unknown() {
    assert_eq!(resolve("fn f<T>() { T::from_trusted(x); }\n", &["Owner"], None), Membership::Unknown);
}

#[test]
fn an_unqualified_call_is_unknown() {
    assert_eq!(resolve("fn f() { from_trusted(x); }\n", &["Owner"], None), Membership::Unknown);
}
```

- [x] **Step 3: Run them, verify they fail**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo test --manifest-path xtask/Cargo.toml steps::ident_gate::resolver_tests
```

Expected: FAIL — `Resolver` / `Membership` not defined.

- [x] **Step 4: Implement against the tests**

Classification reads the segment _before_ the leaf. Order matters and every
branch has a test: owner set first (so a renamed owner wins over any other
reading), then the multi-segment spelled-out name, then in-file `use` bindings,
then in-file type definitions, then `Self` via `impl_self`; a single-segment
path is an unqualified call and is `Unknown`. `Resolver::for_file` collects
non-glob `use` leaves — recursing `UseTree` through
`UsePath`/`UseGroup`/`UseName`/`UseRename`, ignoring `UseGlob` — plus
`struct`/`enum`/`union`/`type` names.

- [x] **Step 5: Run the tests, verify they pass** — same command. Expected:
      PASS.
- [x] **Step 6: Gate and commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo xtask check
git add xtask/src/steps/ident_gate.rs
git commit -m "feat(xtask): resolve a policed path's qualifier within a file (#790)"
```

---

### Task 4: Wire suppression into the walker

**Files:**

- Modify: `xtask/src/steps/ident_gate.rs` — `Gate`, `Scanner`, `scan`,
  `Gate::problems` (`:476`), `Gate::violations` (`:444`)
- Modify: `xtask/src/steps/raw_html_door_check.rs:74-78` and
  `xtask/src/steps/html_sink_check.rs:85-89` — **one line each, `owner: None`**.
  `Gate` is a `const` struct literal in both and has no `Default`, so
  `..Default::default()` is not available in a `const` initializer: a new field
  is a hard compile error until they name it. This is a compile requirement, not
  a behaviour change.

**Interfaces:**

- Consumes `owner_aliases` (Task 2), `Resolver`/`Membership` (Task 3).
- Produces: `Gate` gains `pub owner: Option<&'static str>`. New
  `pub fn scan_owned(source: &str, population: &[&str], owner: Option<(&str, &BTreeSet<String>)>) -> Result<Scan, String>`
  — the owner name and its alias set travel as **one** argument, because they
  are meaningless apart and must not disagree. `scan` keeps its signature and
  delegates with `None`, so the sibling gates and every existing shared test are
  untouched (AC1). `Gate::problems` builds the owner set from the
  `(path, source)` pairs it already has; `Gate::violations` builds a single-file
  set from its own fixture.

- [x] **Step 1: Write the failing tests**

Add a `["from_trusted"]` helper beside the existing `classified` (which scans
`["GUARDED"]` and must keep doing so):

```rust
fn classified_owned(src: &str) -> Classified {
    let owners = owner_aliases(&[("t.rs".into(), src.to_string())], "Owner");
    let s = scan_owned(src, &["from_trusted"], Some(("Owner", &owners))).unwrap();
    classify(src, &s, TOKEN)
}

fn classified_unowned(src: &str) -> Classified {
    let s = scan(src, &["from_trusted"]).unwrap();
    classify(src, &s, TOKEN)
}

#[test]
fn an_owner_qualified_site_still_needs_a_marker() {
    assert_eq!(classified_owned("fn a() { Owner::from_trusted(x); }\n").unexempt.len(), 1);
}

#[test]
fn another_types_site_needs_no_marker() {
    let c = classified_owned("struct ContentType;\nfn a() { ContentType::from_trusted(x); }\n");
    assert!(c.unexempt.is_empty(), "not this door: {:?}", c.unexempt);
    assert!(c.marked.is_empty(), "and it earns no census entry either");
}

#[test]
fn self_in_the_owners_impl_needs_a_marker() {
    // Exercises `impl_stack`, which the Resolver tests cannot reach.
    let c = classified_owned("impl Owner { fn f() { Self::from_trusted(x); } }\n");
    assert_eq!(c.unexempt.len(), 1);
}

#[test]
fn self_in_another_impl_needs_no_marker() {
    let c = classified_owned("impl ContentType { fn f() { Self::from_trusted(x); } }\n");
    assert!(c.unexempt.is_empty(), "{:?}", c.unexempt);
}

#[test]
fn the_owners_definition_site_needs_a_marker() {
    // A `fn` ident is not a Path: this passes only if the fn visitors participate.
    let c = classified_owned("impl Owner { fn from_trusted(v: V) -> Self { v } }\n");
    assert_eq!(c.unexempt.len(), 1);
}

#[test]
fn another_types_definition_site_needs_no_marker() {
    let c = classified_owned("impl ContentType { fn from_trusted(v: V) -> Self { v } }\n");
    assert!(c.unexempt.is_empty(), "{:?}", c.unexempt);
}

#[test]
fn a_free_module_scope_definition_is_flagged() {
    let c = classified_owned("fn from_trusted(v: V) -> V { v }\n");
    assert_eq!(c.unexempt.len(), 1, "no impl, so no owner to rule out");
}

#[test]
fn an_unqualified_call_is_flagged() {
    let c = classified_owned("fn a() { from_trusted(x); }\n");
    assert_eq!(c.unexempt.len(), 1);
}

#[test]
fn an_unresolvable_qualifier_is_flagged() {
    let c = classified_owned("use foo::*;\nfn a() { Mystery::from_trusted(x); }\n");
    assert_eq!(c.unexempt.len(), 1);
}

#[test]
fn another_types_site_in_a_macro_body_is_still_flagged() {
    // D4: macro bodies are not resolved, so they stay in the population.
    let c = classified_owned("struct ContentType;\nfn a() { view! { ContentType::from_trusted(x) } }\n");
    assert_eq!(c.unexempt.len(), 1);
}

#[test]
fn a_marker_over_a_now_ignored_site_is_an_orphan() {
    // The mirror of Task 5's marker deletions.
    let c = classified_owned(
        "struct ContentType;\n// guard:allow stale\nfn a() { ContentType::from_trusted(x); }\n",
    );
    assert_eq!(c.orphans, vec![2]);
}

#[test]
fn without_an_owner_every_site_is_in_the_population() {
    // AC1: the sibling gates' behaviour is unchanged.
    let c = classified_unowned("struct ContentType;\nfn a() { ContentType::from_trusted(x); }\n");
    assert_eq!(c.unexempt.len(), 1);
}

#[test]
fn a_site_is_recorded_exactly_once() {
    // AC1a: resolution suppresses, never records. The `len() == 1` is what pins it —
    // double-recording on an unmarked line yields two `Unmarked` entries, not `Shared`.
    let c = classified_owned("fn a() { Owner::from_trusted(x); }\n");
    assert_eq!(c.unexempt.len(), 1, "recorded once, not once per hook");
    assert!(matches!(c.unexempt[0].why, Why::Unmarked));
}

#[test]
fn an_owner_alias_from_another_file_puts_a_site_in_the_population() {
    // AC8 — D2's whole reason for existing. Must fail if the harvest is removed.
    let reexport = ("a.rs".to_string(), "pub use crate::render::Owner as Doc;\n".to_string());
    let site_src = "use crate::a::Doc;\nfn f() { Doc::from_trusted(x); }\n";
    let site = ("b.rs".to_string(), site_src.to_string());
    let owners = owner_aliases(&[reexport, site], "Owner");
    let s = scan_owned(site_src, &["from_trusted"], Some(("Owner", &owners))).unwrap();
    assert_eq!(classify(site_src, &s, TOKEN).unexempt.len(), 1);
}
```

- [x] **Step 2: Run them, verify they fail**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo test --manifest-path xtask/Cargo.toml steps::ident_gate
```

Expected: FAIL — `scan_owned` not defined.

- [x] **Step 3: Implement against the tests**

`Scanner` gains `owner: Option<(&'p str, &'p BTreeSet<String>)>` — **one field
carrying the tuple**, for the same reason `scan_owned` takes one argument: with
no owner there is no `&BTreeSet` to store, so split fields would need an
`EMPTY: BTreeSet` const just to compile. It also gains a `Resolver`, an
`impl_stack: Vec<Option<String>>`, and `suppressed: HashSet<(usize, usize)>`
keyed on `ident.span().start()`.

The span key is sound because `xtask/Cargo.toml:27` enables proc-macro2's
`span-locations` feature — that one feature gates all of `LineColumn`, so
`column` is exactly as real as the `line` the existing code already relies on
(`ident_gate.rs:290`, `:320`, `:385`). Note `line` is 1-based and `column`
0-based; do not let the tuple's `.0` be confused with `Mention.line`.

**Each suppressing hook must insert into `suppressed` _before_ delegating to
`syn::visit::visit_*`.** Reversing those two lines still compiles and silently
suppresses nothing.

Three walker changes:

1. **`visit_ident` stays the only recorder** — it records when `is_member(i)`
   **and** the ident's span is not in `suppressed`. Unchanged otherwise.
2. **`visit_path`** (new): when an owner is configured and the path's leaf is a
   member, call `Resolver::membership`; on `OtherType`, insert the **leaf
   ident's** span into `suppressed`. `Door` and `Unknown` insert nothing, so
   `visit_ident` records them. Always delegate to `syn::visit::visit_path`. With
   no owner it inserts nothing at all.
3. **`visit_item_impl`** (`:339-347`) additionally pushes the self-type's final
   path segment onto `impl_stack` (`None` for a non-path self-type), popping
   after recursion — the same push/recurse/pop shape `fn_stack` already uses.
4. **The two fn visitors** `visit_item_fn` (`:349`) and `visit_impl_item_fn`
   (`:361`): when an owner is configured and `sig.ident` is a member, consult
   `impl_stack.last().and_then(Option::as_deref)`. An enclosing impl whose type
   is **not** in the owner set suppresses the `sig.ident` span; the owner's own
   impl, or no impl at all, suppresses nothing — so the owner's definition and a
   free module-scope definition both stay flagged. That flattening conflates "no
   impl" with "non-path self type", which is the fail-closed direction.

**Do not add a `visit_trait_item_fn` override.** A `TraitItemFn` is reached
through `visit_item_trait`, not `visit_item_impl`, so `impl_stack` is empty
there and the override would suppress nothing — a trait's `fn from_trusted`
stays flagged, which is both correct and what happens without the override. The
one case where `impl_stack.last()` _is_ populated inside a trait item — a
`trait` declared inside a non-owner impl's method body — is precisely where it
would suppress **wrongly**.

`walk_macro_tokens` (`:311-325`) is untouched — D4.

- [x] **Step 4: Run the whole xtask suite**

AC1 rests on the pre-existing shared `marker_tests` passing untouched, so run
everything, not just the new tests:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo test --manifest-path xtask/Cargo.toml
```

Expected: PASS, including every pre-existing `marker_tests` case.

- [x] **Step 5: Gate and commit** — `GATE.owner` is still unset, so the tree is
      unchanged.

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo xtask check
git add xtask/src/steps/ident_gate.rs
git commit -m "feat(xtask): suppress policed sites whose qualifier is another type (#790)"
```

---

### Task 5: Point the gate at `RenderedHtml` and drop the markers

**One commit.** Setting `GATE.owner` makes the four `ContentType` markers
orphans immediately, and the pre-commit hook runs the gate — so the marker
deletions cannot land in a later commit.

**Files:**

- Modify: `xtask/src/steps/rendered_html_from_trusted_check.rs` — `GATE`
  (`:92`), `verdict` (`:98-100`), `recovery` (`:101-109`), module doc (`:21-37`,
  `:55-59`), fixtures at `:194`, `:200`, `:206`, and the verdict test at `:380`
- Modify: `common/src/media.rs` — markers at `:872`, `:968`, `:972`; the doc
  paragraph inside `:858-870`; the prose at `:939` (**locate by content** — Step
  3 shifts it)
- Modify: `common/src/feed/feed_path.rs` — marker at `:97`; prose at `:87`

**Interfaces:** Consumes `Gate::owner` (Task 4). No API changes anywhere (AC15).

- [x] **Step 1: Fix the gate's fixtures**

Read them first — this is where the plan's earlier draft was wrong. `:195`'s
fixture is
`"fn detect(n: &str) -> ContentType { ContentType::from_trusted(n) }\n"`: **no
`use`, no `struct`, no impl**, so under D3 that qualifier is `Unknown` and the
site is _still flagged_. So:

- `a_content_type_door_is_in_the_population_and_needs_a_marker` (`:194`) and
  `a_marked_content_type_door_passes` (`:200`) keep passing unchanged. Retitle
  them to say what they now demonstrate — an **unresolvable** qualifier, not "a
  different type".
- Add the payoff test: the same call with a binding (`struct ContentType;` in
  the fixture, or `use crate::media::ContentType;`) is clean **and unmarked**.
- `a_from_trusted_on_an_unrelated_type_is_still_flagged` (`:206`) — `Widget` is
  likewise unbound, so it already asserts the `Unknown` path. Keep it, and add a
  bound-`Widget` sibling asserting clean, so both halves of the rule have a
  test.

- [x] **Step 2: Run them, verify the new ones fail**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo test --manifest-path xtask/Cargo.toml steps::rendered_html_from_trusted_check
```

Expected: the two **new** bound-qualifier tests FAIL (owner not yet set); the
retitled ones PASS.

- [x] **Step 3: Set the owner, fix the prose, delete the markers**

- `GATE` gains `owner: Some("RenderedHtml")`.
- `verdict` names `RenderedHtml`'s door instead of hedging about every
  `from_trusted` in production code.
- `recovery` **drops** "A `from_trusted` on a different type (`ContentType`,
  #584) is not this door at all — say so and move on." In its place, say what an
  unresolvable qualifier means: the gate could not tell whose door this is, so
  name the type explicitly or mark the site.
- Module doc (`:21-37`, `:55-59`) — replace the `ContentType`-collision passage
  and "#790 tracks removing the collision at its source instead" with D1's rule
  and D3's accepted blind spot (AC19).
- Delete the four markers, located by content
  (`// rendered-html-from-trusted:allow ContentType…`). **Leave both
  `common/src/render.rs` markers** — `:111` and `:156` — alone.
- Rewrite `ContentType::from_trusted`'s doc: delete the whole parenthetical
  about `#398`, `#778` and `#790`. The "grep `ContentType::from_trusted` to
  enumerate every mint site" instruction must now stand on its own — say it is a
  convention backed by the named tests (`detect_content_type_outputs_are_valid`,
  `feed_path::…::format_content_types`), not a build-time guarantee (AC13).
- Fix the two prose references at `media.rs:939` and `feed_path.rs:87` (AC14).

- [x] **Step 4: Run the tests**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo test --manifest-path xtask/Cargo.toml
```

Expected: PASS. The verdict test at `:380` is another unresolvable-qualifier
fixture, so its assertions still hold — but its doc comment ("The verdict fires
at `ContentType::from_trusted` and at definition sites too") goes stale and must
be rewritten to say "at unresolvable-qualifier sites".

- [x] **Step 5: Confirm the sibling gates changed by one line each and no more**

```bash
git diff wt-base-issue-790...HEAD -- xtask/src/steps/raw_html_door_check.rs xtask/src/steps/html_sink_check.rs
```

Expected: exactly one added line per file, `owner: None,` — the `const Gate`
literals had to name the new field to compile (Task 4). Anything else in that
diff is out of scope (AC9).

- [x] **Step 6: Confirm the gate agrees on the real tree**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo xtask check
```

Expected: PASS with `[ ok ] rendered-html-from-trusted` — zero violations with
the markers gone (AC16). A failure here is informative: `Unmarked` at a
`render.rs` line means resolution is not reaching the owner's own door; an
orphan means a marker outlived its site.

- [x] **Step 7: Confirm no rename crept in**

```bash
rg -n 'fn from_trusted' common/src
```

Expected: two definitions, `common/src/media.rs` and `common/src/render.rs`,
both still named `from_trusted` (AC15).

- [x] **Step 8: Commit**

```bash
git add xtask/src/steps/rendered_html_from_trusted_check.rs common/src/media.rs common/src/feed/feed_path.rs
git commit -m "feat(xtask,common): police RenderedHtml's door only; drop the ContentType markers (#790)"
```

---

### Task 6: ADR draft and the six stale-prose sites

**Files:**

- Create: `docs/adr/0110-gate-population-membership-is-structural.md`
- Modify: `docs/adr/0079-*.md` (§89), `docs/adr/0094-*.md` (§122-127 and §229),
  `common/src/render.rs` (`:216-218`), `xtask/src/steps/ident_gate.rs` (module
  doc §29-33, §50-52, §69-73)

**Interfaces:** none. The draft is numbered at ship by
`cargo xtask adr promote`.

- [x] **Step 1: Write the ADR draft** via `jaunder-adr` — numberless, in
      `docs/adr/drafts/`. Content per D6:
  - **Context** — #398 built the gate on a leaf ident; #778 deleted a qualifier
    exemption as a pattern-decided exemption under ADR-0085 principle 3, leaving
    the codebase paying for the over-approximation in markers on a provably
    harmless population.
  - **Decision** — identifying a gate's population is **structural**; exempting
    a site from it requires a human marker. Reading a qualifier to decide
    membership is therefore not a self-exemption. A gate that cannot determine
    membership must **fail closed** and keep the site in the population — that
    is what makes narrowing safe.
  - **Consequences** — a gate may police a name another type also uses without
    taxing that type; ADR-0085 principle 3 is unchanged and now applies only to
    exemptions, which is what it was always about. The residual blind spots are
    bounded and enumerated (Step 5), and one deserves naming here: **membership
    resolution is only as wide as the gate's roots.** A renaming re-export
    living outside `POLICED_ROOTS` is never harvested, so a use site inside them
    could resolve to another type and be suppressed. That is the price of
    resolving names without a compiler, and it is why the roots must cover every
    tree the gate claims to police.
- [x] **Step 2: ADR-0079 §89** — "the `from_trusted` ident wherever it appears
      (#778 widened it to definitions and to other types' doors)" is now false
      for other types' doors. State the resolved-qualifier rule; cite the new
      ADR.
- [x] **Step 3: `common/src/render.rs:216-218`** — carries that same sentence
      **verbatim in code**. Fix it identically.
- [x] **Step 4: ADR-0094** — §229 (the note that `ident_gate` lost the free
      `ContentType` coverage; it is back, by resolution) and §122-127 (that the
      affected sites "take ordinary markers like anything else", which turned
      the doc-comment instruction into something "the gate enforces" — the exact
      claim Task 5 walked back in `media.rs`).
- [x] **Step 5: `ident_gate.rs`'s module doc** — §50-52's unreadable class 1 ("A
      `use … as` rename … evades ident matching — `syn` has no name resolution")
      is now false for an owner-configured gate; §29-33 and §69-73 describe the
      old design. Rewrite, and add to that same numbered blind-spot list all
      three residual evasions (AC19):
  1. **rename-of-a-rename** — D2 harvests a single `use …Owner as X`; a rename
     of a rename across two modules evades.
  2. **a renaming re-export outside `POLICED_ROOTS`** — the harvest only sees
     the files `run_scan` collected from `roots`, so a rename living outside
     them is invisible, and a use site inside them spelling the alias would
     resolve to `OtherType` and be suppressed. Fail-open. No live hole, since
     the roots cover every production `src` tree — but it is the sharper
     statement of blind spot 1 and belongs in the ADR's Consequences too.
  3. **a free `fn from_trusted` nested inside a non-owner impl's method body** —
     `impl_stack.last()` still names the impl, so the inner definition is
     suppressed although it belongs to no impl. Fail-open, vanishingly unlikely.
- [x] **Step 6: Gate and commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo xtask check
```

Expected: PASS, including `adr-format` and `adr-readme-parity`. A numberless
draft must not appear in the README table — `adr promote` does that at ship.

```bash
git add docs/adr common/src/render.rs xtask/src/steps/ident_gate.rs
git commit -m "docs(adr): record that gate population membership is structural (#790)"
```

---

## Before the PR

A foreground `devtool run` that exceeds 10 minutes is moved to the background
and survives; a run started in background mode gets killed. Prefer the
foreground form for long gates.

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-790-contenttype-rename -- cargo xtask validate
```

The branch touches `common/`, so run the full `validate` with e2e rather than
`--no-e2e`. Then hand off to `jaunder-ship`.
