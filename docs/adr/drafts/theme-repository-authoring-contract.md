# ADR-DRAFT: Theme repositories use the Jaunder CLI for canonical artifacts

- Status: proposed
- Date: 2026-09-12
- Issue: [#1443](https://github.com/jaunder-org/jaunder/issues/1443)

## Context

Theme Packages are portable ZIP archives, but authors currently have only the
browser Studio for validation and preview. There is no repository convention, no
offline package command, and no reproducible thumbnail contract. A custom theme
can therefore live in source control only through project-specific build scripts
whose validation, ZIP bytes, and screenshots may drift from Jaunder.

A theme repository may use handwritten CSS or a higher-level authoring language.
Making one preprocessor part of the Theme Package contract would couple portable
package compatibility to an unrelated toolchain. Thumbnail rendering has the
opposite problem: browser, font, viewport, fixture, and motion differences must
be fixed somewhere if repositories are to compare generated images.

The authoring interface could live in a separate tool, contributor-only xtask,
or the deployed `jaunder` binary. Theme authors need the same package compiler
as their target Jaunder release without checking out the Jaunder source tree,
while deployments must not acquire a bundled browser.

## Decision

A standard theme repository keeps `theme.json`, canonical `style.css`, and its
optional manifest-declared `assets/` tree at the root. Documentation, committed
`preview.png`, workflows, and optional preprocessor source remain repository
files, not package members. `style.css` is the only stylesheet source Jaunder
consumes; preprocessing is repository-owned and runs before Jaunder.

The deployed binary provides the database-independent `jaunder theme check`,
`package`, and `thumbnail` interface. Directory input is an adapter over the
same bounded Theme Package validator and CSS transformer used by Studio; package
output uses the deterministic exporter. The commands never execute repository
hooks or mutate Jaunder instance state.

Thumbnail generation renders a fixed, versioned Style Contract fixture at a
fixed viewport, waits for local fonts, suppresses motion and caret rendering,
and captures a PNG through an externally supplied Chromium-compatible browser.
Jaunder neither bundles nor downloads that browser. Theme repositories commit
the PNG, and a pinned GitHub workflow fixes the Jaunder/browser/font
environment, rejects thumbnail drift, builds the deterministic ZIP, and attaches
that ZIP to tagged releases.

The thumbnail preview's loopback HTTP server is a CLI-owned, command-lifetime
transport with no supported external address, not a deployed application
endpoint. Its routes receive host integration coverage, while the public
thumbnail command receives browser coverage through the pinned repository
workflow. Jaunder does not expose a test-only preview address solely to attach
Playwright.

Theme discovery remains ordinary links between GitHub repositories. There is no
registry or remote package-fetch interface in this decision.

## Consequences

Theme repositories gain one portable layout and one version-matched tool for
validation, packaging, and presentation artifacts. Package rules remain local to
the existing compiler instead of being reimplemented by templates or CI.
Generated ZIPs stay out of normal source history while repository landing pages
retain an immediately visible preview.

Authors may use Sass or any other build system, but they must commit current
portable CSS and make their own preprocessing drift check run before the pinned
Jaunder workflow. Jaunder does not inherit those languages, lockfiles, code
execution, or compatibility policies.

The production binary gains offline authoring commands and a canonical preview
fixture, but not a browser closure. Local thumbnail bytes can vary with an
unpinned browser or fonts; the pinned GitHub workflow is the canonical
authority. A thumbnail demonstrates one representative surface only and does not
replace manual responsive, accessibility, cross-browser, and route coverage.

A future gallery, registry, one-click remote import, additional canonical
fixtures, or higher-level stylesheet contract requires a separate decision.
