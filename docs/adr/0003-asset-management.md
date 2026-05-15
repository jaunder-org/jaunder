# ADR-0003: Asset Management for Single-Binary Distribution

* Status: accepted
* Deciders: mdorman, Gemini CLI
* Date: 2026-05-14

## Context and Problem Statement

To fulfill the goal of being a "single-binary" application, Jaunder needs a way to serve static assets (CSS, JS, themes) without requiring external files to be present on the target filesystem.

## Decision Drivers

*   Ease of Deployment: A single file is easier to distribute and run.
*   Performance: Fast access to assets and support for browser caching.
*   Theming: Support for multiple themes and user-uploadable stylesheets.

## Decision Outcome

Chosen option: Use `rust-embed` to embed CSS and other static assets directly into the server binary.

### Implementation Details

*   `jaunder.css` and `jaunder-themes.css` are embedded using `rust-embed`.
*   Dedicated Axum handlers serve these files from the binary, including ETag and conditional-request support (via `axum-embed`).
*   Theme switching is handled via a `data-theme` attribute on the HTML shell.
*   User-uploadable stylesheets remain architecturally distinct and are served from the storage layer.

## Consequences

*   Good: No external asset files needed for deployment.
*   Good: Clean separation between base styles and user customizations.
*   Bad: Assets are coupled to the Rust build cycle (though `rust-embed` supports disk-reads in debug mode).
