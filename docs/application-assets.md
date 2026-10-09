# Application asset inventory and cache lifecycle

This is the bounded delivery inventory for the application, not a list of
user-owned Media or every installed Theme Package. Runtime instances are
inventoried by their canonical producer or stored revision; consumers must not
infer identities from filenames.

## Inventory and disposition

| Class / source                                                         | Address and policy                                                               | Authority / disposition                                                                                                                                                                                                    |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Protected application CSS, `server/assets/jaunder.css`                 | `/theme/<full-SHA-256>`; `public, max-age=31536000, immutable` on 200/304        | Canonical system artifact compiler; installed persistent bytes and eligibility. Non-selectable and system-only.                                                                                                            |
| Bundled Studio, Terminal, Reader, `host/system_theme_sources/`         | Same digest route and immutable policy                                           | Canonical Theme Package compiler and stored immutable revisions. Named selections follow installed release; system ownership is separate from custom quotas.                                                               |
| Published custom CSS and declared WOFF2/raster assets                  | Same digest route and immutable policy                                           | Canonical package compilation, stored revision and durable content eligibility. Custom CSS remains scoped.                                                                                                                 |
| Private draft CSS/assets, `/theme/draft/<theme-id>/<path>`             | `private, no-store`, including errors                                            | Owner admission; draft mutation never overwrites a published digest.                                                                                                                                                       |
| CSR glue, WASM, runtime modules and declared representations           | Full-SHA-256 `/pkg/` URLs; `public, max-age=31536000, immutable` on 200/304      | `devtool csr-bundle` role-tagged manifest; per-representation ETags, MIME from logical asset, `Vary: Accept-Encoding`. Manifest and fixed-name aliases are not served. Independent CSR storage contract remains unchanged. |
| Stable favicon, `public/favicon.ico`                                   | `/favicon.ico`; `no-cache` on 200/304                                            | Embedded site inventory; ETag revalidation rather than heuristic freshness. Bytes are unchanged.                                                                                                                           |
| Generated embedded index document                                      | `/index.html`; `no-cache` on 200/304                                             | Generated shell; ETag revalidation. It is not an immutable runtime asset.                                                                                                                                                  |
| Public projector documents                                             | Existing five-minute public freshness, body-derived ETag                         | Resolved presentation and application digest references; same projector/CSR document contract.                                                                                                                             |
| Private/missing/unmatched SPA shells                                   | `no-store`                                                                       | Not asset payloads; do not cache a mutable fallback as an immutable asset.                                                                                                                                                 |
| Content-addressed Media, `/media/<source>/<p1>/<p2>/<hash>/<filename>` | Existing `public, max-age=31536000, immutable`                                   | Exact Media identity and independent Media store. Unchanged by styling consolidation. Remote Media proxy resolution is not a second immutable asset namespace.                                                             |
| DB-independent thumbnail transport                                     | Generated application/package digest references, command-owned loopback lifetime | Same compiler and immutable response semantics; no production address, database dependency, or `/style` fallback.                                                                                                          |
| `/style/jaunder.css`, `/style/jaunder-themes.css`                      | 404 before SPA fallback; no aliases                                              | Retired stable stylesheet routes. `StaticAssets` and unused aggregate `server/assets/jaunder-themes.css` are removed. Application source remains compiler input, not a stable HTTP mount.                                  |

Inline semantic markup, style bindings and icons are document content, not
independently cacheable files. There are no additional stable public files in
`public/` today. Adding one requires an explicit mutable policy or admission to
an existing canonical content-addressed producer, plus response tests.

Styling responses share `server/src/immutable_content.rs`: exact logical MIME,
strong digest ETag, immutable policy, and empty-body conditional 304. CSR
representations retain their separate negotiated-response contract. Trace
collection resolves the actual document-owned stylesheet links and resource
entries; historical timing field names no longer imply retired `/style` URLs.
Staged public stylesheets are excluded from attribution, and Home has no public
stylesheet owner.

## Upgrade and rollback

The binary carries its complete canonical system inventory. Before accepting
requests, startup verifies and installs exact bytes and atomically advances
system references. Missing/corrupt admitted content or installation failure
prevents serving; there is no embedded styling fallback. A new application or
bundled package revision changes the affected digest URLs, not unchanged assets.

Deploy through the supported package/NixOS module and retain the normal data
directory and database. Do not delete the content store to perform an upgrade.
Home uses protected application CSS only; public Local and author permalinks use
their resolved public Theme Package in addition to application styling.

Rollback selects the earlier immutable package and lets the same startup
installer restore its system references. It does not delete the newer release's
retained bytes. Content remains eligible while referenced and until at least
31,536,300 seconds after detachment (one-year asset freshness plus five-minute
public-document freshness). Collection requires both no live reference and an
elapsed retention deadline. System bytes do not consume custom-theme quotas.

A refreshed document obtains the new digest references. Existing cached legacy
HTML or already-fresh `/style` responses cannot be remotely evicted by retiring
the routes; legacy documents can be unstyled until refreshed. Cache clearing is
not the upgrade protocol, and redirects or aliases must not conceal that break.

## Backup and restore

Use the canonical `jaunder backup` / `jaunder restore` commands. Backups include
retained styling bytes, eligibility and system metadata; do not copy only the
active CSS or edit the archive to replace digests.

Restore into a fresh, initialized, **never-served** target using the configured
service environment and service user. Keep the application stopped throughout
restore. Stopping a previously served target is insufficient: it is non-empty
and the importer correctly refuses it. The startup lock and empty-target guards
must not be weakened. Preserve PostgreSQL's canonical database bootstrap when
applicable.

After successful import, start the exact selected package, verify runtime
identity and actual restored schema, and configure the canonical base URL before
routing browser traffic to that target. Startup re-verifies system content. The
styling harness enforces this with a masked initial boot followed by an ordinary
but proxy-isolated boot; only explicit post-verification admission switches the
stable origin.

## Qualification and limits

From a clean checkout in the repository devShell:

```sh
devtool run -- cargo xtask production-baseline qualify-styling
```

This opt-in host command is not a production operation or an automatic CI job.
It binds the executing tools, three canonical fixture products/CSR bundles and
six VM profiles to one clean source pin, retaining anonymous/authenticated
Chromium contexts behind one Caddy origin per backend. On each SQLite and
PostgreSQL backend it proves Create → A warm → B application → A rollback → B
public theme → A rollback → same-backend restored A. It checks actual cache
events, exact running identities, computed presentation, retained-byte 200 and
empty 304 responses, retired-route 404s, Home protection and restored
readability. Restore uses the canonical current backup-format constant and
compares actual schema before browser admission.

Sanitized JSON/Markdown is published atomically under
`docs/evidence/styling-qualification/`; raw credentials, traces, backups, disks
and logs stay private. The historical
[pre-rebase qualification](evidence/styling-qualification/df860966fac563c6cdb1ba6ddac175822f19df38-1791488769470905508/summary.md)
records backup format 3 and schema 50. The completed
[post-recovery qualification](evidence/styling-qualification/11d459bfd85c9286b0a583cb181074f7ccdb671b-1791506451527739238/summary.md)
records format 3 and observed schema 51 on both backends, including same-backend
restore with deferred PostgreSQL foreign keys and the incoming Org verse
migration preserved. These proofs complement, rather than replace, the fourteen
refreshed full-page visual pairs and focused interaction/accessibility tests.

Chromium (or automated WebKit elsewhere) is **not actual Safari or iPhone
qualification**. Desktop Safari and iPhone remain unqualified until separately
observed. Styling qualification does not widen the ordinary format-1 baseline
validators, replace its four-restore matrix, prove production rollout, satisfy
[#1419](https://github.com/jaunder-org/jaunder/issues/1419), or establish
milestone acceptance. See [production baseline](production-baseline.md) for that
separate contract and [architecture](ARCHITECTURE.md) for delivery ownership.
