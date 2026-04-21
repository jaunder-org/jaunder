# UI Import Plan

Import the `build/` static-site design system into Jaunder as Leptos components,
wired to real data and structured for long-term extensibility (theme switching,
user-uploadable stylesheets).

## Scope

Screens to port: **timeline** (authenticated home feed + unauthenticated landing),
**compose** (new post), and any screen covered by an existing Leptos page.
The **settings** screen is explicitly out of scope for this phase.

## Architecture decisions

### CSS

`jaunder.css` and `jaunder-themes.css` are **embedded in the binary** using
`rust-embed`, served via dedicated Axum handlers, and loaded via `<link>` tags in
the HTML shell. They are not processed by cargo-leptos. Rationale:

- **Single-binary distribution.** `rust-embed` compiles files into the binary at
  build time (in release mode; debug mode reads from disk for fast iteration). No
  files need to be present on the target filesystem.
- **Theme switching is attribute-based.** The `data-theme="terminal|studio|reader"`
  CSS variable packs mean switching themes requires only changing one HTML attribute;
  both theme files are always present in the binary and always loaded.
- **User-uploadable stylesheets are architecturally distinct.** They are served
  dynamically from storage (a separate `<link>` injected by the server per-request).
  Keeping base + theme sheets as separately addressable HTTP resources preserves this
  clean separation and allows independent browser caching.
- **cargo-leptos adds no value here.** The CSS has no Rust-specific preprocessing
  needs; bundling it couples the design system to the Rust build cycle and obscures
  its provenance.

The crates used are `rust-embed` (file embedding) and `axum-embed` (Axum handler
integration with ETag and conditional-request support out of the box).

Load order in the HTML shell:

```html
<link rel="stylesheet" href="/style/jaunder.css">
<link rel="stylesheet" href="/style/jaunder-themes.css">
<!-- future: <link rel="stylesheet" href="/api/users/{id}/stylesheet"> -->
```

The Leptos-compiled `/pkg/jaunder.css` remains for any minimal framework-specific
needs; it will likely be empty or near-empty after this migration.

### Theme context

A `RwSignal<Theme>` is provided at the app root via Leptos context. `Theme` is an
infallible enum (`Terminal | Studio | Reader`). The signal drives a reactive
`data-theme` attribute on the outermost shell element. On WASM, the value is
persisted to and restored from `localStorage`. No UI for changing the theme is
built in this phase, but the full runtime infrastructure is in place.

### Layout

The current `App` structure (`<header><HeaderNav /></header><main><Routes /></main>`)
is replaced with a **nested-route shell layout**:

```
App
└── AppShell  (ParentRoute at "/")
    ├── j-shell  (div with data-theme, flex row)
    │   ├── Sidebar  (sticky left nav, present on every screen)
    │   └── j-main  (scrollable content area)
    │       └── <Outlet />  ← individual page routes render here
    └── auth-only routes nested inside, fallback for public routes
```

`HeaderNav` is retired; its auth-checking logic moves into `Sidebar`.

### Component placement

All `#[component]` functions live in `web/src/pages/` (coverage exclusion is
path-based per `CLAUDE.md`). Shared UI primitives go in a new file
`web/src/pages/ui.rs`, exported via `web/src/pages/mod.rs`.

---

## Steps

### Step 1 · CSS infrastructure

1.1 Add `rust-embed` and `axum-embed` as dependencies in `server/Cargo.toml`
    (embedding and serving happen server-side).

1.2 Copy `build/jaunder.css` and `build/jaunder-themes.css` into a new
    `server/assets/` directory. This is the compile-time source; `rust-embed`
    reads from here at build time.

1.3 In `server/src/`, define a `StaticAssets` struct with `#[derive(RustEmbed)]`
    pointed at `assets/`. Register an `axum-embed` handler mounted at `/style`
    on the Axum router.

1.4 Update the `shell()` function in `web/src/lib.rs` to load both stylesheets
    via `<link>` tags in `<head>`, replacing (or supplementing) the `<Stylesheet>`
    macro call for `/pkg/jaunder.css`.

1.5 Verify the app builds, assets are served at `/style/jaunder.css` and
    `/style/jaunder-themes.css`, and the styles render in a browser.

### Step 2 · Theme context

2.1 Add a `Theme` enum (`Terminal | Studio | Reader`) in `web/src/` with `Display`
    and `FromStr` impls producing the string IDs used in CSS (`terminal`, etc.).

2.2 In `App`, create a `RwSignal<Theme>` defaulting to `Terminal`, restore it from
    `localStorage` on WASM startup, and provide it via `provide_context`.

2.3 In the shell layout (step 4), consume the signal and apply it as a reactive
    `data-theme` attribute on the `.j-shell` root div.

2.4 On WASM, write the signal to `localStorage` whenever it changes.

### Step 3 · Shared UI components (`web/src/pages/ui.rs`)

Port each JS partial to a Leptos `#[component]`. Components take typed props
matching the data they display; they never touch server functions directly.

3.1 `Icon` — wraps an SVG path string and size into the `<svg class="j-icon">` pattern.

3.2 `Avatar` — derives initials and an `oklch` hue from a display name, renders the
    `j-av` div. Props: `name: String`, `size: u32`.

3.3 `Dot` — renders a `j-dot` span colored via `var(--c-{proto})`. Props: `proto: String`.

3.4 `Chip` — filter pill with optional protocol dot, label, and count.
    Props: `label: String`, `proto: Option<String>`, `count: Option<u32>`, `active: bool`.

3.5 `Topbar` — renders the `j-topbar` bar. Props: `title: String`,
    `sub: Option<String>`, and a `children` slot for the right-hand action area.

3.6 `PostCard` — renders an `<article class="j-post">` from a `TimelinePostSummary`
    (the type already used in `HomePage`). Includes avatar, header, body, and stats footer.

3.7 `InlineComposer` — the compact draft row at the top of the timeline. Wires to
    the existing compose flow (navigates to `/posts/new` or submits inline — decide
    at implementation time).

3.8 `Sidebar` — the left nav. Accepts the active nav key and reads the theme signal
    and current user from context (no prop drilling). Renders nav items, sources list,
    and auth-aware footer (user info when logged in, Sign-in button when not).
    Discuss granularity (monolithic component vs. sub-components) at implementation time.

### Step 4 · Layout restructure

4.1 Create `AppShell` as a Leptos `#[component]` in `web/src/pages/mod.rs`.
    It renders the `j-shell` div (with reactive `data-theme`), `<Sidebar>`, the
    `j-main` div, and `<Outlet />` inside `j-main`.

4.2 Restructure the `<Router>` in `App` to use `AppShell` as a parent route wrapping
    all existing routes (use `<ParentRoute>` + `<Route>` nesting).

4.3 Delete `HeaderNav` once `Sidebar` covers its functionality.

4.4 Update integration and e2e tests that assert on the old `<header>`/`<nav>`
    structure to match the new sidebar structure.

### Step 5 · Timeline / home screen

5.1 Rewrite `HomePage` (`web/src/pages/home.rs`) to use `Topbar`, `InlineComposer`
    (auth only), and a list of `PostCard` components fed by the existing
    `list_home_feed` / `list_local_timeline` server functions.

5.2 The unauthenticated branch renders the hero section (headline, description,
    protocol chip row) above the local posts list, mirroring `screenUnauth`.

5.3 Update or add integration tests covering both auth and unauth branches of the
    new homepage rendering.

5.4 Update e2e tests to reflect the new DOM structure.

### Step 6 · Compose screen

6.1 Apply the compose screen layout to the existing new-post page
    (`web/src/pages/posts.rs` → `CreatePostPage`). The two-column grid:
    left `j-compose-body` (avatar + editor), right `j-compose-aside`
    (cross-post destinations + character count + action buttons).

6.2 Cross-post destination rows are UI-only in this phase (hardcoded or from
    user's connected protocols if that data is available). Discuss at implementation
    time what data source to use.

6.3 Update integration and e2e tests for the new compose layout.

---

## Out of scope (this phase)

- Settings screen UI
- Theme-switching UI (infrastructure only)
- User stylesheet upload/management
- Protocol connection management UI
