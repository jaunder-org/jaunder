# `AbsoluteUrl` through `WebSubClient::send_publish` — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop flattening `AbsoluteUrl` to `&str` for the `WebSubClient` trait
hop and the RSD renderer, per ADR-0063 §5.

**Architecture:** A signature change rippling outward from one trait.
`AbsoluteUrl` already exists and every value on both sides of these calls
already has that type, so this is adoption, not construction — no new types, no
new validation, no wire change. Test fixtures build values through
`common::test_support::parse_absolute_url`.

**Tech Stack:** Rust 2024, `async-trait`, `reqwest`, `axum` (test hubs),
`rstest`, `cargo nextest`.

**Spec:**
[`docs/archive/2026-08-09-issue-688-absoluteurl-websub-spec.md`](./2026-08-09-issue-688-absoluteurl-websub-spec.md)

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Per-commit gate:** `cargo xtask check` must pass clean before **every**
  commit, including doc-only ones (**`jaunder-commit`**). Run it via
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-688-absoluteurl-websub -- cargo xtask check`.
- **Markdown is gated** — run `devtool run -- prettier -w <file>` on any `.md`
  you edit.
- **Final gate:** `cargo xtask validate` (full, **with** e2e) — spec AC14.
- **This change must not alter any wire format.** `websub.jsonl` bytes and the
  `hub.mode`/`hub.url` form body stay byte-identical.
- **Test URL fixtures** are built with
  `common::test_support::parse_absolute_url`, never `AbsoluteUrl` internals. It
  is reachable from both sides with **no `cfg` needed**: `common/src/lib.rs:49`
  gates the module on `any(test, feature = "test-support")` so `common`'s own
  tests see it, and `server/Cargo.toml:74` dev-depends on `common` with
  `test-support` enabled, covering both `server/tests/` and `server/src`'s
  `#[cfg(test)]` mods.
- Normalization shifts some literals: `"https://feed"` → `"https://feed/"`.

---

## Review header

**Scope — in:** the `WebSubClient` trait + its five impls; `worker.rs`'s
hub/feed plumbing; `CapturedPing`; `render_rsd_document` + its caller; the
ADR-0063 §5 amendment; two test reshapes.

**Scope — out:** distinct `HubUrl`/`FeedUrl` types (filed, task 1); `HubForm`
(deliberately stays `String`, spec D3); every other transposition site found by
the sweep (filed, task 1).

| Task | One line                                                                     |
| ---- | ---------------------------------------------------------------------------- |
| 1    | File the separable concerns; record their numbers in the spec                |
| 2    | Correct issue #688's body — the transposition rationale is false             |
| 3    | `render_rsd_document` takes `&AbsoluteUrl`; RSD escaping test restated       |
| 4    | Production retype: trait + three impls + `worker.rs` — lib compiles          |
| 5    | Test retype: all fixtures and the two test-crate impls — all targets compile |
| 6    | Amend ADR-0063 §5 with the wire-decoding-double carve-out                    |
| 7    | Full `cargo xtask validate` and spec-AC sweep                                |

**Key risks / decisions:**

- **Tasks 4 and 5 are one compile unit and one commit.** The cut is deliberate:
  task 4 ends where `cargo check -p jaunder --lib` genuinely passes (production
  code is internally consistent), task 5 ends where `--all-targets` passes. Both
  checkpoints are real. Do **not** try to commit between them — the tree does
  not build until 5 is done.
- `HubForm` stays `String` on purpose. Do not "fix" it.
- `PingOutcome::NoHub` must survive — `ping_websub` keeps its `Option`.
- Expect **zero** edits to `feed_worker.rs`'s ping assertions (task 5 step 3).

---

## File Structure

| File                                               | Change                                             | Task |
| -------------------------------------------------- | -------------------------------------------------- | ---- |
| `common/src/atompub/rsd.rs`                        | signature → `&AbsoluteUrl`; escaping test restated | 3    |
| `server/src/websub/mod.rs`                         | trait signature                                    | 4    |
| `server/src/websub/http.rs`                        | impl                                               | 4    |
| `server/src/websub/file_capture.rs`                | impl                                               | 4    |
| `server/src/websub/noop.rs`                        | impl                                               | 4    |
| `server/src/feed/worker.rs`                        | `.as_deref()`→`.as_ref()`; two signatures; tracing | 4    |
| `server/src/websub/*.rs` (`#[cfg(test)]` mods)     | fixtures; delete one test; `127.0.0.1:1` reshape   | 5    |
| `server/tests/helpers/websub_capturing.rs`         | impl + `CapturedPing` fields                       | 5    |
| `server/tests/feed/feed_worker.rs`                 | `FailingWebSubClient` impl                         | 5    |
| `docs/adr/0063-domain-value-newtype-convention.md` | §5 amendment                                       | 6    |

Unchanged, verified only: `server/src/atompub/rsd.rs:39` (already passes two
`AbsoluteUrl`s).

---

### Task 1: File the separable concerns

**Files:**

- Modify: `docs/superpowers/specs/2026-08-09-issue-688-absoluteurl-websub.md`
  (Separable-concerns section — replace each `#TBD`)

**Interfaces:**

- Produces: issue numbers recorded in the spec. Nothing downstream consumes
  them; this task exists so the findings are not lost behind this cycle.

- [x] **Step 1: Check the three existing issues for overlap**

```bash
gh issue view 751 --repo jaunder-org/jaunder
gh issue view 697 --repo jaunder-org/jaunder
gh issue view 827 --repo jaunder-org/jaunder
```

For each concern below: already covered (record that issue's number) or file
new.

- [x] **Step 2: File what is not covered** (via **`jaunder-issues`**) — filed
      #875, #877, #878, #879, #880; none of #751/#697/#827 covered them

Five concerns, all milestone "Domain-value type safety (newtypes)", label
`type-safety`:

1. **Distinct `HubUrl` / `FeedUrl` newtypes.** Body must state that #688 typed
   both `send_publish` params as the same `AbsoluteUrl`, which delivers §5
   adoption but **not** transposition safety, and ask whether two distinct types
   are worth it.
2. **`server/tests/helpers/mod.rs:220,235,282`** —
   `atompub_authed(method, uri, username, ..)`, `atompub_xml`, `atompub_at`:
   three adjacent `&str`.
3. **`tools/devtool/src/pg.rs:23`** — `PgEnv { test_url, bootstrap_url }`; a
   swap points the suite at the bootstrap database. Note
   `server/src/cli.rs:48 InvalidPgUrl` already exists as a partial `PgUrl`.
4. **`web/src/sidebar/component.rs:9`** and **`web/src/posts/render.rs:140`** —
   `RootRelativeUrl` candidates (`icon_path`/`href`, `banner`/`permalink`).
5. **`on_regen_failure(feed_url: &str)`** (`server/src/feed/worker.rs:276`) —
   after task 4, `process_feed_group` passes the same `&feed_path` to
   `ping_websub` as `&FeedPath` and to `on_regen_failure` as `&str`, two
   adjacent lines apart (`worker.rs:201` vs `:205`). Deliberately out of scope
   here; file it so the asymmetry reads as a decision, not an oversight.

- [x] **Step 3: Record the numbers in the spec**

Replace each `_(#TBD)_` in the spec's Separable-concerns list with the real
issue number (or the covering issue's number), and add a bullet for concern 5.
Spec AC13.

- [x] **Step 4: Commit** — `de0550d2`

```bash
devtool run -- prettier -w docs/superpowers/specs/2026-08-09-issue-688-absoluteurl-websub.md
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-688-absoluteurl-websub -- cargo xtask check
git add docs/superpowers/specs/2026-08-09-issue-688-absoluteurl-websub.md
git commit -m "docs(spec): record #688's separable concerns as filed issues (#688)"
```

---

### Task 2: Correct issue #688's body

**Files:** none in-tree — this edits the GitHub issue.

**Interfaces:** none.

- [x] **Step 1: Edit the issue body**

Remove the section "## Why the ADR-0063 §4 read-path allowance does not cover
this" and its claim that typing the two parameters makes a swap a compile error.
Replace with a short statement that both parameters become the same
`AbsoluteUrl`, so the change delivers ADR-0063 §5 adoption (and removes a
re-derive round trip) but **not** transposition safety, linking the
distinct-types issue from task 1 and this spec. Also correct the Scope section:
`HubForm` is deliberately excluded (spec D3), and `CapturingWebSubClient` is a
fifth impl the original scope missed.

Spec AC12. Use the GitHub MCP tools.

- [x] **Step 2: Verify**

```bash
gh issue view 688 --repo jaunder-org/jaunder
```

Expected: no "compile error" transposition claim remains; the §5 rationale and
the follow-up link are present. No commit — this task touches no files.

---

### Task 3: `render_rsd_document` takes `&AbsoluteUrl`

Independent of the WebSub trait; done first so one commit lands green on its
own.

**Files:**

- Modify: `common/src/atompub/rsd.rs:19` (signature), `:31-32` (escape calls),
  `:36-61` (tests)
- Verify only: `server/src/atompub/rsd.rs:39`

**Interfaces:**

- Produces:
  `pub fn render_rsd_document(service_url: &AbsoluteUrl, homepage_url: &AbsoluteUrl) -> String`

- [x] **Step 1: Rewrite the two tests**

Replace the whole `#[cfg(test)] mod tests` block in `common/src/atompub/rsd.rs`.
No `cfg` guard is needed on the `test_support` import — see Global Constraints.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::parse_absolute_url;

    #[test]
    fn rsd_document_contains_engine_name_and_urls() {
        let out = render_rsd_document(
            &parse_absolute_url("https://example.com/atompub/service"),
            &parse_absolute_url("https://example.com/home"),
        );
        assert!(out.contains("<engineName>Jaunder</engineName>"));
        assert!(out.contains("https://example.com/atompub/service"));
        assert!(out.contains("https://example.com/home"));
        assert!(out.contains("apiLink="));
    }

    // `&` is a legal query separator and survives AbsoluteUrl normalization, so
    // escaping it is what keeps the document well-formed XML. (`<` and `"` are
    // percent-encoded by normalization and can no longer reach this function.)
    #[test]
    fn rsd_document_escapes_query_ampersand() {
        let out = render_rsd_document(
            &parse_absolute_url("https://example.com/atompub?foo=1&bar=2"),
            &parse_absolute_url("https://example.com/home"),
        );
        assert!(out.contains("foo=1&amp;bar=2"));
        assert!(!out.contains("foo=1&bar=2"));
    }
}
```

- [x] **Step 2: Run, verify they fail**

Run: `devtool run -- cargo nextest run -p common rsd` Expected: FAIL —
`render_rsd_document` takes `&str`, not `&AbsoluteUrl`.

- [x] **Step 3: Change the signature**

`common/src/atompub/rsd.rs:19`:

```rust
use crate::absolute_url::AbsoluteUrl;

pub fn render_rsd_document(service_url: &AbsoluteUrl, homepage_url: &AbsoluteUrl) -> String {
```

The two `quick_xml::escape::escape(...)` calls at `:31-32` **stay** (spec D5)
and **must** read the inner value out explicitly:

```rust
homepage = quick_xml::escape::escape(homepage_url.as_ref()).into_owned(),
service = quick_xml::escape::escape(service_url.as_ref()).into_owned(),
```

`.as_ref()` is required, not stylistic: `escape` takes
`impl Into<Cow<'a, str>>`, and deref coercion does not apply through a generic
parameter — `&AbsoluteUrl` does not satisfy it.

Extend the doc comment's "Both URLs are XML-escaped to prevent injection." to
say why it is still load-bearing after typing: `&` is legal in a query and
survives normalization.

- [x] **Step 4: Run, verify they pass**

Run: `devtool run -- cargo nextest run -p common rsd` Expected: PASS (2 tests)

- [x] **Step 5: Verify the caller needs no change**

Run: `devtool run -- cargo check -p jaunder` Expected: PASS —
`server/src/atompub/rsd.rs:39` already passes two `AbsoluteUrl`s from `compose`.

- [x] **Step 6: Commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-688-absoluteurl-websub -- cargo xtask check
git add common/src/atompub/rsd.rs
git commit -m "refactor(atompub): render_rsd_document takes AbsoluteUrl (#688)"
```

---

### Task 4: Production retype — trait, three impls, worker

Everything needed for `cargo check -p jaunder --lib` to pass. **No commit** —
the test targets do not build until task 5.

**Files:**

- Modify: `server/src/websub/mod.rs:16` (trait)
- Modify: `server/src/websub/http.rs:37`, `file_capture.rs:23`, `noop.rs:9`
  (impls)
- Modify: `server/src/feed/worker.rs:159, 167-172, 214-221, 233-256`

**Interfaces:**

- Produces:
  `async fn send_publish(&self, hub_url: &AbsoluteUrl, feed_url: &AbsoluteUrl) -> Result<(), WebSubError>`
  — consumed by task 5.
- Produces: `process_feed_group(.., hub_url: Option<&AbsoluteUrl>, ..)` and
  `ping_websub(feed_url: &FeedPath, .., hub_url: Option<&AbsoluteUrl>, ..)`.

- [x] **Step 1: Change the trait**

`server/src/websub/mod.rs`:

```rust
use common::absolute_url::AbsoluteUrl;

#[async_trait]
pub trait WebSubClient: Send + Sync {
    async fn send_publish(
        &self,
        hub_url: &AbsoluteUrl,
        feed_url: &AbsoluteUrl,
    ) -> Result<(), WebSubError>;
}
```

- [x] **Step 2: Update the three production impls**

`http.rs:37` — signature, plus two reads at the `reqwest` boundary (spec D1's
sanctioned external flatten). `reqwest`'s `IntoUrl` is sealed with impls only
for `Url`, `String`, `&str`, `&String`, so `&AbsoluteUrl` is not accepted; use
the unambiguous `&**hub_url` rather than leaning on inference through
`post::<U: IntoUrl>`:

```rust
async fn send_publish(
    &self,
    hub_url: &AbsoluteUrl,
    feed_url: &AbsoluteUrl,
) -> Result<(), WebSubError> {
    let form = [("hub.mode", "publish"), ("hub.url", feed_url.as_ref())];
    let res = self
        .client
        .post(&**hub_url)
        .timeout(self.timeout)
        .form(&form)
        .send()
        .await
        // ... error mapping and status handling unchanged
```

(`feed_url.as_ref()` is safe here — the array's element type is pinned by the
sibling `&str` literal.)

`file_capture.rs:23` — **signature only.** `serde_json::json!` takes the values
via `Serialize`, which the macro emits as `serialize_str(&self.0)`, so the body
is untouched and `websub.jsonl` is byte-identical.

`noop.rs:9` — signature only:

```rust
async fn send_publish(
    &self,
    _hub_url: &AbsoluteUrl,
    _feed_url: &AbsoluteUrl,
) -> Result<(), WebSubError> {
    Ok(())
}
```

- [x] **Step 3: Un-flatten the worker's read and signatures**

`worker.rs:159`: `hub_url.as_deref()` → `hub_url.as_ref()`.

```rust
async fn process_feed_group(
    &self,
    feed_path: FeedPath,
    recs: Vec<FeedEventRecord>,
    hub_url: Option<&AbsoluteUrl>,
    identity: Option<&common::site::SiteIdentity>,
) { .. }

async fn ping_websub(
    &self,
    feed_url: &FeedPath,
    ids: &[FeedEventId],
    attempt: i32,
    hub_url: Option<&AbsoluteUrl>,
    identity: Option<&common::site::SiteIdentity>,
) { .. }
```

`ping_websub` **keeps `Option`** — it owns the no-hub branch at `:264-268` that
records `PingOutcome::NoHub` and calls `mark_pinged`. That branch does not move
and its behavior does not change (spec D2, AC4). The call at `:201` already
passes `&feed_path`, so it is unchanged.

- [x] **Step 4: Fix the worker body**

`compose(base, feed_url)` at `:233` still compiles — `FeedPath` derives
`StrNewtype`, so `&FeedPath` coerces to `&str`. `send_publish(hub, &absolute)`
at `:236` now passes `&AbsoluteUrl` for both.

The four `tracing` macros at `:234, 240, 248, 256` record `feed_url` and `hub`
as bare identifiers, which requires `tracing::Value`; neither newtype implements
it. Switch both to `Display`:

```rust
tracing::info!(feed_url = %feed_url, hub = %hub, attempt, "feed.websub.ping.attempted");
```

Apply the same at `:240` (`succeeded`), `:248` (`exhausted`, no `attempt`), and
`:256` (`failed`, which also has `error = %e`). **Field names and event names
must not change** — they are the observability contract.

- [x] **Step 5: Verify the library compiles**

Run: `devtool run -- cargo check -p jaunder --lib` Expected: **PASS.**
Production code is now internally consistent. `--all-targets` still fails (task
5); that is expected and is the only remaining breakage.

---

### Task 5: Test retype — fixtures and the two test-crate impls

**Files:**

- Modify: `server/src/websub/http.rs` `#[cfg(test)]` mod — delete one test,
  re-fixture four call sites, comment `HubForm`
- Modify: `server/src/websub/file_capture.rs:61,65,87,100`, `noop.rs:21`,
  `mod.rs:57-76`
- Modify: `server/tests/helpers/websub_capturing.rs:6-10, 28-40`
- Modify: `server/tests/feed/feed_worker.rs:17-27`

**Interfaces:**

- Consumes: task 4's trait signature.
- Produces:
  `CapturedPing { pub hub_url: AbsoluteUrl, pub feed_url: AbsoluteUrl }`.

- [x] **Step 1: Delete the obsolete test and re-fixture `websub/*.rs`**

Delete `returns_http_error_for_invalid_url_scheme` (`http.rs:108-116`) entirely
(spec D6). Its subject — reqwest rejecting an unparseable URL — is unreachable
now, and scheme rejection is covered by `rejects_non_http_schemes`
(`common/src/absolute_url.rs:106-115`); its `WebSubError::Http(_)` arm stays
covered by `returns_http_error_on_connection_refused`.

Add `use common::test_support::parse_absolute_url;` to each test module.

`http.rs` — **four** call sites build the hub URL from a bound address
(`posts_form_body_to_hub_on_success`, `returns_hub_refused_on_4xx`,
`returns_http_error_on_connection_refused`,
`returns_timeout_when_hub_does_not_respond`), formerly lines 123, 138, 173, 186:

```rust
c.send_publish(
    &parse_absolute_url(&format!("http://{addr}/")),
    &parse_absolute_url("https://example.com/feed.rss"),
)
```

`HubForm` is **unchanged**; add the comment (spec AC6):

```rust
// Deliberately String, not AbsoluteUrl: this is the wire decoder on the test hub,
// so it must record exactly what was posted. A validating field would turn a
// malformed send into an axum form rejection this test never sees, instead of a
// readable assertion diff. ADR-0063 §5 (wire-decoding doubles).
#[derive(Debug, Deserialize, Clone)]
struct HubForm { .. }
```

`file_capture.rs:61,65,87,100` — wrap both literals in `parse_absolute_url(..)`.
The literals contain `~`, unreserved and preserved by normalization, so the
`assert_eq!(first["feed_url"], "https://site/~alice/feed.rss")` assertions at
`:74-75` hold unchanged.

`noop.rs:21` — `"https://feed"` normalizes to `"https://feed/"`; the test only
asserts `Ok`, so nothing else moves:

```rust
c.send_publish(
    &parse_absolute_url("https://hub"),
    &parse_absolute_url("https://feed"),
)
```

`mod.rs` — reshape the HTTP arm (spec D6, AC11) and wrap the capture arm's
literals at `:70`:

```rust
// None ⇒ the live HTTP client fails on an unreachable hub. Port 1 on loopback
// has no listener, so the connect is refused immediately — no DNS, no egress.
let http = default_client(None);
assert!(
    http.send_publish(
        &parse_absolute_url("http://127.0.0.1:1/"),
        &parse_absolute_url("https://example.com/feed.rss"),
    )
    .await
    .is_err()
);
```

- [x] **Step 2: Type `CapturedPing` and the two test-crate impls**

`server/tests/helpers/websub_capturing.rs`:

```rust
use common::absolute_url::AbsoluteUrl;

#[derive(Debug, Clone)]
pub struct CapturedPing {
    pub hub_url: AbsoluteUrl,
    pub feed_url: AbsoluteUrl,
}

#[async_trait]
impl WebSubClient for CapturingWebSubClient {
    async fn send_publish(
        &self,
        hub_url: &AbsoluteUrl,
        feed_url: &AbsoluteUrl,
    ) -> Result<(), WebSubError> {
        self.pings
            .lock()
            .expect("mutex not poisoned")
            .push(CapturedPing {
                hub_url: hub_url.clone(),
                feed_url: feed_url.clone(),
            });
        Ok(())
    }
}
```

(`AbsoluteUrl` derives `Clone` and `Debug`, so both the field types and the
`#[derive]` above hold.)

`server/tests/feed/feed_worker.rs:21` — signature only, body verbatim:

```rust
async fn send_publish(
    &self,
    _hub_url: &AbsoluteUrl,
    _feed_url: &AbsoluteUrl,
) -> Result<(), WebSubError> {
    // preserve the existing body exactly
}
```

- [x] **Step 3: Confirm the ping assertions need NO edit**

The `.pings()` assertions live at `feed_worker.rs:110-118`
(`worker_pings_hub_when_configured`) and `:158-163`
(`worker_groups_duplicate_events_into_single_regen`). All of them compile
unchanged: the `StrNewtype` trailer emits `PartialEq<str>` **and**
`PartialEq<&str>`, the `ends_with` calls resolve through `Deref<Target = str>`,
and the `{}` format arg uses the generated `Display`. The stored hub value
`"https://hub.example.com/"` is already normalized, so no trailing-slash shift
applies.

**Expected outcome: zero lines changed in those two test bodies.** If you find
yourself editing them, stop — something upstream is wrong.

- [x] **Step 4: Compile all targets**

Run: `devtool run -- cargo check -p jaunder --all-targets` Expected: PASS.

- [x] **Step 5: Run the server suite**

Run: `devtool run -- cargo nextest run -p jaunder` Expected: PASS — in
particular `feed::feed_worker`'s ping assertions and the `PingOutcome::NoHub`
coverage.

- [x] **Step 6: Commit tasks 4 and 5 together**

One compile unit, one commit. Gate once over both:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-688-absoluteurl-websub -- cargo xtask check
```

Expected: PASS. Then stage, then commit (**`jaunder-commit`** — no pathspec
commit):

```bash
git add server/src/websub/ server/src/feed/worker.rs \
        server/tests/helpers/websub_capturing.rs server/tests/feed/feed_worker.rs
git commit -m "refactor(websub): AbsoluteUrl through send_publish and the worker (#688)"
```

---

### Task 6: Amend ADR-0063 §5

**Files:**

- Modify: `docs/adr/0063-domain-value-newtype-convention.md:402-406`

**Interfaces:** none.

- [x] **Step 1: Append the carve-out paragraph**

After the existing "**Sole carve-out — external types.**" paragraph, add:

```markdown
**The same carve-out covers test doubles that _decode the wire_** — an axum
`Form`/`Json` extractor in a spawned test hub, a capture-file parser. Decode
into the primitive and validate explicitly in the test body: a validating field
turns a malformed send into a transport-layer rejection the test never observes,
instead of a readable assertion diff. Instances: `HubForm`
(`server/src/websub/http.rs`) and `Resp` (`server/tests/web/web_auth.rs`), whose
production counterpart `web/src/auth/api.rs LoginResponse` _is_ typed. This does
**not** extend to in-process doubles, which receive already-typed values and
take the newtype (`CapturedPing`).
```

The heading now covers two cases — reword it to "**Carve-outs — external types
and wire decoders.**" so the text is not self-contradictory.

- [x] **Step 2: Check ADR status and cross-references**

ADR-0063 is amended, not superseded. Confirm its status line still reads
Accepted and that `docs/adr/README.md`'s table needs no change — this is an
amendment to an existing numbered ADR, so **no `cargo xtask adr promote`**.

- [x] **Step 3: Format and commit**

```bash
devtool run -- prettier -w docs/adr/0063-domain-value-newtype-convention.md
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-688-absoluteurl-websub -- cargo xtask check
git add docs/adr/0063-domain-value-newtype-convention.md
git commit -m "docs(adr): ADR-0063 §5 covers wire-decoding test doubles (#688)"
```

---

### Task 7: Full gate and acceptance sweep

**Files:** none (verification only).

**Interfaces:** none.

- [x] **Step 1: Run the full local gate**

Spec AC14 requires the **full** `validate`, with e2e — the spec makes a claim
about the e2e WebSub capture path that `--no-e2e` cannot test. Long, cold run:
use Bash background mode.

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-688-absoluteurl-websub -- cargo xtask validate`
Expected: PASS, all four `{sqlite,postgres}×{chromium,firefox}` combos.

If an e2e WebSub fixture needs updating, **stop and investigate** — per the
spec's Verification section that signals the serialization changed, which this
plan asserts it does not. Do not patch the fixture.

- [x] **Step 2: Walk the spec's 14 acceptance criteria**

```bash
git diff wt-base-issue-688..HEAD --stat
```

AC1 five impls · AC2 no `.as_deref()`, both signatures, `&FeedPath` · AC3 no
re-derive on the production path · AC4 `NoHub` intact · AC5 `CapturedPing` typed
**and the ping assertions untouched** · AC6 `HubForm` field types unchanged +
comment · AC7 ADR §5 paragraph · AC8 RSD `&AbsoluteUrl` + escaping kept · AC9
`&`-escaping test, no `&lt;` assertion · AC10 scheme test gone · AC11
`127.0.0.1:1` · AC12 issue body corrected · AC13 issue numbers in the spec ·
AC14 green.

- [x] **Step 3: Confirm the wire is unchanged**

```bash
git diff wt-base-issue-688..HEAD -- server/src/websub/file_capture.rs
```

Expected: the `serde_json::json!` block and the `hub.mode`/`hub.url` form keys
are untouched — only the signature changed.

- [x] **Step 4: Hand off to `jaunder-ship`**

No commit. Any fixes found in steps 2–3 go in as their own commit before
shipping.

---

## Execution notes — where reality differed from the plan

Recorded so the diff is readable against the plan rather than silently
divergent.

1. **Tasks 3, 4 and 5 landed as one commit (`30bf4c58`), not two.** The plan had
   task 3 committing separately. The pre-commit gate reads the **working tree**,
   not the index, and task 4's edits were started while task 3's commit was
   still running its gate — so the gate saw a half-applied trait change and
   failed. Once the tree contained tasks 3–5, splitting them would have meant
   committing a tree the gate had never checked, which is the invariant that
   matters more than commit granularity. Lesson for the next cycle: do not edit
   while a gate runs.

2. **Task 3's "Expected: FAIL" was wrong.** The rewritten RSD tests passed
   against the _old_ `&str` signature, because `&AbsoluteUrl` deref-coerces to
   `&str` at a call site. No test can pin this signature change — it is a
   type-level refactor whose only verifier is the compiler at the call sites
   being constrained. The red step was skipped rather than faked.

3. **`cargo nextest run -p jaunder` (task 5 step 5) is not runnable bare** in
   this worktree: three `case_2_postgres` tests fail with `ConnectionRefused`
   because no local Postgres is listening, and nextest's fail-fast then abandons
   the run at 137/1402. The gate supplies Postgres via Nix, so
   `cargo xtask check` is the real checkpoint. Prefer it over a bare nextest for
   anything backend-touching.

4. **Task 1 filed five issues, not four** — `on_regen_failure` (#880) was added
   once task 4 made the asymmetry concrete.

5. **Zero edits were needed** to `feed_worker.rs`'s ping assertions, as
   predicted; `--all-targets` compiled without touching them.

## Self-Review

**Spec coverage:** AC1→T4,T5 · AC2→T4 · AC3→T4 · AC4→T4 · AC5→T5 · AC6→T5 ·
AC7→T6 · AC8→T3 · AC9→T3 · AC10→T5 · AC11→T5 · AC12→T2 · AC13→T1 · AC14→T7.
D1→T4,T5 · D2→T4 · D3→T5 · D4→T6 · D5→T3 · D6→T5. No spec section is unmapped.

**Placeholders:** none — every step carries the real signature, the real test,
or the real command. The one deliberate deferral is task 1's "check for
overlap," whose output is issue numbers, not code.

**Type consistency:** `send_publish(&AbsoluteUrl, &AbsoluteUrl)` is written
identically in T4 and T5. `Option<&AbsoluteUrl>` is used for both
`process_feed_group` and `ping_websub` in T4. `CapturedPing`'s field types in T5
match spec D3. `FeedPath` appears only in T4, consistent with the corrected D2.

**Checkpoint honesty:** every "Expected: PASS" is reachable at that point — T4
step 5 (`--lib`) after the worker is retyped, T5 step 4 (`--all-targets`) after
the fixtures. No step claims a green that the tree cannot produce.
