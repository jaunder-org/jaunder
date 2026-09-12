# Theme repository authoring implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for bounded task
> ownership. This outline exists because the approved spec adds a public CLI and
> filesystem contract, a browser-process seam, and a reusable cross-repository
> GitHub workflow.

## Scope

In:

- A bounded repository-directory adapter over the existing Theme Package
  compiler and deterministic exporter.
- Database-independent `jaunder theme check`, `package`, and `thumbnail`
  commands with the approved explicit operands.
- A versioned canonical Style Contract thumbnail fixture and external Chromium
  adapter.
- A pinned GitHub workflow, Theme Studio guidance, durable authoring docs, and
  focused behavioral coverage.
- The proposed authoring-contract ADR and its architecture projection.

Out:

- Theme registries, galleries, remote import, official theme designs, Sass or
  another preprocessor, arbitrary repository hooks, bundled browsers, and any
  change to Theme Package or Style Contract version 1.

## Task outline

- [x] Task 1: Validate and package repository source through the production CLI
  - Contract: `jaunder theme check <repository>` and
    `jaunder theme package <repository> --output <zip>` use one host-owned
    repository adapter. Root support files are ignored; `assets/` is closed,
    symlink-free, regular-file-only, and manifest-complete. Both actions run the
    existing package validator and CSS transformer with deterministic local
    asset URLs. Package writes are atomic, non-overwriting, and byte-stable.
  - Ownership: `host` owns filesystem admission and package preparation;
    `server` owns clap parsing, command dispatch, output, and atomic destination
    publication. The CLI must not open storage.
  - Verification: focused host tests cover support files, missing/undeclared
    assets, symlinks/non-regular entries, unsafe paths, and deterministic bytes.
    Behavioral cases run both commands against unsupported or unscopable CSS and
    external, escaped, and unresolved asset URLs, proving the shared transformer
    rejects them before output publication. CLI tests and an actual command
    smoke prove parsing, successful ZIP creation, and non-overwrite behavior. An
    integration test submits those exact CLI-produced ZIP bytes through Studio's
    import boundary and observes a private, unselected draft with the prior
    public selection unchanged.

- [ ] Task 2: Generate canonical thumbnails through an external browser
  - Depends on: Task 1's repository adapter and CLI nesting.
  - Contract:
    `jaunder theme thumbnail <repository> --browser <executable> --output <png>`
    compiles the same accepted source, serves a loopback-only preview containing
    a versioned deterministic Style Contract fixture and package-local assets,
    and captures exactly 1200×800 PNG bytes. A private Chromium DevTools adapter
    owns browser spawn, readiness through `document.fonts.ready`, motion/caret
    suppression, screenshot capture, typed failure, bounded shutdown, and
    temporary-profile cleanup. CDP request interception rejects every
    non-preview-origin request and records every accepted request, so the
    command performs no browser discovery, download, database access, or
    non-loopback fetch.
  - Ownership: a pure host-renderable web module owns fixture semantics and
    HTML; the server command owns loopback serving and the external browser
    lifecycle. The fixture exposes representative navigation, masthead, Post,
    metadata, tag, and continuation hooks without introducing a second Style
    Contract.
  - Canonical environment: one Nix-owned `themeThumbnailEnvironment` contract is
    shared by the smoke and GitHub workflow. It fixes the exact Chromium
    derivation, the existing isolated fontconfig/font set, headless/GPU and sRGB
    flags, `C.UTF-8`, `TZ=UTC`, and device scale factor 1; neither consumer
    recreates these pins independently.
  - Verification: pure renderer tests defend fixture version/content and
    escaping; command tests cover browser/process/CDP/HTTP failure paths and
    reject a synthetic non-loopback request. A smoke inside the canonical Nix
    environment generates a valid 1200×800 PNG twice from unchanged input,
    proves byte identity, and asserts that every recorded preview request is the
    fixture document or a validated package-local asset.

- [ ] Task 3: Publish the repository workflow and author guidance
  - Depends on: Tasks 1 and 2's final CLI and artifact contracts.
  - Contract: a reusable GitHub workflow pins the Jaunder source revision and
    consumes the canonical `themeThumbnailEnvironment`. It checks repository
    source, generates a temporary canonical thumbnail and fails on `preview.png`
    drift, emits the deterministic ZIP as a workflow artifact, and attaches it
    to a tagged GitHub release. Repository-owned preprocessing precedes this
    workflow and separately checks committed `style.css`; the workflow never
    executes an undeclared build hook.
  - Theme Studio: always show concise repository-authoring guidance; a genuine
    empty catalog gets an explanatory next-action state, while loading and
    server errors retain their distinct paths. Link to the durable authoring
    guide rather than embedding the full package manual.
  - Documentation: update `docs/themes.md` with the canonical repository tree,
    preprocessing boundary, exact CLI invocations, thumbnail fixture and pinned
    workflow contract, release ZIP, and the unchanged draft-to-selection flow.
  - Verification: focused component/render coverage asserts genuine empty versus
    error behavior and guidance;
    `devtool run -- cargo xtask e2e-local theme-management.spec.ts` exercises
    the authenticated empty Studio surface. A checked-in minimal repository
    fixture and caller workflow invoke the reusable workflow on the branch,
    proving thumbnail-drift failure and artifact upload in GitHub Actions.
    Before merge, an exact temporary tag at the reviewed head exercises the tag
    path into a draft GitHub release; verify the uploaded ZIP bytes against the
    local deterministic artifact, then delete that exact draft release and tag.
    No workflow source-text assertion stands in for these observations.

## Cross-task contracts

- There is one filesystem admission path and one package compiler. CLI actions,
  thumbnails, workflow automation, and tests must not reproduce manifest, MIME,
  CSS, path, or archive rules.
- The repository adapter returns an opaque accepted value plus canonical
  deterministic package bytes; callers cannot obtain a package or compiled
  preview from unchecked directory contents.
- Preview asset URLs are loopback-only and derived from validated canonical
  paths. The browser is untrusted process input: executable failure, protocol
  failure, unexpected exit, timeout, and cleanup failure remain distinguishable.
- Thumbnail fixture identity is versioned separately from the Theme Package
  schema but keyed to the Style Contract it renders. Changing fixture semantics
  intentionally invalidates canonical thumbnails.
- `preview.png` is source-controlled; ZIPs are workflow/release outputs.
  `package` fails rather than silently overwrite its caller-selected output;
  `thumbnail` may replace its explicitly selected PNG.

## Risk checks

- Preserve ADR-0184's closed non-executable package, external-resource ban,
  bounded parsing, selector scoping, and same-origin asset model.
- Refuse symlinks and non-regular entries before reading; prevent traversal and
  TOCTOU-sensitive path reopening; bound total and per-file reads before ZIP
  construction.
- Bind the preview server to loopback on an ephemeral port, serve only validated
  members, and prevent the canonical fixture from making external requests.
- Keep browser dependencies out of the deployable closure: the binary speaks to
  an explicit executable but does not bundle Chromium, Node, or Playwright.
- Keep `mod.rs` files to declarations/re-exports and maintain host/wasm
  file-level separation.
- Update all CLI help, architecture, user docs, workflow inputs, component
  behavior, and focused tests together; no compatibility alias or deprecated
  command path.
- Each task passes `devtool run -- cargo xtask check` through `jaunder-commit`;
  the completed branch passes the repository's required ship gates before PR.
