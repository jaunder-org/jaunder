# `#[macros::server]` Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating individual tasks to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-30-issue-714-server-fn-attribute-macro.md`](../specs/2026-07-30-issue-714-server-fn-attribute-macro.md).
The spec is the _what/why_; this plan is the _how_. Reference it by AC number
rather than re-deriving its analysis.

**Goal:** Replace three per-server-fn derived literals with one attribute macro
that derives all of them, and make `(vertical, ident)` a uniqueness key the
compiler enforces.

**Architecture:** A `#[proc_macro_attribute]` in `macros` splits into a pure
core (path + args → derived values, unit-testable) and a thin shell (calls
`Span::call_site().file()`). It emits `#[::leptos::server]` +
`#[::tracing::instrument]` and wraps the body in
`crate::error::server_boundary`. A placement rule — server fns live only in
`web/src/<vertical>/api.rs` — turns `(vertical, ident)` into a primary key,
which requires moving five timeline queries into a new `timeline` vertical.

**Tech Stack:** Rust (stable), `syn` 2 + `quote` + `proc-macro2`, leptos
`#[server]`, `tracing` / `tracing-opentelemetry`, `cargo xtask` gates,
Playwright e2e.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- Every commit must pass the pre-commit gate: run `cargo xtask check` first
  (**jaunder-commit**). One clean commit per task. **No task may commit a
  knowingly-red tree** — the gate is git-enforced.
- **`git add` new files before running any Nix-backed step.** Flake source is
  git-tracked-only, so an untracked new file is invisible to the derivations and
  `cargo xtask check` will build a tree that does not contain it. Affects Tasks
  4, 8, 10.
- `CONTRIBUTING.md` is binding: backend parity, coverage policy, dialect-file
  rules.
- `macros` **is** coverage-measured — error paths need tests; only the thin
  shell is exempted, via block-form `cov:ignore-start`/`-stop` with a reason
  (`CONTRIBUTING.md:451,458,539`). There is no structural "thin-shell"
  exemption.
- Wire URLs and span names change **only** for the five timeline queries; the
  other 50 keep their exact current values.
- `docs/coverage/server-fns.json` and the seed capture are **generated — never
  hand-edited** (`CONTRIBUTING.md:480`).
- Run gates in the foreground with a long timeout (`timeout: 600000`);
  background runs get killed.

---

## Review header

**Scope — in:** the attribute macro; converting all 55 server fns; deleting the
`boundary!` label and the `server_fn` log field; the `timeline` vertical move;
retiring `server-fn-endpoint` and tracing rules 1–2; coverage-gate endpoint
computation; ADR/doc updates for #714, #722, #698.

**Scope — out:** retiring the registrar gate via `linkme` (Task 1 files it); the
TypeScript URL guard (#712); relocating `server/tests/web/web_posts.rs` tests;
changing any of the other 50 URLs or span names.

| #   | Task                                                         | Delivers                         |
| --- | ------------------------------------------------------------ | -------------------------------- |
| 1   | File the `linkme` follow-up issue                            | separable concern captured       |
| 2   | **P1: prove the span is in scope at the boundary event**     | **voids the spec if it fails**   |
| 3   | Delete the label                                             | AC-6                             |
| 4   | `macros`: `syn` `full` + pure core                           | AC-7, AC-8, AC-9 (happy), AC-17a |
| 5   | `macros`: the attribute shell; `web` depends on `macros`     | AC-11, AC-13, AC-17b             |
| 6   | **All four gate consumers tolerate the new spelling**        | AC-3, AC-14 (part)               |
| 7   | Convert the 50 fns already in `<vertical>/api.rs`            | AC-2 (part), AC-12               |
| 8   | The `timeline` move                                          | AC-2 (rest), AC-12               |
| 9   | **e2e → regenerate** — restores a green `check`              | AC-19                            |
| 10  | Runtime wire assertions                                      | AC-4, AC-5                       |
| 11  | Retire the old world                                         | AC-15, AC-16, gate deletions     |
| 12  | Coverage: computed endpoint, dead branches, seed cross-check | AC-9 (anti-drift), AC-10         |
| 13  | ADRs and docs; final `validate --no-e2e`                     | AC-20 – AC-24, AC-18             |

**Key risks and decisions:**

- **Task 2 can void the plan.** If the instrument span is not in scope when the
  boundary logs, the label is not redundant. Stop and re-open the design; the
  fallback is #714's original comparing gate.
- **Four gates consume one enumeration, not two.**
  `web_server_fns::server_fns_in` feeds the registrar gate, the tracing gate,
  `server_fn_endpoint_check`, **and** `xtask/src/server_fns.rs` (the coverage
  inventory). Task 6 must teach _all four_, or Task 7's commit fails the gate 50
  times: `endpoint_of` returns `None` for a `#[macros::server]` attribute, and
  the endpoint gate hard-errors on that (`server_fn_endpoint_check.rs:105-111`)
  while `Mode::Fix` deliberately **never** synthesizes an endpoint (`:14`).
- **Task 9 is not optional and cannot be deferred.** The moment Task 8 creates
  `timeline/api.rs`, the live inventory keys become `timeline::list_*` while
  `docs/coverage/server-fns.json` still says `posts::list_*`. That reddens
  `cargo xtask check` two ways — the "no e2e flow drives this server fn" verdict
  (`snapshot.rs:146-149`) and the xtask unit test
  `seed_capture_covers_the_committed_snapshots_fns`
  (`server_fn_coverage_check.rs:353-368`, run by `host_tests.rs:12-17`). So
  regeneration lands **immediately after** the move, not at the end.
- **Tasks 6–11 are a migration.** The enumerator accepts _both_ attribute
  spellings from Task 6 until Task 11 removes the old one. Task 11 is what
  finishes the migration; it is not cleanup.
- **Partial conversion is invisible to the gates** — they fail open on an empty
  enumeration, which is exactly why Task 11 adds AC-15's count assertion.
- **`timeline` gains a compile-time dependency on `posts`**
  (`timeline_post_summary`, re-exported in Task 8). Decided in the spec; do not
  re-litigate mid-task.
- **The one step no amount of reading can predict is `steps::nix::coverage`.**
  Tasks 4, 5 and 7 each change _what is coverage-measured_: a new `macros`
  module, a `cov:ignore` block, and 50 fn bodies whose boundary wrapper moves
  from a `macro_rules!` expansion to a **proc-macro** expansion. How llvm-cov
  attributes proc-macro-generated code is not knowable statically. If coverage
  moves unexpectedly at Task 7, that is the cause — investigate before reaching
  for `cov:ignore`, and if a marker really is needed it is a reviewable decision
  (`CONTRIBUTING.md:539`), not a silent fix.

---

## Task 1: File the `linkme` follow-up issue

**Files:** none in-tree.

**Interfaces:** Produces: an issue number, cited in Task 13's ADR-0066 edit.

- [x] **Step 1: File it** via **jaunder-issues** in `jaunder-org/jaunder`.

Title:
`xtask/web: retire the registrar gate via linkme auto-registration now that a wrapper macro exists`

Body must state: ADR-0066:28-29 rejected alternative B ("auto-register … **via a
wrapper attribute macro in the `macros` crate**") _because that macro did not
exist_; `#[macros::server]` now does. In scope there: whether
`server_fn_registrar_check.rs` and the hand-maintained list at
`server/tests/helpers/mod.rs:33-88` can both be retired. Note the registrar gate
is currently a uniqueness guard (spec R5), so retiring it must not remove that
guarantee.

Labels: `tooling`, `web`. Not blocked — it is independent.

- [x] **Step 2: Record the number** — filed as **#731**
      (https://github.com/jaunder-org/jaunder/issues/731), added to Jaunder
      Backlog (#1). Cited in Task 13, Step 4.

_No commit — this task touches no files._

---

## Task 2: P1 — prove the instrument span is in scope at the boundary event

**This is the precondition. If Step 2 fails, STOP and report — the spec is void
and the fallback is building #714's comparing gate.**

Deliberately macro-independent: a hand-written `#[tracing::instrument]` fn
exercises the question, so this runs before the macro exists and is not subject
to the placement rule.

**Files:** Modify/Test: `web/src/error.rs` (the existing
`#[cfg(test)] mod tests`)

**Interfaces:**

- Consumes: `web::error::server_boundary(server_fn: &'static str, future)` — the
  current signature; Task 3 changes it.
- Produces: `ScopeRecorder`, reused by Task 10's AC-5 test.

- [ ] **Step 1: Write the test.**

The existing
`server_boundary_evaluates_tracing_fields_when_subscriber_is_active`
(`web/src/error.rs:296`) installs `fmt().with_test_writer()` and asserts only on
the returned `WebError` — it captures nothing about spans, and no span-recording
layer exists in the tree. Add:

```rust
/// P1 (#714): the ADR-0011 span must be in scope when the boundary logs a failure.
/// The whole "delete the `boundary!` label" design rests on the span — not the
/// label — carrying the failing fn's identity. If this fails, the label is not
/// redundant and the spec is void.
#[cfg(feature = "server")]
#[tokio::test]
async fn boundary_failure_event_carries_the_enclosing_instrument_span() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    /// Records the span-scope names present on each event.
    struct ScopeRecorder(Arc<Mutex<Vec<Vec<String>>>>);

    impl<S> Layer<S> for ScopeRecorder
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
            let names = ctx
                .event_scope(event)
                .map(|scope| {
                    scope.from_root().map(|s| s.metadata().name().to_string()).collect()
                })
                .unwrap_or_default();
            self.0.lock().expect("scope recorder mutex").push(names);
        }
    }

    // A stand-in for a `#[server]` fn: same attribute, same boundary call.
    #[tracing::instrument(name = "web.example.do_thing")]
    async fn do_thing() -> WebResult<()> {
        server_boundary("do_thing", async {
            Err(InternalError::server(OuterError { source: SourceError }))
        })
        .await
    }

    let events: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
    let subscriber = tracing_subscriber::registry().with(ScopeRecorder(Arc::clone(&events)));
    let _guard = tracing::subscriber::set_default(subscriber);

    assert!(do_thing().await.is_err(), "the fixture must take the failure path");

    let recorded = events.lock().expect("scope recorder mutex").clone();
    assert!(
        recorded.iter().any(|scope| scope.iter().any(|n| n == "web.example.do_thing")),
        "the boundary failure event must be emitted inside the instrument span; \
         recorded scopes: {recorded:?}"
    );
}
```

- [ ] **Step 2: Run it.**

Run:
`devtool run -- cargo nextest run -p web --features server boundary_failure_event_carries_the_enclosing_instrument_span`

Expected: **PASS.** This is a _probe_, not red-green — it asserts a property of
current code. A FAIL means P1 is false: **stop the plan** and report.

- [ ] **Step 3: Commit.**

```bash
cargo xtask check
git add web/src/error.rs
git commit -m "test(web): pin that the boundary failure event carries its instrument span (#714)"
```

---

## Task 3: Delete the label

Delivers AC-6 on its own, before the macro exists. Task 2 has just proved the
span carries the identity the label duplicated.

**Files:**

- Modify: `host/src/error.rs:302-330` (and its test at `:599-621`)
- Modify: `web/src/error.rs:114-128` (and five tests passing `"test_fn"`)
- Modify: `web/src/lib.rs:9-19`
- Modify: the 15 files holding the 55 `boundary!` call sites

**Interfaces — Produces:**

- `web::error::server_boundary<T>(future: impl Future<Output = InternalResult<T>>) -> WebResult<T>`
- `host::error::InternalError::emit_boundary_failure(&self)`
- `boundary!({ … })` — one argument. Task 11 deletes it.

- [ ] **Step 1: Change both signatures and the macro.**

`host/src/error.rs`: drop the `server_fn: &'static str` parameter and the
`server_fn,` field from the `emit!` field list; the other five fields stay. The
metric (`host/src/error.rs:330`) takes only kind/class, so no metric dimension
is lost.

`web/src/error.rs`: drop the parameter; the call becomes
`error.emit_boundary_failure();`.

`web/src/lib.rs`:

```rust
#[macro_export]
macro_rules! boundary {
    ($body:block) => {
        $crate::error::server_boundary(async move $body).await
    };
}
```

Rewrite its doc comment: it must no longer mention `$name`, and its current
claim that the label feeds "the error metric" (`web/src/lib.rs:12-13`) is
already stale — the metric never took it. State that the failing fn is
identified by the enclosing ADR-0011 span, pinned by Task 2's test.

- [ ] **Step 2: Update all 55 call sites.** `boundary!("<ident>", {` →
      `boundary!({`.

**Two are not mechanical:** `posts::create` (`web/src/posts/api.rs:173-174`) and
`posts::update` (`:320-321`) destructure `CreateArgs`/`UpdateArgs` _before_ the
wrapper. Leave those `let` statements exactly where they are — only the macro
call changes.

Update the tests passing a label: `web/src/error.rs`
(`:250, :305, :352, :457, :464, :474`) and `host/src/error.rs:614,616,621`.

- [ ] **Step 3: Verify.**

Run: `rg -n 'boundary!\("' web/src` → **no matches.** Run:
`rg -n 'server_fn' host/src/error.rs` → **no matches.**

- [ ] **Step 4: Run the suites.**

Run: `devtool run -- cargo nextest run -p web -p host --features server` → PASS.

- [ ] **Step 5: Commit.**

```bash
cargo xtask check
git add host/src/error.rs web/src
git commit -m "refactor(web): drop the boundary! label; the span already names the fn (#714)"
```

---

## Task 4: `macros` — `syn` `full` and the pure core

**Files:**

- Modify: `macros/Cargo.toml:11`
- Create: `macros/src/server_fn.rs`
- Modify: `macros/src/lib.rs` (add `mod server_fn;`)

**Interfaces — Produces (Task 5 calls exactly this):**

```rust
pub(crate) struct Derived {
    pub endpoint: String,                // "/audiences/rename"
    pub span_name: String,               // "web.audiences.rename"
    pub server_args: Vec<syn::Meta>,     // forwarded to #[server]
    pub instrument_args: Vec<syn::Meta>, // forwarded to #[tracing::instrument]
}

pub(crate) fn derive(
    file: &str,
    ident: &syn::Ident,
    args: &[syn::Meta],
) -> Result<Derived, syn::Error>;
```

`file` is a parameter, never read from `proc_macro` here — that is what makes
every branch reachable from `cargo test` (spec, "Macro structure").

- [ ] **Step 1: Add the feature.** `macros/Cargo.toml:11` →
      `syn = { workspace = true, features = ["full"] }`. Required: root
      `Cargo.toml:98` is `syn = "2"` with no features, and syn 2's defaults
      exclude `full`, which parsing an `ItemFn` and rewriting its `Block` needs.

- [ ] **Step 2: Write the failing tests** in `macros/src/server_fn.rs`'s
      `#[cfg(test)]     mod tests` — one per branch.

```rust
use syn::parse_quote;

fn ident(s: &str) -> syn::Ident { syn::Ident::new(s, proc_macro2::Span::call_site()) }

#[test]
fn derives_endpoint_and_span_name_from_path_and_ident() {
    let d = derive("web/src/audiences/api.rs", &ident("rename"), &[]).expect("derives");
    assert_eq!(d.endpoint, "/audiences/rename");
    assert_eq!(d.span_name, "web.audiences.rename");
}

#[test]
fn a_remapped_path_prefix_does_not_change_the_vertical() {
    let d = derive("/build/src-abc123/web/src/audiences/api.rs", &ident("rename"), &[])
        .expect("derives");
    assert_eq!(d.endpoint, "/audiences/rename");
}

#[test]
fn forwards_input_to_server_and_skip_all_to_instrument() {
    let args: Vec<syn::Meta> = vec![parse_quote!(input = MultipartFormData), parse_quote!(skip_all)];
    let d = derive("web/src/media/api.rs", &ident("upload"), &args).expect("derives");
    assert_eq!(d.server_args.len(), 1);
    assert_eq!(d.instrument_args.len(), 1);
}

#[test]
fn forwards_a_skip_list_to_instrument() {
    let args: Vec<syn::Meta> = vec![parse_quote!(skip(name))];
    let d = derive("web/src/audiences/api.rs", &ident("rename"), &args).expect("derives");
    assert_eq!(d.instrument_args.len(), 1);
    assert!(d.server_args.is_empty());
}

#[test]
fn rejects_a_passed_endpoint() {
    let args: Vec<syn::Meta> = vec![parse_quote!(endpoint = "/x/y")];
    let e = derive("web/src/audiences/api.rs", &ident("rename"), &args).unwrap_err();
    assert!(e.to_string().contains("endpoint"), "{e}");
}

#[test]
fn rejects_a_passed_name() {
    let args: Vec<syn::Meta> = vec![parse_quote!(name = "web.x.y")];
    let e = derive("web/src/audiences/api.rs", &ident("rename"), &args).unwrap_err();
    assert!(e.to_string().contains("name"), "{e}");
}

#[test]
fn rejects_fields_because_the_pii_allowlist_no_longer_checks_it() {
    let args: Vec<syn::Meta> = vec![parse_quote!(fields(who = "x"))];
    let e = derive("web/src/audiences/api.rs", &ident("rename"), &args).unwrap_err();
    assert!(e.to_string().contains("fields"), "{e}");
}

#[test]
fn rejects_an_unrecognized_key() {
    let args: Vec<syn::Meta> = vec![parse_quote!(ret)];
    let e = derive("web/src/audiences/api.rs", &ident("rename"), &args).unwrap_err();
    assert!(e.to_string().contains("ret"), "{e}");
}

#[test]
fn rejects_a_path_with_no_web_src_marker() {
    let e = derive("server/src/lib.rs", &ident("rename"), &[]).unwrap_err();
    assert!(e.to_string().contains("web/src"), "{e}");
}

#[test]
fn rejects_a_fn_directly_under_web_src() {
    let e = derive("web/src/mail.rs", &ident("send"), &[]).unwrap_err();
    assert!(e.to_string().contains("vertical"), "{e}");
}

#[test]
fn rejects_a_nested_submodule() {
    // The case that makes (vertical, ident) lossy — spec "Placement rule".
    let e = derive("web/src/posts/api/listing.rs", &ident("list_by_tag"), &[]).unwrap_err();
    assert!(e.to_string().contains("api.rs"), "{e}");
}
```

- [ ] **Step 3: Run, verify failure.**

Run: `devtool run -- cargo nextest run -p macros server_fn` Expected: FAIL —
`derive` / `Derived` not defined.

- [ ] **Step 4: Implement `derive`.** Every branch is pinned above. Two rules
      the tests cannot express: - The vertical is taken **after** the `web/src/`
      marker via `split_once("web/src/")`, mirroring
      `xtask/src/web_server_fns.rs:224-226`, so a `--remap-path-prefix` build
      agrees with a host build. Never assume an absolute path. - The accepted
      shape is exactly `<vertical>/api.rs`: one segment then `api.rs`. Longer →
      the nested-submodule error; shorter → the no-vertical error.

- [ ] **Step 5: Run, verify pass.** Same command. Expected: PASS (11 tests).

- [ ] **Step 6: Commit** — `git add` the new file **before** the gate (Nix
      source is git-tracked-only).

```bash
git add macros/Cargo.toml macros/src/server_fn.rs macros/src/lib.rs
cargo xtask check
git commit -m "feat(macros): derive server-fn endpoint and span name from path and ident (#714)"
```

The `cargo xtask check` here also exercises the Nix build with the changed `syn`
features — that is the AC-17 obligation to re-check the shared vendor, not an
assumption. If the vendor derivation rebuilds, that is expected; if it _fails_,
the feature change is the cause.

---

## Task 5: `macros` — the attribute shell; `web` depends on `macros`

**Files:** Modify `macros/src/lib.rs`, `macros/src/server_fn.rs`,
`web/Cargo.toml`

**Interfaces — Produces:** `#[macros::server]`, `#[macros::server(skip_all)]`,
`#[macros::server(skip(name))]`,
`#[macros::server(input = MultipartFormData, skip_all)]`.

- [ ] **Step 1: Add the dependency.** `web/Cargo.toml` gains
      `macros = { path = "../macros" }` (the form at `common/Cargo.toml:15`). No
      `unused_crate_dependencies` lint is configured, so it being briefly unused
      is fine.

- [ ] **Step 2: Write the failing tests** in `macros/src/server_fn.rs`:

```rust
#[test]
fn expands_to_absolute_attribute_paths_in_order_with_a_wrapped_body() {
    let f: syn::ItemFn = parse_quote! {
        pub async fn rename(name: AudienceName) -> WebResult<()> { do_it().await }
    };
    let out = expand("web/src/audiences/api.rs", &[parse_quote!(skip(name))], f)
        .expect("expands")
        .to_string();

    // Absolute paths — attribute macros are not path-hygienic (AC-11).
    let server_at = out.find(":: leptos :: server").expect("emits ::leptos::server");
    let instr_at = out.find(":: tracing :: instrument").expect("emits ::tracing::instrument");
    // Order is load-bearing: #[server] must be OUTERMOST so it relocates the
    // instrumented body (spec "Boundary", assumptions 1 and 2).
    assert!(server_at < instr_at, "::leptos::server must precede ::tracing::instrument: {out}");

    assert!(out.contains(r#"endpoint = "/audiences/rename""#), "{out}");
    assert!(out.contains(r#"name = "web.audiences.rename""#), "{out}");
    assert!(out.contains("skip (name)"), "{out}");
    assert!(out.contains("crate :: error :: server_boundary"), "{out}");
    assert!(out.contains("async move"), "{out}");
}

#[test]
fn expand_propagates_a_derive_error() {
    let f: syn::ItemFn = parse_quote! { pub async fn x() -> WebResult<()> { y().await } };
    assert!(expand("web/src/posts/api/listing.rs", &[], f).is_err());
}
```

- [ ] **Step 3: Run, verify failure.**

Run: `devtool run -- cargo nextest run -p macros expand` Expected: FAIL —
`expand` not defined.

- [ ] **Step 4: Implement.**

```rust
pub(crate) fn expand(
    file: &str,
    args: &[syn::Meta],
    f: syn::ItemFn,
) -> Result<proc_macro2::TokenStream, syn::Error>;
```

Specified by the tests, plus one rule they cannot express: the original block is
wrapped as `crate::error::server_boundary(async move #block).await` and becomes
the fn's sole statement. Any statements that preceded the old `boundary!` are
inside `#block` already, because Task 7 moves them in when it converts.

Then the shell — the only proc-macro-context code in the crate:

```rust
/// Declares a jaunder `#[server]` fn: derives the wire endpoint, the ADR-0011 span
/// name, and the error boundary from the file path and the fn ident.
///
/// Accepts `input = …` (forwarded to `#[server]`) and `skip(…)` / `skip_all`
/// (forwarded to `#[tracing::instrument]`). Everything else is a hard error.
#[proc_macro_attribute]
pub fn server(args: TokenStream, item: TokenStream) -> TokenStream {
    // cov:ignore-start — the only proc-macro-context code in this crate:
    // `Span::call_site().file()` panics outside a live expansion, so no
    // `cargo test` can reach this. All logic lives in `server_fn::{derive, expand}`,
    // which take the path as a parameter and are unit-tested. (#714)
    let file = proc_macro::Span::call_site().file();
    let parsed = syn::parse_macro_input!(args with
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated);
    let f = syn::parse_macro_input!(item as syn::ItemFn);
    match server_fn::expand(&file, &parsed.into_iter().collect::<Vec<_>>(), f) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
    // cov:ignore-stop
}
```

- [ ] **Step 5: Run, verify pass.** `devtool run -- cargo nextest run -p macros`
      → PASS.

- [ ] **Step 6: Commit.**

```bash
cargo xtask check
git add macros/src web/Cargo.toml
git commit -m "feat(macros): the #[macros::server] attribute shell (#714)"
```

---

## Task 6: All four gate consumers tolerate the new spelling

**Four gates share one enumeration.** Conversion cannot start until every
consumer of `web_server_fns::server_fns_in` handles `#[macros::server]`;
otherwise Task 7's commit fails `cargo xtask check`. The endpoint gate is the
sharpest case: `endpoint_of` returns `Ok(None)` for the new attribute and
`:105-111` hard-errors on that, while `Mode::Fix` **never** synthesizes an
endpoint (`:14`).

**Files:**

- Modify: `xtask/src/web_server_fns.rs` (predicate + `WebServerFn`)
- Modify: `xtask/src/steps/server_fn_registrar_check.rs:102-110`
- Modify: `xtask/src/steps/server_fn_tracing_check.rs`
- Modify: `xtask/src/steps/server_fn_endpoint_check.rs`
- Modify: `xtask/src/server_fns.rs:69-73`

**Interfaces — Produces:** `WebServerFn::uses_macro_attr: bool`, true when the
attribute path's last segment came from `macros::server`. Tasks 7–11 branch on
it.

- [ ] **Step 1: Write the failing tests.**

`web_server_fns.rs`:

```rust
#[test]
fn enumerates_both_attribute_spellings() {
    let src = "#[macros::server]\npub async fn a() {}\n\
               #[server(endpoint = \"/v/b\")]\npub async fn b() {}\n";
    let fns = server_fns_in(src).expect("parses");
    assert_eq!(fns.len(), 2);
    assert!(fns[0].uses_macro_attr);
    assert!(!fns[1].uses_macro_attr);
}
```

`server_fn_registrar_check.rs` (AC-3 — today `skip_all`/`skip(…)` hit the
`:82-89` hard error because `server_fn_default_named` demands every arg be
`Meta::NameValue`):

```rust
#[test]
fn instrument_args_on_the_macro_attribute_are_not_a_positional_rename() {
    let sources = vec![(
        "web/src/audiences/api.rs".to_string(),
        "#[macros::server(skip_all)]\npub async fn create() {}\n\
         #[macros::server(skip(name))]\npub async fn rename() {}\n".to_string(),
    )];
    let registrar = wrap_reg(
        "server_fn::axum::register_explicit::<web::audiences::Create>();\n\
         server_fn::axum::register_explicit::<web::audiences::Rename>();",
    );
    assert!(problems(&sources, &registrar).is_none());
}
```

`server_fn_endpoint_check.rs`:

```rust
#[test]
fn a_macro_attributed_fn_is_not_missing_an_endpoint() {
    // The macro derives the endpoint; there is no attribute argument to check,
    // and Mode::Fix must not try to write one.
    let src = "#[macros::server]\npub async fn create() {}\n";
    assert!(problems(&[("web/src/audiences/api.rs".into(), src.into())]).is_none());
}
```

`server_fn_tracing_check.rs` (AC-14 — the retained PII rules must read
`skip`/`skip_all` from the new attribute):

```rust
#[test]
fn pii_rules_read_arguments_from_the_macro_attribute() {
    // `Secret` is not on RECORDABLE_TYPES, so an unskipped one must fail...
    let src = "#[macros::server]\npub async fn a(token: Secret) {}\n";
    assert!(problems(&[("web/src/v/api.rs".into(), src.into())]).is_some());
    // ...and skipping it via the new attribute must pass.
    let ok = "#[macros::server(skip(token))]\npub async fn a(token: Secret) {}\n";
    assert!(problems(&[("web/src/v/api.rs".into(), ok.into())]).is_none());
}
```

`server_fns.rs` (the coverage inventory — `endpoint` is stored
**leading-slash-stripped**, `:24-27`, and compared as
`format!("{vertical}/{ident}")` at `snapshot.rs:121`):

```rust
#[test]
fn a_macro_attributed_fn_gets_its_endpoint_computed() {
    let src = "#[macros::server]\npub async fn create() {}\n";
    let fns = inventory_for("web/src/audiences/api.rs", src);
    assert_eq!(fns[0].endpoint.as_deref(), Some("audiences/create")); // no leading slash
}
```

- [ ] **Step 2: Run, verify failure.**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: FAIL on the five new tests.

- [ ] **Step 3: Implement.**

- `web_server_fns.rs`: match on the attribute path's **last segment** being
  `server`, so both `server` and `macros::server` are enumerated (`is_ident` is
  false for the two-segment form — `:96`). Record `uses_macro_attr`.
- `server_fn_default_named`: consider only arguments routed to `#[server]`.
  `input = …` is the sole routed argument and _is_ `Meta::NameValue`, so
  positional-rename detection is preserved, not defeated.
- `server_fn_endpoint_check`: skip `uses_macro_attr` fns entirely — no presence
  check, no `Mode::Fix` rewrite.
- `server_fn_tracing_check`: rules 1 (presence/placement — `:335-354`) and 2
  (span name — `:363-374`) **skip** `uses_macro_attr` fns; the per-parameter
  loop (`:376-407`) still runs, sourcing `skip`/`skip_all` from the
  `#[macros::server]` args instead of `parse_instrument(&f.attrs[index])`
  (`:356`).
- `server_fns.rs:72`: for `uses_macro_attr` fns compute `<vertical>/<ident>`
  (**no leading slash**, matching `:24-27`); otherwise keep `endpoint_of`.

- [ ] **Step 4: Run, verify pass.** Same command → PASS.

- [ ] **Step 5: Commit.**

```bash
cargo xtask check
git add xtask/src
git commit -m "feat(xtask): enumerate #[macros::server] across all four server-fn gates (#714)"
```

---

## Task 7: Convert the 50 fns already in `<vertical>/api.rs`

**Files:** the 14 `web/src/<vertical>/api.rs` files (every holder but
`posts/api/listing.rs`). 55 − 5 = 50 fns; 15 − 1 = 14 files.

**Interfaces:** Consumes Task 5's attribute. Produces no signature changes —
generated types and URLs are byte-identical.

- [ ] **Step 1: Convert.** Two attributes become one, and the wrapper goes:

```rust
// before
#[server(endpoint = "/audiences/rename")]
#[tracing::instrument(name = "web.audiences.rename", skip(name))]
pub async fn rename(audience_id: AudienceId, name: AudienceName) -> WebResult<()> {
    boundary!({ … })
}

// after
#[macros::server(skip(name))]
pub async fn rename(audience_id: AudienceId, name: AudienceName) -> WebResult<()> {
    …          // body only — the macro supplies the boundary
}
```

For `posts::create` (`web/src/posts/api.rs:173-174`) and `posts::update`
(`:320-321`) the `let … = args;` destructuring becomes an ordinary first
statement of the body — the macro wraps the whole body, so it lands inside the
`async move` block.

`media::upload` keeps its `input`:
`#[macros::server(input = MultipartFormData, skip_all)]`.

- [ ] **Step 2: Verify the counts.**

Run: `rg -c '#\[macros::server' web/src` → 14 files summing to **50**. Run:
`rg -n '#\[server\(' web/src` → matches only in `posts/api/listing.rs` (5).

- [ ] **Step 3: Build both targets.**

Run: `devtool run -- cargo check -p web --all-features --all-targets` → PASS.
Run:
`devtool run -- cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`
→ PASS.

The wasm run (AC-12) proves the client still discards the wrapped body — spec
"Boundary", assumptions 1–2.

- [ ] **Step 4: Run the server suite.**

Run: `devtool run -- cargo nextest run -p jaunder --test integration` → PASS,
with no test edits: the URLs are unchanged.

- [ ] **Step 5: Commit.**

```bash
cargo xtask check
git add web/src
git commit -m "refactor(web): adopt #[macros::server] across the verticals (#714)"
```

---

## Task 8: The `timeline` move

**Files:**

- Create: `web/src/timeline/api.rs`, `web/src/timeline/server.rs`
- Delete: `web/src/posts/api/listing.rs`
- Modify: `web/src/timeline/mod.rs`, `web/src/posts/mod.rs` (re-export **and**
  module doc), `web/src/posts/api.rs:15-16`,
  `server/src/projector/mod.rs:42-43`, `web/src/posts/component.rs:30`,
  `web/src/cockpit/component.rs:13`, `web/src/home/component.rs:9`,
  `server/tests/helpers/mod.rs:61-65`, `server/tests/web/web_posts.rs`

**Interfaces — Produces:**
`web::timeline::{list_by_user, list_local_timeline, list_home_feed, list_by_tag, list_by_user_and_tag}`
and types `ListByUser`, `ListLocalTimeline`, `ListHomeFeed`, `ListByTag`,
`ListByUserAndTag`; under `feature = "server"`,
`web::timeline::{fetch_user_posts, fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts_by_tag}`.
Also `web::posts::timeline_post_summary`.

- [ ] **Step 1: Re-export the one dependency that does not travel.**

`page_from_rows` calls `crate::posts::server::timeline_post_summary`
(`listing.rs:18,44`); `posts/mod.rs:13` declares `mod server;` **privately** and
`:59` re-exports only `post_response`. In `web/src/posts/mod.rs`:

```rust
#[cfg(feature = "server")]
pub use server::{post_response, timeline_post_summary};
```

- [ ] **Step 2: Create the two files.**

`web/src/timeline/api.rs` — the five fns, each `#[macros::server]`, bodies
unchanged except that `timeline_post_summary` now comes from `crate::posts`.
`web/src/timeline/server.rs` — the four `fetch_*` helpers, the private
`page_from_rows` (`listing.rs:32`), and the file's `mod tests` (`:311-499`).

`web/src/timeline/mod.rs` gains the wiring, matching
`web/src/audiences/mod.rs:17-27`:

```rust
mod api;
#[cfg(feature = "server")]
mod server;

pub use api::{
    list_by_tag, list_by_user, list_by_user_and_tag, list_home_feed, list_local_timeline,
    ListByTag, ListByUser, ListByUserAndTag, ListHomeFeed, ListLocalTimeline,
};
#[cfg(feature = "server")]
pub use server::{
    fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts, fetch_user_posts_by_tag,
};
```

The explicit `pub use api::{…}` is **mandatory, not stylistic**: the registrar
gate requires the path be exactly `web::<vertical>::<Leaf>` and rejects longer
ones because `mod api` is private in every vertical
(`server_fn_registrar_check.rs:170-179`).

Rewrite `timeline/mod.rs`'s module doc (`:5-9`): it claims the vertical is
server-less and that the types come from `crate::posts` — both false. The types
are `common/src/seed.rs:36,62`.

- [ ] **Step 3: Delete the old file and its glob.**

Delete `web/src/posts/api/listing.rs`; delete `mod listing;` and
`pub use listing::*;` (`web/src/posts/api.rs:15-16`); drop the five fns and five
types from `posts/mod.rs`'s `pub use api::{…}` blocks (`:37-52`). Fix
`posts/mod.rs:5-6`, which says the wire types live "in [`api`] (with the
timeline/listing surface in its `listing` submodule)".

- [ ] **Step 4: Update every consumer.**

- `server/src/projector/mod.rs:42-43` →
  `use web::timeline::{fetch_local_timeline, fetch_posts_by_tag, fetch_user_posts, fetch_user_posts_by_tag};`
  (call sites `:204, :258, :294, :327` need no edit).
- `web/src/posts/component.rs:30`, `web/src/cockpit/component.rs:13`,
  `web/src/home/component.rs:9` → import the `list_*` fns from
  `crate::timeline`.
- `server/tests/helpers/mod.rs:61-65` →
  `web::timeline::{ListByUser, ListLocalTimeline, ListHomeFeed, ListByTag, ListByUserAndTag}`.
- `server/tests/web/web_posts.rs` → update `ServerFn::PATH` references in place.
  Do **not** relocate these tests; out of scope.

Leave alone: the xtask **test fixtures** that use `list_home_feed` /
`web::posts::ListHomeFeed` as synthetic source
(`server_fn_registrar_check.rs:455,459`, `server_fn_tracing_check.rs:598`) —
synthetic strings, not references. Also `web/src/timeline/component.rs:72` names
two of the fns in a doc comment only.

- [ ] **Step 5: Build both targets and run the suites.**

Run: `devtool run -- cargo check -p web --all-features --all-targets` Run:
`devtool run -- cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`
Run: `devtool run -- cargo nextest run -p jaunder --test integration`

The wasm clippy run is the **only** thing that catches a missed component-file
import.

- [ ] **Step 6: Commit — expect `cargo xtask check` to fail the coverage step.**

`git add` the new files first. The coverage gate is now red _by construction_:
the live inventory says `timeline::list_*` while `docs/coverage/server-fns.json`
still says `posts::list_*`. Task 9 fixes it, and the two must land as **one
commit** so no committed tree is red.

**Do not commit here.** Proceed directly to Task 9 and commit once, at Task 9
Step 5.

---

## Task 9: e2e → regenerate — restores a green `check`

Completes Task 8. The move re-keys five fns, which reddens `cargo xtask check`
two ways: the "no e2e flow drives this server fn" verdict
(`snapshot.rs:146-149`) and the xtask unit test
`seed_capture_covers_the_committed_snapshots_fns`
(`server_fn_coverage_check.rs:353-368`, run via `host_tests.rs:12-17`).

- [ ] **Step 1: Produce a capture.**

Run: `devtool run -- cargo xtask e2e sqlite chromium` (foreground,
`timeout: 600000`).

**Expect a non-zero exit.** `Command::E2e` appends
`server_fn_coverage_check::verify_after_combo` (`xtask/src/lib.rs:427-431`),
which compares the fresh capture against the not-yet-regenerated snapshot. The
combo itself still runs and still writes
`.xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz`
(`server_fn_coverage/io.rs:25`) — that artifact is all this step needs. Confirm
the tarball exists before continuing; do not treat the non-zero exit as a stop.

- [ ] **Step 2: Regenerate.**

Run: `devtool run -- cargo xtask server-fn-coverage regenerate` Then re-reduce
the seed with `xtask/src/server_fn_coverage/testdata/reduce-otel-capture.mjs`.
**Never hand-edit either file** (`CONTRIBUTING.md:480`); a hand-patched seed
would make Task 12's cross-check assert the rule against itself.

- [ ] **Step 3: Confirm the re-keying.**

`docs/coverage/server-fns.json` has a `covered` section and an `orphans`
section; the five appear in **both**, and both must now read `timeline::…`.
`docs/coverage/server-fns-allowlist.json` holds only `media::delete` and
`sessions::revoke` — confirm it did **not** churn.

- [ ] **Step 4: Verify green.**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml` → PASS.

- [ ] **Step 5: Commit Tasks 8 and 9 together.**

```bash
git add web/src server/src server/tests docs/coverage xtask/src/server_fn_coverage/testdata
cargo xtask check
git commit -m "refactor(web): move the timeline queries into their own vertical (#714)"
```

---

## Task 10: Runtime wire assertions

**Files:** Create `server/tests/web/server_fn_wire.rs`; register it in the `web`
test module. (`server/Cargo.toml:26` has `web` with `features = ["server"]`;
`:77` has `server_fn` as a dev-dependency; the target is
`[[test]] name = "integration"` at `:15-17`.)

- [ ] **Step 1: Write the AC-4 test.**

**Do not assert the list against a sibling list in the same file** — that only
proves the author kept two hand-maintained lists in sync, and forgetting a fn in
both still passes. Assert against the **registrar list**
(`server/tests/helpers/mod.rs:33-88`), the same independent counterpart AC-15
uses.

```rust
//! The wire contract of every `#[macros::server]` fn, read from the generated
//! types rather than from source — so these hold even if xtask's syn enumeration
//! breaks (spec R3/R5).

use server_fn::ServerFn;

/// Every path is exactly `/api/<vertical>/<ident>`, and all are distinct (AC-4).
#[test]
fn every_server_fn_path_is_api_vertical_ident_and_distinct() {
    let mut actual: Vec<String> = Vec::new();
    macro_rules! check {
        ($ty:ty, $vertical:literal, $ident:literal) => {{
            assert_eq!(<$ty as ServerFn>::PATH, concat!("/api/", $vertical, "/", $ident));
            actual.push(<$ty as ServerFn>::PATH.to_string());
        }};
    }
    check!(web::auth::GetSession, "auth", "get_session");
    // … one line per fn, all 55, mirroring server/tests/helpers/mod.rs:33-88 …
    check!(web::timeline::ListByTag, "timeline", "list_by_tag");

    assert_eq!(
        actual.len(),
        crate::helpers::REGISTERED_SERVER_FN_COUNT,
        "every registered server fn must be covered here"
    );

    let mut sorted = actual.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), actual.len(), "server-fn paths must be pairwise distinct");
}
```

Add `pub const REGISTERED_SERVER_FN_COUNT: usize` to
`server/tests/helpers/mod.rs`, derived from the registration block rather than
typed as a literal.

- [ ] **Step 2: Write the AC-5 test** — the derived span name and attribute
      order, driven at runtime, reusing Task 2's `ScopeRecorder`. This is the
      only end-to-end assertion on the derived span name and the only thing
      pinning that `::leptos::server` wraps `::tracing::instrument`; a missing
      span is otherwise invisible (spec AC-10's masking note).

      **It must carry a backend-guard marker or the `test-backend-pattern` step fails.**
      `test_pattern_check.rs:35` scans `server/tests`, and a bare `#[tokio::test]` there
      needs either a backend template or a `// guard:no-backend — <reason>` marker with a
      **non-empty** reason (#419). Write it immediately above the attribute:

```rust
// guard:no-backend — drives a server fn in-process to observe its emitted span;
// touches no database.
#[tokio::test]
async fn a_server_fn_emits_its_derived_span_name() { … }
```

      Precedent: `server/tests/misc/cli_subprocess.rs:17`. The AC-4 test is a plain
      `#[test]` and is exempt (`test_pattern_check.rs:13-14`).

- [ ] **Step 3: Run.**

Run:
`devtool run -- cargo nextest run -p jaunder --test integration server_fn_wire`
→ PASS. A failure means the conversion or the move changed a URL.

- [ ] **Step 4: Commit** (`git add` the new file before the gate).

```bash
git add server/tests
cargo xtask check
git commit -m "test(server): pin every server-fn path and its derived span name (#714)"
```

---

## Task 11: Retire the old world

**Files:** Delete `xtask/src/steps/server_fn_endpoint_check.rs`; modify
`xtask/src/lib.rs:35,342,380`, `xtask/src/web_server_fns.rs:6,13` (doc links)
and its predicate, `xtask/src/steps/server_fn_tracing_check.rs`,
`xtask/src/server_fns.rs`, `web/src/lib.rs`

- [ ] **Step 1: Delete `boundary!`** (`web/src/lib.rs:9-19`). Verify first:
      `rg -n 'boundary!' web/src` → **no matches** (the definition line reads
      `macro_rules! boundary {`, which this pattern does not match, so a clean
      result means every call site is gone). Nothing outside `web/src` uses it.

- [ ] **Step 2: Delete the endpoint gate** and its registry entries, and repoint
      the two intra-doc links at `web_server_fns.rs:6,13` so the `doc_links`
      step passes.

- [ ] **Step 3: Shrink the tracing gate.** Delete rules 1 (`:335-354`) and 2
      (`:363-374`) with their tests and `Mode::Fix` rewriting; delete
      `is_cfg_attr_instrument` (`:318`), its guard and test (`:709-717`); delete
      the `fields(…)` value-allowlist, `IGNORED_ARGS` (`:102`) and the
      `err`/`ret` rejection (`:201-207`); drop `problems_with`'s now-unused
      `vertical` parameter (`:330` — `:345` and `:362` were its only uses).
      **Retain and keep tested:** the per-parameter skipped-or-recordable rule,
      the nameless-parameter rule (`:382-388` — currently **untested**, so add a
      test), and default-deny on unmodelled arguments.

- [ ] **Step 4: Drop the transitional tolerance.** The predicate accepts only
      `macros::server`; delete `uses_macro_attr` and every branch on it. In
      `server_fns.rs`, **delete `endpoint_of` itself** (`:86-102`) along with
      its three tests (`captures_ident_endpoint_and_module`,
      `endpoint_is_found_after_another_argument`,
      `bare_server_attr_has_no_endpoint`) — with the fallback gone it is
      unreachable, and leaving it in place is `dead_code` under `-D warnings`.

- [ ] **Step 4b: Convert every surviving xtask test fixture to the new
      spelling.**

      **This is what makes Step 4 safe, and it is not small.** Every xtask unit test
      builds its input as a source *string* containing `#[server…`. Once the predicate
      accepts only `macros::server`, each such fixture enumerates to **zero** fns and its
      assertion silently flips — the same fail-open the whole design is guarding against,
      reproduced inside the gate's own tests.

      Fixture counts today (`rg -c -F '"#[server' xtask/src`):

      | file | fixtures |
      |---|---|
      | `steps/server_fn_tracing_check.rs` | 25 (minus those deleted with rules 1–2 in Step 3) |
      | `steps/server_fn_registrar_check.rs` | 17 |
      | `web_server_fns.rs` | 11 (the enumerator's own tests) |
      | `server_fns.rs` | 5 (three die with `endpoint_of` in Step 4) |
      | `steps/server_fn_coverage_check.rs` | 2 |
      | `steps/server_fn_endpoint_check.rs` | 8 — **deleted whole** in Step 2 |

      Convert `#[server(endpoint = "/v/x")]` → `#[macros::server]`, and
      `#[server(input = Json, endpoint = "…")]` → `#[macros::server(input = Json)]`.
      Keep at least one fixture per file asserting the **old** spelling is no longer
      enumerated, so the narrowing itself is pinned rather than assumed.

      Also check `steps/proffered_secret_check.rs` — it holds one `#[server` fixture but
      is **not** a consumer of this enumeration, so it likely needs no change; confirm
      rather than assume.

      This is a bulk mechanical sweep across five files — a good candidate for
      **jaunder-dispatch** rather than doing it inline.

- [ ] **Step 4c: Update the module-doc prose** in `server_fns.rs:16` and
      `web_server_fns.rs:29`, both of which still describe the `#[server]`
      spelling. `doc_links` will not catch this; AC-24's intent covers it.

- [ ] **Step 5: Add AC-15's count assertion.**

```rust
/// All three remaining gates fail OPEN on an empty enumeration (`problems()`
/// returns `None` — server_fn_tracing_check.rs:482-484,
/// server_fn_registrar_check.rs:272-274), so a stale attribute predicate would be a
/// silent green across all of them. Compare against the registrar's entry count
/// rather than a literal, which every new server fn would churn and invite
/// blind-bumping.
#[test]
fn the_enumeration_matches_the_registrar_entry_count() { … }
```

`registered_entries` returns a `BTreeSet` (`:138-140`), so this compares against
the **deduped** set — deliberately, since a duplicated `register_explicit` line
is harmless and must not redden the gate.

- [ ] **Step 6: Run.**
      `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml` →
      PASS.

- [ ] **Step 7: Commit.**

```bash
cargo xtask check
git add xtask/src web/src/lib.rs
git commit -m "refactor(xtask): retire the endpoint gate and the derived-name rules (#714)"
```

---

## Task 12: Coverage — dead branches and the seed cross-check

**Files:** `xtask/src/server_fn_coverage/snapshot.rs:118-134`,
`xtask/src/steps/server_fn_coverage_check.rs`,
`xtask/src/server_fn_coverage/extract.rs` (doc only)

- [ ] **Step 1: Delete the two vacuous drift branches** in `verdict`
      (`snapshot.rs:122-131`) — the `None` → "bare `#[server]` with no
      `endpoint`" branch and the `Some(ep)` ≠ derived branch — with their tests
      (`:316-327`). Both are unreachable now that the endpoint is always
      computed.

- [ ] **Step 2: Record the masking note (AC-10's second half).** In
      `extract.rs`'s module doc, state that `identify` (`:111-136`) returns on
      the span-name + `code.namespace` signal and only falls through to `uri` on
      a miss — so now that every fn carries a span, the URI signal is masked at
      runtime and a wrong computed endpoint would **not** show as lost coverage.
      The endpoint is kept as defence in depth (ADR-0081's lesson), and Step 3
      is its only live verification.

- [ ] **Step 3: Write the anti-drift test (AC-9).**

**Establish seed-presence by span name + `code.namespace`, never by the endpoint
being checked.** Matching computed endpoints against seed URIs and skipping the
misses makes the drift case _identical to_ the skipped case — the failure
`extract.rs:16-22` records having already happened. Then assert the endpoint
observed for that span equals the computed one, **and assert the matched count
is exactly 53**, so a silent shrink fails loudly. The seed was regenerated in
Task 9, so this passes now.

- [ ] **Step 4: Run.**
      `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml` →
      PASS.

- [ ] **Step 5: Commit.**

```bash
cargo xtask check
git add xtask/src
git commit -m "feat(xtask): cross-check computed server-fn endpoints against the seed capture (#714)"
```

---

## Task 13: ADRs, docs, and the final gate

**Files:** `docs/adr/0082`, `0011`, `0070`, `0066`, `0013`, `0016`, `0056`,
`0059`, `0065`, `docs/web-style-guide.md`, `CONTRIBUTING.md`

Each edit is specified by its AC; invent no scope beyond them.

- [ ] **Step 1: ADR-0082** per **AC-20** — `:55-68`, `:81-86`, `:101-107`, the
      rejected-alternatives additions (`DISABLE_SERVER_FN_HASH`,
      `SERVER_FN_MOD_PATH`, why #698's variant is unsafe post-#684), and the
      greppability consequence.
- [ ] **Step 2: ADR-0011** per **AC-21**.
- [ ] **Step 3: ADR-0070** per **AC-22** — `timeline` is no longer server-less;
      add the placement rule as a tightening of §1; correct
      `web/src/timeline/mod.rs:5-9`'s attribution of the types to
      `crate::posts`.
- [ ] **Step 4: ADR-0066** per **AC-23** — `:103-107` and `:114`. Cite **#731**
      where the registrar's future is discussed.
- [ ] **Step 5: The doc sweep** per **AC-24** — including
      `docs/web-style-guide.md:181-183`, which names `timeline/` as the
      canonical server-less vertical that "omits `api.rs` too", and recording
      the placement rule in **both** `CONTRIBUTING.md` and
      `docs/web-style-guide.md:171-215`.
- [ ] **Step 6: Prettier only what changed.**

Run `prettier -w` on the specific files edited above — not the whole tree, which
would pull unrelated reformatting into this commit.

- [ ] **Step 7: Commit.**

```bash
cargo xtask check
git add docs CONTRIBUTING.md
git commit -m "docs: record the macro-derived server-fn contract (#714, #722, #698)"
```

- [ ] **Step 8: The full local gate (AC-18).**

Run **after** Step 7's commit — `Command::Validate` runs `clean_tree_precheck`
and **returns early, blocked**, on a dirty tree (`xtask/src/lib.rs:364-372`).

Run: `devtool run -- cargo xtask validate --no-e2e` (foreground,
`timeout: 600000`) → PASS.

---

## Closing

**AC-25** (closing #714, #722, #698) is **jaunder-ship**'s job, not a plan task.

Before ship, confirm the spec's two build-time deferrals resolved during the
work: whether leptos re-exports at `::leptos::server` or
`::leptos::prelude::server` (AC-11), and that the Nix toolchain is ≥1.88 with
`--remap-path-prefix` preserving the `web/src/` marker (R2). Neither can be
settled by reading.
