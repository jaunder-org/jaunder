# Feature-correct markup adapter host test

## Outcome

The documented default-feature web package test command completes successfully,
while the trusted-markup adapter's rendered-HTML assertion continues to run only
in a feature configuration that supports Tachys HTML serialization.

## Decisions

- Keep `Markup::inject_into` and its production callers unchanged.
- Treat the existing `to_html()` assertion as server-feature proof: gate that
  test with `feature = "server"`, where `leptos_axum` enables Leptos/Tachys SSR
  support and `AnyAttribute` can be serialized.
- If completing the default-feature package run reveals another assertion whose
  claimed behavior exists only behind an existing `feature = "server"`
  production branch, gate that assertion to the same feature rather than
  inventing default-feature behavior. The full run exposed exactly one:
  noncanonical `CeremonyHandle` rejection, whose validator is intentionally
  server-only.
- Do not enable Leptos SSR globally for web dev-dependencies or default host
  tests. The default-feature package run must continue to exercise the actual
  no-SSR host graph.
- Do not replace the behavioral assertion with a fake no-SSR rendering path or a
  compile-only assertion. The server-feature test must still verify the exact
  element, accumulated attributes, and verbatim trusted markup bytes.
- Preserve the existing host/CSR boundary and production feature definitions;
  this is test configuration correction only.

## Acceptance

- `cargo xtask test-local -- -p web` passes with the web crate's default
  features.
- Running the named adapter and ceremony-handle rejection tests with
  `web/server` enabled passes and retains their exact behavioral assertions.
- The normal static/wasm verification surface remains green, proving the CSR
  boundary was not weakened.
- No production code, dependency feature, sanitization rule, or trusted-markup
  API changes.

## Boundaries

- Do not add `leptos/ssr` to default or dev-dependency features.
- Do not alter `Markup`, `RenderedHtml`, the raw HTML sink census, or component
  call sites.
- Do not broaden this issue into changes to Tachys, Leptos, server rendering, or
  the test runner.
