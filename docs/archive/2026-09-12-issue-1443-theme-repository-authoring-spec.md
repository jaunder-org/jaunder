# Theme repository authoring

Issue: [#1443](https://github.com/jaunder-org/jaunder/issues/1443)

## Outcome

Theme authors can maintain a portable Theme Package in an ordinary GitHub
repository, verify and package it with the shipped `jaunder` binary, and produce
a recognizable preview thumbnail through one documented, reproducible workflow.
Theme Studio explains this path instead of presenting an unexplained empty
catalog.

## Load-bearing decisions

- A theme repository's canonical package source is `theme.json`, `style.css`,
  and the optional manifest-declared `assets/` tree at the repository root.
  Repository support files such as `README.md`, `preview.png`, `.github/`, and
  optional authoring source directories are outside the package.
- `style.css` is the portable source of truth at the Jaunder boundary. Authors
  may generate it with Sass, another preprocessor, or handwritten CSS, but the
  preprocessor and its dependency graph remain owned by the theme repository.
  Jaunder neither executes repository build hooks nor standardizes a
  higher-level stylesheet language.
- The deployable binary exposes one nested authoring interface:
  `jaunder theme check <repository>`,
  `jaunder theme package <repository> --output <zip>`, and
  `jaunder theme thumbnail <repository> --browser <executable> --output <png>`.
  Every repository, output, and browser operand is explicit. These commands
  require no Jaunder database, catalog, account, or running server and never
  mutate instance state.
- `check` reads the named repository's canonical package members, applies the
  same bounded archive, manifest, asset, CSS parsing, scoping, naming, and URL
  rules as Theme Studio, and reports actionable path-bearing validation
  failures. It ignores every repository-root entry other than `theme.json`,
  `style.css`, and `assets/`; within `assets/`, it rejects missing declared
  assets, undeclared entries, symlinks, non-regular files, and unsafe paths
  before passing the collected bytes to the shared validator.
- `package` succeeds only for source accepted by `check` and atomically writes a
  deterministic importable ZIP containing exactly the canonical package members.
  It fails rather than overwrite an existing output.
- `thumbnail` succeeds only for source accepted by `check`. It renders the
  package against a fixed, versioned Style Contract fixture at a documented
  1200×800 viewport and writes a PNG. The fixture uses deterministic content,
  exercises representative navigation, masthead, Post, metadata, tag, and
  continuation hooks, loads only package-local assets, waits for fonts, and
  disables animation and caret rendering.
- Thumbnail generation uses the explicitly named, externally installed
  Chromium-compatible browser. The Jaunder binary does not discover, bundle, or
  download a browser. A missing or unusable executable is a clear command
  failure.
- `preview.png` is committed in each theme repository. Package ZIPs are
  generated artifacts attached to tagged GitHub releases rather than committed
  binaries.
- A documented, pinned GitHub workflow is the authority for the canonical
  thumbnail and release ZIP. It pins Jaunder, Chromium, fonts, fixture version,
  and viewport; fails when committed `preview.png` differs; and publishes the
  deterministic ZIP for release tags. Repository-owned preprocessing, when
  present, runs before Jaunder and separately proves that committed `style.css`
  is current.
- Theme discovery remains links between ordinary GitHub repositories. This
  change creates no registry, remote search protocol, automatic catalog
  population, or remote-import trust path.
- Theme Studio provides concise, always-available repository-authoring guidance
  and a genuine empty-catalog explanation. Durable package grammar, authoring,
  thumbnail, workflow, and release instructions remain in `docs/themes.md`.

## Acceptance

- A repository containing only the canonical package source plus normal support
  files passes `jaunder theme check <repository>`; malformed manifests,
  undeclared or unsafe assets, unsupported CSS, and escaped or external URLs
  fail through the same rules as Studio import.
- Repeated `jaunder theme package <repository> --output <zip>` runs over
  unchanged source produce byte-identical ZIPs, and Studio can import the result
  as a private, unselected draft.
- `jaunder theme thumbnail <repository> --browser <executable> --output <png>`
  produces a 1200×800 PNG without database or network access and reports a clear
  error when the browser executable is unavailable.
- The pinned GitHub workflow detects a stale committed thumbnail and exposes the
  deterministic ZIP as a workflow artifact; a release tag publishes that ZIP as
  a release asset.
- A newly authenticated author with an empty catalog sees an explanation and
  concrete create/import/repository-authoring next actions, while loading and
  server failures remain visibly distinct from a genuine empty catalog.
- The authoring guide documents the repository tree, tool-agnostic preprocessing
  boundary, all three commands, canonical thumbnail contract, pinned workflow,
  release artifact, and the existing private-draft → preview → publish → select
  lifecycle.

## Boundaries

- No official theme repository, sample visual design, registry, gallery,
  repository search, rating, update notification, or one-click remote import is
  delivered here.
- No Sass compiler, generic build-hook execution, JavaScript, template language,
  source map, or executable Theme Package content enters Jaunder.
- The thumbnail is a repository presentation artifact, not proof of responsive,
  accessible, cross-browser, or route-complete theme quality; the existing
  manual preview checklist remains authoritative for those properties.
- The package format, Style Contract version 1, catalog ownership, publication,
  selection, quotas, and immutable public asset model do not change.
