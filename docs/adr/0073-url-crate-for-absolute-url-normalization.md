# ADR-0073: The `url` crate is the sanctioned absolute-URL normalizer, and it enters the `common`/wasm graph

- Status: proposed
- Date: 2026-07-20
- Issue: [#448](https://github.com/jaunder-org/jaunder/issues/448)

## Context

ADR-0063 (domain-value newtypes) qualifies a value for a newtype on the
**invariant** axis when it needs normalization a bare `String` can't guarantee.
Absolute feed/site URLs — the site origin (`SiteIdentity.base_url`), the WebSub
hub target, and the server-composed feed/atompub URLs — are exactly that: scheme
case, host case, default port, percent-encoding, and trailing slash all need
canonicalizing so two spellings of one URL are not both representable. Before
#448 these crossed as bare `String`/`Option<String>`, composed by
`format!("{base}{path}")` with an ad-hoc `percent_encode_path` helper, and the
only scheme check lived in a single web form.

Introducing the `AbsoluteUrl` newtype (#448) forces a dependency decision. The
normalization it needs is precisely what the `url` crate does correctly (IDNA
host handling, RFC-3986 percent-encoding, origin canonicalization) — but `url`
was previously an `xtask`-only dependency. Adding it to `common` matters because
`common` is compiled to **wasm**: it is in the Leptos client's dependency graph,
and the client deserializes `SiteIdentity.base_url` (the site-settings page
reads it), so `url::Url::parse` becomes **reachable** in the client wasm binary
— not merely compiled and dead-stripped. The alternatives were hand-rolling URL
normalization (host casing, percent-encoding, scheme validation) or leaning on
the existing `urlencoding` crate, which is a query-param encoder, not a URL
parser.

## Decision

`url` is the project's sanctioned absolute-URL parser/normalizer. It is added to
`[workspace.dependencies]` and to `common`'s `[dependencies]`, and
`AbsoluteUrl`'s `FromStr` chokepoint (and its `join` composer) parse/normalize
through `url::Url`.

We **accept** that `url` (and its `idna` unicode tables) are compiled for wasm
and reachable in the client binary. We do **not** hand-roll URL normalization,
and we do **not** repurpose `urlencoding` for parsing. The bundle cost is judged
acceptable because `common` already ships heavier machinery to wasm (`rss`,
`atom_syndication`, `quick-xml`, `serde_json`,
`unicode-normalization`/`unicode-segmentation`), and a single correct
normalization chokepoint is worth more than avoiding one more unicode
dependency.

## Consequences

- **Commits us to** a modest client-wasm bundle increase (`idna` tables atop the
  unicode tables already present) in exchange for correct, single-chokepoint URL
  normalization that no boundary — CLI, config, storage, or web — can bypass.
- **A one-time shared-vendor cold rebuild** when `url` first joins the `common`
  graph; version-match the existing `xtask` `url` pin (`2.5.x`) to reuse the
  vendor.
- **Rules out** hand-rolled URL normalization and re-deriving the
  scheme/trailing-slash rules per boundary; `AbsoluteUrl` is now the one place
  they live (per ADR-0063 §4/§5).
- **Follow-up:** the composed absolute-or-relative URL fields (feed `self_url`/
  `canonical_url`, atompub `id`/`edit_uri`/`content_src`, post
  `permalink`/`preview_url`) are a sum type, not `AbsoluteUrl`, and are typed
  under a separate issue (#560).
