# Plan — #505: `Topbar` title/sub props as `TextProp`

Spec:
[2026-07-17-issue-505-topbar-textprop.md](../specs/2026-07-17-issue-505-topbar-textprop.md)

## Review header

- **Goal:** Replace `Topbar`'s `Signal<String>` text props with
  `leptos::TextProp` so static call sites pass bare `&'static str` literals,
  eliminating ~24 `.to_string()` wrappers that exist only to satisfy the prop
  type.
- **Scope (in):** the two prop type annotations in `web/src/ui/topbar.rs`; the
  `.to_string()` cleanup at every `<Topbar …>` call site across 13 `web/src`
  files.
- **Scope (out):** the server projector `render_topbar` and its parity tests
  (untouched — see spec "Non-goals"); any behavior change; any new tests.
- **Tasks:** (1) swap prop types in `topbar.rs`; (2) sweep call sites to drop
  `.to_string()`; (3) verify build/gate + browser-drive reactive titles.
- **Key risks/decisions:**
  - The three _reactive_ callers (`posts.rs:465` bare closure, `posts.rs:1225` /
    `1421` `Signal::derive`) must keep updating — `TextProp`'s `From<Fn>` and
    `From<Signal>` impls cover them (verified, spec "Decision"). They only lose
    a `.to_string()` on their _static_ `sub=` arg.
  - This is a refactor over existing coverage: the compiler (all call sites must
    still convert) and the unchanged
    `topbar_*_matches_reactive_component_markup` tests are the regression guard;
    no red-green TDD step applies.
  - `web` is a dual-target (host + wasm) crate — verify with
    `--all-features --all-targets` reach, which `cargo xtask check` provides.

## For agentic workers

Execute with **`jaunder-iterate`**; the call-site sweep (Task 2) is a good
candidate to delegate via **`jaunder-dispatch`** so the file bulk stays out of
the driver's context. Tick checkboxes in real time. Single logical change →
**one commit** at the end (Task 3), not per-file.

## Global constraints

- No `Co-Authored-By` trailer on the commit.
- Pre-commit hook runs the full `cargo xtask check`; run it green _before_
  committing (`jaunder-commit`). Do not edit tracked files while the gated
  commit is building.
- Preserve exact user-visible strings (including the non-ASCII `·` in
  `home.rs:66`) — only the `.to_string()` call is removed, never the literal.

---

## Task 1 — Swap `Topbar` prop types to `TextProp`

**File:** `web/src/ui/topbar.rs`

**Change** the two prop annotations in `pub fn Topbar` (lines 28–29):

```rust
#[prop(into)] title: TextProp,
#[prop(optional, into)] sub: Option<TextProp>,
```

- The component **body is unchanged**: `{move || title.get()}` and
  `{move || s.get()}` still compile — `TextProp::get()` returns
  `Oco<'static, str>`, which renders as a text node identically to the prior
  `Signal<String>::get()`.
- **No import change** — `TextProp` is re-exported through the existing
  `use leptos::prelude::*;` (leptos `text_prop::*`, verified).
- Leave `render_topbar` and the `#[cfg(test)]` module untouched.

**Check:** `cargo check -p web` — the component compiles; call sites still
passing `"…".to_string()` (a `String`) also compile via `From<String>`, so
nothing breaks mid-refactor.

## Task 2 — Drop `.to_string()` at every call site

Sweep the 13 files below; in each `<Topbar …>` invocation remove the
`.to_string()` suffix from `title=` / `sub=` string-literal arguments, leaving
the bare literal (`title="Posts"`). Do **not** touch reactive args (bare
closures, `Signal::derive`).

| File                              | `<Topbar>` lines                                                         |
| --------------------------------- | ------------------------------------------------------------------------ |
| `web/src/audiences/mod.rs`        | 292                                                                      |
| `web/src/pages/posts.rs`          | 35, 697, 942; `sub` only on 465 & 1225; **none** on 1421 (both reactive) |
| `web/src/pages/invites.rs`        | 20                                                                       |
| `web/src/pages/email.rs`          | 18, 84                                                                   |
| `web/src/pages/auth.rs`           | 55, 157, 228                                                             |
| `web/src/pages/password_reset.rs` | 20, 79                                                                   |
| `web/src/pages/home.rs`           | 64 (title 65 + sub 66 — keep the `·`)                                    |
| `web/src/pages/profile.rs`        | 22                                                                       |
| `web/src/pages/sessions.rs`       | 16                                                                       |
| `web/src/pages/media.rs`          | 31                                                                       |
| `web/src/pages/site.rs`           | 16                                                                       |
| `web/src/pages/cockpit.rs`        | 83, 90                                                                   |
| `web/src/pages/backup.rs`         | 18                                                                       |

**Check (acceptance grep):** `rg -n 'Topbar' -A3 web/src` then confirm
`rg -n '(title|sub)="[^"]*"\.to_string\(\)' web/src` returns **no matches**.

## Task 3 — Verify and commit

1. `cargo xtask check` (host static + clippy + Nix coverage/tests) → green. This
   compiles `web` across features/targets; a leftover `.to_string()` mismatch or
   an unconvertible call site would fail here.
2. **Browser-drive the reactive titles** (spec acceptance) via the `verify` /
   e2e path: load a user timeline page (`posts.rs:465` — title `Posts by …`) and
   a tag feed (`posts.rs:1225`/`1421` — title `#tag`) and confirm the heading
   text renders and reflects the reactive value.
3. Commit the whole change as one commit (message references #505), no
   `Co-Authored-By` trailer, via `jaunder-commit`.

## Self-review

- Every `<Topbar` file from `rg -l` is in the Task 2 table. ✅
- Reactive callers identified and explicitly excluded from title edits;
  `posts.rs:1421` has no `.to_string()` to remove (both args reactive). ✅
- No new files, no test changes, no ADR, no follow-up issues to file. ✅
- Parity contract for `render_topbar` preserved (untouched). ✅
