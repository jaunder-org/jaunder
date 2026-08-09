# Spec — #688: `AbsoluteUrl` through `WebSubClient::send_publish`

- Issue: [#688](https://github.com/jaunder-org/jaunder/issues/688)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  §5 (existing-newtype adoption — **amended by this issue**)
- Date: 2026-08-09

## Problem

`WebSubClient::send_publish` (`server/src/websub/mod.rs:16`) takes two `&str`
URLs:

```rust
async fn send_publish(&self, hub_url: &str, feed_url: &str) -> Result<(), WebSubError>;
```

Both values are already `AbsoluteUrl` on either side of the call — the type is
flattened purely for the trait hop:

- `common/src/feed/mod.rs:37` —
  `FeedsConfig.websub_hub_url: Option<AbsoluteUrl>`
- `storage/src/site_config.rs:166` —
  `get_feeds_websub_hub_url() -> sqlx::Result<Option<AbsoluteUrl>>`
- `server/src/feed/worker.rs:233` — the call site builds
  `let absolute = compose(base, feed_url);`

ADR-0063 §5 makes adoption of an existing newtype **mandatory** on every surface
we define — "flattening it to a primitive requires express owner approval" — and
its sole carve-out is handing the value to a type we do not own. Neither the
trait nor its implementations are external types, so the flatten here has no
justification. The worker additionally `.as_deref()`s a validated
`Option<AbsoluteUrl>` straight out of storage and threads the primitive through
two internal signatures, which is exactly the "re-derived as a bare `String`"
shape §5 names.

`common/src/atompub/rsd.rs:19 render_rsd_document(service_url, homepage_url: &str)`
is the same flatten, with the same already-`AbsoluteUrl` caller
(`server/src/atompub/rsd.rs:39`), and is swept in the same pass.

### What this does _not_ buy — and a correction to the issue

Issue #688's body claims that typing the two parameters "turns a wrong-endpoint
publish into a compile error" under §1's transposition criterion. **That claim
is false and is not the basis for this work.** Both parameters become the _same_
type, `&AbsoluteUrl`, so `send_publish(feed, hub)` still compiles and still
pings the wrong endpoint. ADR-0063 §1's transposition examples
(`UserId`/`PostId`, `RawToken`/`TokenHash`) are all **distinct** type pairs; one
newtype in both slots buys no transposition safety at all.

Actual transposition safety would require distinct `HubUrl`/`FeedUrl` types — a
larger, separate design question, filed as a follow-up (see Separable concerns).
**The issue body must be corrected** so the false rationale does not propagate.

What this change _does_ deliver: mandatory §5 adoption, and the elimination of a
flatten/re-derive round trip on a value storage already validated.

## Decision

Thread `AbsoluteUrl` from the storage read to the trait call with no
re-derivation, and from the `compose` call to the RSD renderer.

### D1 — Trait and impls take `&AbsoluteUrl`

```rust
async fn send_publish(
    &self,
    hub_url: &AbsoluteUrl,
    feed_url: &AbsoluteUrl,
) -> Result<(), WebSubError>;
```

Borrowed, not owned: it matches the current `&str` shape, and the worker
retries, so `absolute` is used across attempts.

There are **five** `impl WebSubClient` blocks, all of which change in step:

| Impl                        | Location                                      |
| --------------------------- | --------------------------------------------- |
| `HttpWebSubClient`          | `server/src/websub/http.rs:37`                |
| `FileCapturingWebSubClient` | `server/src/websub/file_capture.rs:23`        |
| `NoopWebSubClient`          | `server/src/websub/noop.rs:9`                 |
| `CapturingWebSubClient`     | `server/tests/helpers/websub_capturing.rs:29` |
| `FailingWebSubClient`       | `server/tests/feed/feed_worker.rs:21`         |

Roughly ten in-crate test call sites currently pass string literals
(`http.rs:112,123,138,173,186`; `file_capture.rs:61,65,87,100`; `noop.rs:21`;
`mod.rs:61,70`) and must construct values via
`common::test_support::parse_absolute_url`. This churn is expected and in scope.
Note `"https://feed"` normalizes to `"https://feed/"`, so a few literal
assertions shift by a trailing slash.

The inner values are read out at the two external boundaries ADR-0063 §5
sanctions: `reqwest`'s form value and `IntoUrl` in `http.rs`, and
`serde_json::json!` in `file_capture.rs`. The generated `Serialize` emits
`serialize_str(&self.0)` (`macros/src/str_newtype.rs:202-209`), so the
`websub.jsonl` wire bytes are unchanged.

### D2 — The worker's plumbing is un-flattened, not re-parsed

`worker.rs:150-159` reads `Option<AbsoluteUrl>` from storage and immediately
`.as_deref()`s it. That becomes `.as_ref()`, and both internal signatures take
the newtype:

```rust
async fn process_feed_group(.., hub_url: Option<&AbsoluteUrl>, ..)
async fn ping_websub(.., hub_url: Option<&AbsoluteUrl>, ..)
```

`ping_websub` **stays `Option`** — it owns the no-hub branch
(`worker.rs:264-268`), which records `PingOutcome::NoHub` and marks the batch
pinged. That branch does not move, and the metric must survive unchanged.

The alternative — re-parsing the `&str` back into `AbsoluteUrl` at the call site
— is rejected: it re-introduces a fallible parse of a value storage already
validated, creating an error path with no sensible handling.

`tracing` fields recording `hub` (worker.rs:234, 240, 248, 256) become
`hub = %hub`, since `&AbsoluteUrl` is not a `tracing` primitive.

`ping_websub`'s **`feed_url` becomes `&FeedPath`**. It is not an absolute URL —
it is the site-relative feed path fed to `compose` — but it is not untyped
either: the caller at `worker.rs:201` passes `&feed_path` where
`feed_path: FeedPath`, so the current `&str` is a flatten of an _existing_
newtype by the very signature this issue rewrites. Leaving it would fix half a
flatten and leave the other half indistinguishable from an oversight. `compose`
still accepts it via `Deref`; the `feed_url` tracing fields become
`feed_url = %feed_url`.

### D3 — In-process doubles are typed; wire-decoding doubles are not

`CapturedPing` (`server/tests/helpers/websub_capturing.rs:7`) holds two `String`
fields fed from already-typed values, so §5 adoption applies and typing it is
free:

```rust
pub struct CapturedPing {
    pub hub_url: AbsoluteUrl,
    pub feed_url: AbsoluteUrl,
}
```

`HubForm` (`http.rs:74`) **stays `String`** — a deliberate deviation from issue
#688's Scope section, which lists `HubForm` as in scope. It is the `Form`
extractor on the axum hub spawned inside the test: the decoder for bytes
production actually posted. A validating `AbsoluteUrl` field would make a
malformed send fail as an axum form rejection the test never sees, instead of a
readable assertion diff.

Per ADR-0063 §5 this flatten is recorded here as **express owner approval,
granted 2026-08-09**. The approval is what authorizes it in the interim; once D4
lands, `HubForm` falls under a categorical carve-out and needs no per-site
approval.

### D4 — ADR-0063 §5 gains the wire-decoding-double carve-out

D3 is not a one-off.
`server/tests/web/web_auth.rs:34 struct Resp { token: String }` already does the
same thing — decodes the login response as `String`, then parses to `RawToken`
explicitly in the test body — while its production counterpart
`web/src/auth/api.rs:33 LoginResponse` is typed. Two named instances plus ~11
anonymous decode sites (capture-file parsers in `websub/file_capture.rs`,
`mailer/file.rs`; response-JSON parsers in the media and backup tests) share the
shape. §5's carve-out is amended to name it, documenting existing practice
rather than introducing a rule.

### D5 — `render_rsd_document` keeps its XML escaping

The escaping is **load-bearing, not defense-in-depth**: `&` is a legal query
separator and survives `AbsoluteUrl` normalization intact, so without
`quick_xml::escape` a hub URL with a query string emits malformed XML. The two
params become `&AbsoluteUrl`, read out via `Deref` at the `quick_xml::escape`
boundary (§5 external type).

Normalization behavior, probed against the pinned `url` 2.5.8 during spec
review:

| Char | In query | In path  |
| ---- | -------- | -------- |
| `&`  | survives | survives |
| `<`  | `%3C`    | `%3C`    |
| `"`  | `%22`    | `%22`    |
| `'`  | `%27`    | survives |

So the existing test's `<` assertion is unreachable and is dropped; the `&` case
is kept and becomes the test's subject. `'` is reachable via a path and renders
as `&apos;`, harmless in both the element-text and attribute-value contexts the
template uses — no assertion is required for it.

### D6 — A test whose subject the type absorbed is deleted

`http.rs:109 returns_http_error_for_invalid_url_scheme` asserts that `reqwest`
rejects an unparseable URL string. `AbsoluteUrl::from_str` rejects non-`http(s)`
schemes at the parse boundary and the input is now unconstructible, so the test
is deleted rather than retargeted; retargeting would duplicate
`returns_http_error_on_connection_refused` (line 163) under a name describing
something it no longer tests. That test continues to cover the
`WebSubError::Http(_)` arm, and scheme rejection remains covered by the existing
`rejects_non_http_schemes` (`common/src/absolute_url.rs:106-115`).

`mod.rs:61` is a different case — its comment already claims to test an
unroutable host while its input is an unparseable string. It is reshaped to
`http://127.0.0.1:1/`: a valid `AbsoluteUrl`, guaranteed connection-refused, no
DNS and no network egress.

## Acceptance criteria

1. **AC1** — `WebSubClient::send_publish` and **all five** implementations
   listed in D1's table take `hub_url: &AbsoluteUrl, feed_url: &AbsoluteUrl`.
2. **AC2** — No `.as_deref()` on the WebSub hub URL remains in
   `server/src/feed/worker.rs`; `process_feed_group` and `ping_websub` both take
   `hub_url: Option<&AbsoluteUrl>`, and `ping_websub` takes
   `feed_url: &FeedPath`.
3. **AC3** — On the **production** path (`server/src/feed/worker.rs` and the
   three non-test `send_publish` impls), no `AbsoluteUrl` is re-parsed or
   re-derived from a string between `get_feeds_websub_hub_url()` and
   `send_publish`: no `.parse()`, `String::from`, or `.to_string()` on these
   values. (Test files legitimately call `parse_absolute_url` to build fixtures
   and are excluded.)
4. **AC4** — `PingOutcome::NoHub` is still recorded when no hub is configured,
   and the existing `feed_worker.rs` test covering that path passes unchanged.
5. **AC5** — `CapturedPing`'s `hub_url` and `feed_url` fields are `AbsoluteUrl`,
   and the `server/tests/feed/feed_worker.rs` assertions reading `.pings()`
   (lines 110-118 in `worker_pings_hub_when_configured`, and 158-163 in
   `worker_groups_duplicate_events_into_single_regen`) still pass —
   **unchanged**. The generated `PartialEq<&str>` and `Deref<Target = str>` mean
   none of them need editing; a diff touching those assertions is a signal
   something else went wrong.
6. **AC6** — `HubForm`'s **field types** are unchanged, and it carries a comment
   stating why it is deliberately `String`, citing ADR-0063 §5.
7. **AC7** — ADR-0063 §5 contains a paragraph covering wire-decoding test
   doubles, citing `HubForm` and `web_auth.rs`'s `Resp` as its instances.
8. **AC8** — `render_rsd_document` takes two `&AbsoluteUrl` params and still
   calls `quick_xml::escape` on both.
9. **AC9** — An RSD test asserts that `foo=1&bar=2` in a URL query renders as
   `foo=1&amp;bar=2`. No test asserts on `&lt;` from a URL input.
10. **AC10** — `returns_http_error_for_invalid_url_scheme` no longer exists.
11. **AC11** — `websub/mod.rs`'s HTTP arm sends to `http://127.0.0.1:1/` and
    still asserts `is_err()`.
12. **AC12** — Issue #688's body is edited to remove the false transposition
    rationale and point at this spec.
13. **AC13** — The separable concerns below are filed, and **their issue numbers
    are written back into this spec's Separable-concerns section** (or the
    covering issue number recorded, where an existing issue already covers one).
14. **AC14** — `cargo xtask validate` is green — the **full** local gate,
    including the e2e matrix, because the Verification section below reasons
    about the e2e WebSub capture path that `--no-e2e` structurally cannot
    exercise.

## Separable concerns — filed, not folded in

None are folded in; the plan's first task files them (after checking
[#751](https://github.com/jaunder-org/jaunder/issues/751),
[#697](https://github.com/jaunder-org/jaunder/issues/697), and
[#827](https://github.com/jaunder-org/jaunder/issues/827) for overlap) so they
can be picked up concurrently. Checked against #751 (storage row tuples), #697
(the anti-drift gate), and #827 (localStorage keys) — none of these were
covered, so all five were filed fresh:

- **[#875](https://github.com/jaunder-org/jaunder/issues/875)** — distinct
  `HubUrl` / `FeedUrl` newtypes: whether real transposition safety for the
  WebSub pair is worth two distinct types, per the correction above.
- **[#877](https://github.com/jaunder-org/jaunder/issues/877)** —
  `server/tests/helpers/mod.rs:220,235,282`:
  `atompub_authed(method, uri, username, ..)` and siblings, three adjacent
  `&str`, a live swap hazard across the AtomPub test surface.
- **[#878](https://github.com/jaunder-org/jaunder/issues/878)** —
  `tools/devtool/src/pg.rs:23`, `PgEnv { test_url, bootstrap_url }`: two
  adjacent Postgres URLs; a swap points the suite at the bootstrap database.
- **[#879](https://github.com/jaunder-org/jaunder/issues/879)** —
  `web/src/sidebar/component.rs:9` and `web/src/posts/render.rs:140`:
  `RootRelativeUrl` candidates (`icon_path`/`href`, `banner`/`permalink`).
- **[#880](https://github.com/jaunder-org/jaunder/issues/880)** —
  `on_regen_failure(feed_url: &str)` (`worker.rs:276`): after D2 this cycle
  hands the same `&feed_path` to `ping_websub` as `&FeedPath` and to
  `on_regen_failure` as `&str`, four lines apart. Filed so the asymmetry reads
  as a decision.

**Noted during review, not filed:**
`common/src/absolute_url.rs compose(base: &AbsoluteUrl, path: &str)` is the one
surface _we own_ where this cycle's own newtypes still meet a primitive —
`ping_websub` now holds a `FeedPath` and `compose` deref-flattens it back to
`&str`. It is a genuine §5 residual rather than an external-type carve-out. Left
alone because typing it means deciding what `path` actually is (`FeedPath` is
only one of its callers' types), which is a design question, not a mechanical
retype — and #879's `RootRelativeUrl` work is the natural place to answer it.

**Noted, deliberately not filed** as low-value: the xtask-internal
`Span { method, uri }` (`xtask/src/traces/parse.rs:26` and its two conversion
siblings), `CheckEntry`, and `RunRef` (`xtask/src/pr/snapshot.rs`).

## Open questions

None. The design tree is resolved.

## Verification

`cargo xtask validate` (full, with e2e) is the gate. The WebSub e2e path
(`FileCapturingWebSubClient` → `websub.jsonl`) is unchanged on the wire by D1,
so the e2e matrix should need no fixture updates; if it does, that is a signal
the serialization changed and must be investigated, not patched.
