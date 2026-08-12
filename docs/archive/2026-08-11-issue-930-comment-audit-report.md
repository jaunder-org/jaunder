# Findings report — issue #930 comment audit

Outcome catalogue for the comment audit, grouped by defect class (spec:
`docs/archive/2026-08-11-issue-930-comment-audit-spec.md`). The
outcome sections summarize at area level; the per-site file:line detail
is the appendix at the end (the pre-edit inventory, whose entries were
each verified in context during the edit passes). Cross-check any
claimed edit against the appendix plus `git log --follow -p` on the
area commit.

One knowingly reverted edit: an assert *message string* in
`storage/src/posts.rs` had been retensed along with the comments; a
failure message is code, so it was restored verbatim (spec non-goal:
"No code changes beyond comments").

## 1. Backward-looking

**server — done (all inventory sites verified in context and edited):**
tombstone comments deleted or rewritten as present-tense pointers
(`tests/storage/mod.rs`, `cli.rs`, `commands.rs`, `main.rs`, `misc/commands.rs`,
`smtp.rs`, `web_auth.rs`, `web_backup.rs`, `site.rs`, `feed_worker.rs`,
`feed/worker.rs`); history clauses dropped with intent kept (~40 sites across
src + tests); "now"-tense fixes applied; the `commands.rs` cov:ignore block
trimmed _within_ the block (ADR-0094 adjacency preserved).

**storage — done:** all inventoried tombstones deleted, history parentheticals
dropped, "no longer parses" retypings rewritten present tense (incl. the four
near-identical `helpers.rs` homing notes, deduplicated to two), dialect twins
edited in both files, doc-comment histories compressed (`posts.rs:103-108` six
lines → three, `:296-304` upsert rationale retensed, `:2164-2169`, `:875-877`).

**common + macros — done:** atompub delta narration replaced with ADR-0089
citations; media.rs history parentheticals/tense fixed (11 sites); render.rs,
backup.rs, pagination.rs, feed_path.rs, visibility.rs,
slug/text/time/seed/invite/post_body retensed; post_summary.rs test tombstone
deleted; macros provenance and vacuity histories rewritten (`lib.rs` doctest
prose shortened without touching fences — ADR-0095).

**e2e/test-support/CI/root — done:** e2e helper and spec history retensed (~35
sites: mail/websub/polling/capture-trace/selectors/posts/
seed/fixtures/helpers/feeds/auth/audiences/posts.spec/media.spec/
password_reset/playwright.config); "Preview is gone" ×4 → "canonical permalink
is the only route"; step-narration one-liners deleted; `flake.nix` history
clauses dropped and the leptosfmt override promoted; `.rustfmt.toml` compressed;
`Cargo.toml` lettre essay → draft pointer (deny.toml twin points at the same
draft); `scripts/git-add` restatements deleted; test-support retensed.

**xtask + tools — done:** Cluster A (dangling citations to the deleted Node
script) stripped across traces/, audit_wasm, nix_build, lib.rs; Cluster B
provenance one-liners deleted (tools/devtool, tools/coverage, flaky.rs, git.rs,
files.rs, static_checks.rs); Cluster C "replaces-that-test" comments retensed
(sqlx_newtype_decode_check, html_sink_check, raw_html_door_check,
server_fn_registrar_check, adr_readme, coverage/report,
server_fn_coverage/extract); Cluster D module-doc history paragraphs retensed
(coverage/exempt, ident_gate, rendered_html_from_trusted_check,
server_fn_coverage extract/snapshot, sqlx_newtype_bind_check).

**web/csr/client/host — done:** ~70 tense fixes and history-clause deletions
across posts/, app/, auth/, error/, forms/, taglist, tags/, timeline/, cockpit/,
home/, sidebar/, media/, subscriptions/, audiences/, route_segments, html.rs,
feed_events, host/error.rs, host/token.rs, client/. `csr/` untouched
(gate-load-bearing).

## 2. Overlong

**web/csr/client/host — done:** `app/render.rs:96-107` preload essay → 3-line
pointer at `no-wasm-preload` draft; `WASM_URL` doc retensed;
`forms/component.rs` retracting paragraph → pointer at
`labelled-takes-erased-signals` draft; `media/format.rs` float comparison
compressed to the live rule; `host/error.rs` conversions banner compressed.
Kept: `posts/component.rs:167-208` coincidence blocks (prose trimmed only near
markers), `web/src/posts/server.rs` workaround note, `app/render.rs:149-154`.

**common + macros — done:** `render.rs:190-226` essay → 4-line pointer at the
new `rendered-html-storage-decode` draft; `media.rs:1822-1835` → 7 lines in
place; `media.rs:1218-1225`, `:1711-1718` trimmed; `tests/str_newtype.rs`
circularity paragraph deleted. Kept: `entry.rs:40-58` (namespace semantics),
`sqlx_bridge_derive.rs:46-60`, `macros/src/lib.rs:1405-1414`, media.rs
check-order blocks — all dense why.

**storage — done:** `posts.rs:1256-1263` deduplicated; `:446-454` kept (homing
rationale, judgment call); `media.rs` delete-guard block trimmed;
`helpers.rs:852-861` → 3-line pointer at the ADR draft.

## 3. ADR-worthy (promotions)

Note for reviewers: ADR drafts live in `docs/adr/drafts/` and are gitignored
until `cargo xtask adr promote` runs at ship — they will not appear in the PR
diff.

Drafts written so far (all in `docs/adr/drafts/`, gitignored until ship):

- `rstest-reuse-cross-module-templates.md` — pointers at
  `server/tests/atompub/atompub_rsd.rs` (×2), `server/tests/storage/mod.rs`.
- `slug-ordered-tag-lock-acquisition.md` — pointer at `storage/src/posts.rs`
  (#876).
- `sqlx-sqlite-busy-handler-threading.md` — pointer at `storage/src/posts.rs`.
- `one-bad-row-must-not-stop-the-scan.md` — pointers at `storage/src/posts.rs`
  (×2), `storage/src/media.rs`, `storage/src/feed_events.rs`.
- `clear-then-load-restore.md` — pointers at both backup dialect files.
- `absent-user-timing-equalization.md` — pointers at `storage/src/helpers.rs`
  (×2).
- `rendered-html-storage-decode.md` — pointers at `common/src/render.rs` and
  `macros/src/lib.rs` (retargeted prose pointer).
- `no-wasm-preload.md` — pointers at `web/src/app/render.rs` (×2).
- `labelled-takes-erased-signals.md` — pointer at `web/src/forms/component.rs`;
  records the open question the comment used to carry.
- `coverage-probe-dirty-tree-workaround.md` — pointer at
  `xtask/src/coverage/probe.rs`.
- `no-endpoint-drift-check.md` — pointer at
  `xtask/src/server_fn_coverage/snapshot.rs`.
- `lettre-fork-pinned-by-rev.md` — pointers at `Cargo.toml` and `deny.toml`.
- `leptosfmt-pinned-past-release.md` — pointer at `flake.nix`.

Kept in place (not promoted): `storage/src/helpers.rs:449-453`
(`match`-not-`unreachable!` A1-guard idiom — short enough inline).

## 4. Redundant

**server — done:** bare `// Expected` arms collapsed, `// ...` placeholder
deleted, restating step labels deleted (~10; the rest carried intent and
stayed).

**storage — done:** `posts.rs` assert-restatement cluster replaced with one
intent comment; `media_manager.rs` empty-dir/file labels deleted, scenario
headers kept, misleading "verify hard link" comment corrected to name the length
proxy.

## Judgment calls

- server: several flagged "step label" comments were kept after in-context
  review because they carry intent the assertions don't (e.g.
  `tests/storage/mod.rs` "Create a draft (should not appear)", "Second create
  with same slug+date conflicts", "// Group by feed_path to avoid redundant
  regeneration"). Deleted only pure restatements.
- server: `tests/storage/mod.rs` empty match arms
  `Ok(None)/Err(..) => { // Expected }` collapsed to `=> {}` — a textual code
  change with identical semantics, accepted as the mechanical knock-on of
  deleting the comment.
- server: the rstest_reuse spike prose (atompub_rsd.rs, storage/mod.rs) was
  promoted to `docs/adr/0124-rstest-reuse-cross-module-templates.md` and the
  three sites now point there.
- server: `tests/storage/mod.rs:4433` rewrite also fixed a factually wrong
  sibling comment (":4420 said the post id doesn't exist; it is soft-deleted").
- common: `visibility.rs:16-23` (#746 D12 serde trade) retensed in place, not
  promoted — ADR-0075 and the D12 spec decision already record it.
- common: `media.rs` CR/LF acceptance shortened in place (7 lines), not promoted
  — scoped to one type, per the audit's lean.
- web: `tags/api.rs` PageSize-over-TagLimit rationale (#691) kept in place
  rather than drafted — scoped to one fn's API doc; the audit offered either.
- web: `viewer.rs:32-36` forward-looking "Layer C insertion point" ladder and
  `posts/server.rs:176-181` workaround note kept as-is.
- root: `Cargo.toml`'s "deliberately no `panic = 'abort'`" block (#836) kept in
  place, not drafted — 8 tight lines whose whole content is the why for an
  absence, adjacent to the profile it governs.
- e2e: `bootBudget.ts` orphan-allowance blind-spot paragraph kept in place — it
  interlocks with the numbered rules around it; `boot-marks.spec.ts`
  mutation-proof lab note kept (house "Teeth:" idiom, proves the guard can
  fail).
- xtask: `steps/nix.rs` EACCES remove-then-copy note kept in place (audit
  offered promotion) — tightly local to the code it governs; likewise the
  `reduce-otel-capture.mjs` "THE TRAP" block and the two parallel
  `adr_readme.rs` skip-case teeth comments (audit suggested deduping one).
- storage: the sqlx-bridge trait-bound sentence repeated at 18 sites was LEFT
  AS-IS (only its history parenthetical trimmed at one site). Each copy is a
  compliant why-comment at its own site; collapsing to one canonical statement
  - pointers is a structural refactor of comment placement worth its own small
    follow-up, not a silent sweep inside this audit.
- server: the "lived here" tombstones were rewritten to keep the pointer
  (present-tense: where the contract test is homed) and drop the framing.

## Left alone deliberately

Confirmed during the edit passes (originally seed-discovery
observations, each re-verified in context in its area's pass):

- `// Binds: $1 …` placeholder-mapping comments in `storage/src/posts.rs` (~20×)
  — data mapping prose onto runtime-assembled SQL, unreadable from code. Keep.
- Section dividers (`// ---- x ----`, `// ── x ──`) — navigation in
  2000–6000-line files. Keep.
- `Deliberately …`, `Pinned so …`, `X, not Y:` idioms — compliant why-comments,
  not defect signals.
- Elisp `AC-217a:`-style spec tags — traceability data.
- `xtask/src/steps/*` module `//!` docs incl. ADR-0085 honesty prose — long by
  policy.
- Present-tense "no longer expressible" type-level facts
  (`storage/src/posts.rs:3053-3056`, `:3756`; `server/tests/storage/mod.rs:359`)
  — compliant despite matching backward-looking seeds.

## Totals

Approximate edit counts per defect class (sites, not lines), from the six area
passes:

| Area                  | Backward-looking | Overlong | ADR-promoted  | Redundant |
| --------------------- | ---------------- | -------- | ------------- | --------- |
| server                | ~60              | 5        | 1             | ~15       |
| storage               | ~50              | 6        | 5             | ~12       |
| common + macros       | ~60              | 8        | 1             | 3         |
| web/csr/client/host   | ~70              | 5        | 2             | 8         |
| xtask + tools         | ~60              | 3        | 2             | 0         |
| e2e/test-support/root | ~35              | 4        | 2             | ~25       |
| **Total**             | **~335**         | **~31**  | **13 drafts** | **~63**   |

Clean areas: `csr/` (untouched by design), `elisp/`, `.githooks/`, `.github/`
workflows, `client/` (near-clean), `clippy.toml`, `rust-toolchain.toml`.

## Verdict on the standard

**"Comment for intent, not mechanics" (CONTRIBUTING.md, Code conventions) is
well-calibrated for the defect it names — and this tree proves people follow
it.** Redundant restatement, the ❌ case in its worked example, was rare outside
test step-narration (~63 sites in ~28,000 comment lines), and pure why-comments
— the standard's ✅ case — are the house norm and were overwhelmingly excellent.

**It is not sufficient, because it does not name the defect that actually
dominates: arguing against the past.** ~335 sites — five times the redundancy
count — were comments that justify current code by contrast with a previous
implementation, a deleted test, or a removed file ("used to", "no longer",
"replaces `old_test`", "the old parser accepted…"). This shape passes the
current standard's letter: it reads like a why-comment, because it _does_
explain a choice. What makes it a defect is that the contrast is unverifiable —
its other half lives only in `git log` — so it rots into archaeology (the
sharpest case: 30+ comments citing line numbers in a file deleted ~900 commits
ago). The standard also says nothing about length, though overlength was rare
and usually a symptom: a multi-paragraph inline essay almost always turned out
to be an unrecorded decision (13 were promoted to ADR drafts).

**Proposed addition** (to sit directly after the existing bullet; not applied —
for the maintainer to adopt or edit):

> - Comment against the present, not the past. Write intent and why as claims
>   about the code as it stands, verifiable by reading the tree. Do not argue
>   with a previous implementation, a deleted test, or a removed file ("used
>   to", "no longer", "replaces `old_fn`"): the other half of that contrast
>   lives only in `git log`, so the comment is unverifiable the day it is
>   written and archaeology a year later. Keep the issue number; drop the
>   narrative. Two carve-outs: a present-tense fact that happens to use
>   past-shaped words ("a blank title is no longer expressible — the type
>   forbids it") is fine, and backward _compatibility_ with live data ("markers
>   written before #591 lack `is_operator`") is a statement about the present
>   world, not about code history.
> - A comment that outgrows a few sentences is usually a decision record. If it
>   carries a rejected alternative, a measured trade, a workaround's root cause,
>   or an accepted risk, promote it to an ADR (draft in `docs/adr/drafts/`) and
>   leave a one-line pointer. Module-level `//!` docs are comprehensive by
>   design and exempt from this pressure; inline comments should be as short as
>   a true statement of intent allows.

No mechanical enforcement is proposed: the false-positive rate is high (this
audit's own grep seeds flagged many compliant present-tense facts), so the rule
belongs in review culture, not a gate.

---

# Appendix: pre-edit working inventory (subagent candidates)

Retained as the audit trail of what the per-area auditors flagged. The outcome
sections above supersede it: every entry was verified in context during its
area's edit pass and either edited or recorded under "Judgment calls" / "Left
alone deliberately".

## storage (audit-storage)

**ADR-worthy (promote + path pointer):**

- `storage/src/posts.rs:394-402` — slug-ordered `tags` row locks avoid a
  Postgres deadlock (#876). The `sort_by_key`-not-`sort_unstable` half stays
  inline.
- `storage/src/posts.rs:2914-2923` — sqlx-sqlite per-connection OS thread makes
  busy-handler blocking safe.
- `storage/src/posts.rs:1935-1944`, `:3101-3104`,
  `storage/src/media.rs:310-314`, `storage/src/feed_events.rs:57-67` —
  skip/purge-one-bad-row-rather-than-fail-the-scan, restated 4×; one ADR + four
  pointers.
- `storage/src/postgres/backup.rs:101-106` +
  `storage/src/sqlite/backup.rs:87-90` — clear-then-load restore because
  `SET CONSTRAINTS` defers FK checks but not `ON DELETE CASCADE` actions.
- `storage/src/helpers.rs:826-829, :840-844, :852-861` — dummy-Argon2 timing
  equalization + production-only parity admission.
- `storage/src/helpers.rs:449-453` — `match` not `unwrap_or_else(unreachable!)`
  to avoid the A1-guard; promote or keep.

**Backward-looking (rewrite present-tense or delete):**

- Tombstones ("Behavior-preserving translation of the former `web` test"):
  `posts.rs:3968-3970`; `post_service.rs:1384-1385, :1410-1412`;
  `users.rs:822-823, :837-838`; `atomic.rs:255-256`; `audiences.rs:401-403`.
  Delete leading clause, keep the assertion intent.
- Deletions: `post_service.rs:633-636` ("retired here");
  `feed_events.rs:357-360` ("lived here and is gone"); `posts.rs:3062-3064`
  (pure "Before #728…").
- History parentheticals (drop parenthetical only):
  `posts.rs:1192-1193, :1051-1053, :2224-2225, :2871-2872, :3318-3320, :3763-3765`;
  `users.rs:713-716`; `site_config.rs:998-999`; `feed_cache.rs:49-53`;
  `backup.rs:256-258`; `invites.rs:229-230`; `media.rs:431-435, :276-280`;
  `helpers.rs:69-72, :255-259, :844`.
- "No longer parses" retypings:
  `helpers.rs:575-580, :615-618, :620-623, :703-708`;
  `posts.rs:3050-3051, :3128-3129, :3988-3990`;
  `post_service.rs:608-610, :988-990, :1068-1071`; `smtp.rs:249-254`;
  `post_service.rs:253-256 + :419-422` (identical block 2×, dedupe).
- Dialect twins (edit both files): `sqlite/posts.rs:118-121` /
  `postgres/posts.rs:120-123`; `sqlite/posts.rs:163` / `postgres/posts.rs:148`
  (", as before"); `sqlite/posts.rs:182-184` / `postgres/posts.rs:169-171`;
  `sqlite/posts.rs:153-158` (TOCTOU phrasing).
- Doc comments with the defect:
  `posts.rs:103-108, :296-304, :875-877, :903, :2170-2175`;
  `helpers.rs:291-293`; `media.rs:150-154`; `backup.rs:31`; `atomic.rs:80-81` +
  `email.rs:34-35` (identical sentence, delete both);
  `feed_events.rs:61-63, :89-90`; `smtp.rs:51-54`;
  `test_support.rs:1240, :1255`.

**Overlong (shorten; detail → ADRs):**
`posts.rs:1264-1271, :2342-2349, :3656-3664, :3755-3765, :446-454`;
`media.rs:342-352`; `helpers.rs:852-861`.

**Redundant (delete):** `posts.rs:3943, :3946, :3950, :3957, :3964`;
`media_manager.rs:546, :554, :577, :586, :602, :607` (keep `:549` — pure why).

**Judgment call:** sqlx-bridge sentence at 18 sites
(`posts.rs:1022, :1030, :1035, :2221, :2229, :2342, :2395, :2574`;
`media.rs:183, :194`; `users.rs:237`; `sessions.rs:141`; `invites.rs:83`;
`email.rs:104`; `feed_cache.rs:86`; `feed_events.rs:228`; `audiences.rs:159`;
`password.rs:81`; `site_config.rs:339`) — canonical statement (ADR-0071
exists) + per-site one-liners. Also `posts.rs:1012-1014` = `users.rs:232-234`
verbatim.

## common + macros (audit-common)

**Backward-looking — atompub cluster (ADR-0089 already exists; cite it, don't
retell):** `entry.rs:6-8` (delete), `:382, :496, :534-538, :579-581, :787-788`
(delete), `:1219-1220`; `rsd.rs:87-88` (delete clause); `xml.rs:7-9`.

**Backward-looking — rest of common:**
`backup.rs:23-25, :101, :200, :245-248, :265-267, :328-329`; `config_key.rs:4`;
`feed/event_status.rs:51-53`;
`feed/feed_path.rs:172-175, :281-283, :286, :288, :297-298`; `invite.rs:40`;
`pagination.rs:38, :51-52, :186-188, :236-243` (first line only);
`post_body.rs:19-20`; `post_summary.rs:199-204` (delete whole block);
`seed.rs:155-157`; `slug.rs:125, :134`; `text.rs:14`; `time.rs:127`;
`visibility.rs:19-21, :59, :214`;
`render.rs:49-51, :612, :1050-1052, :1165-1169` (keep newline-significance),
`:1761-1763` (keep last clause), `:1832-1833` (→ "(spec D2)");
`pg_role_password.rs:18-20` (rewrite present tense).

**Backward-looking — media.rs:** `:20-23, :288-290, :720-721, :867-871` (delete
parenthetical), `:1013, :1218-1225, :1371, :1398-1400, :1715-1718, :1734-1737`
(keep the trap), `:1822-1835` (shorten hard).

**Backward-looking — macros:** `macros/Cargo.toml:14`;
`macros/src/lib.rs:163-167` (ADR-0095: shorten prose only, never split/reflow
fences); `server_fn.rs:3-6, :81-83`; `sqlx_bridge_derive.rs:223`;
`str_newtype.rs:589-592`; `macros/tests/str_newtype.rs:240-250` (delete 2nd
para).

**Overlong:** `render.rs:190-226` (→ ADR); `entry.rs:40-58`;
`visibility.rs:6-23` (para 3 ADR-worthy);
`media.rs:279-294, :344-351, :519-525, :1218-1225, :1711-1718, :1822-1835`;
`sqlx_bridge_derive.rs:46-60`; `macros/src/lib.rs:1405-1414` (modest); keep
`sqlx_bridge.rs:86-94`.

**ADR-worthy:** (1) `render.rs:190-226` → draft `rendered-html-storage-decode`;
knock-on: `macros/src/lib.rs:473-474` prose pointer must retarget to the draft
path. (2) `visibility.rs:16-23` — the #746 D12 one-convention trade (promote or
pointer to ADR-0075/#746). (3) `media.rs:1822-1835` — lean shorten-in-place. (4)
atompub cluster → cite existing ADR-0089.

**Redundant:** `media.rs:568` (delete), `:1099` (trailing line only);
`backup.rs:322` (low-confidence delete); keep `media.rs:1363`.

**Left alone (common/macros):** gate markers + `test_support/mod.rs:6-8`
(`#![expect]` adjacency); `session_user.rs:27, :71-73` (backward-compat about
live data — keep); hypotheticals (`smtp_port.rs:14` etc.);
`stored_password_hash.rs:22-30`; item `///` essays (API doc content); mailer
dividers; `feed/window.rs:103-107`; `render.rs:381-382`; no TODOs anywhere.

## xtask + tools (audit-xtask)

Headline: near-zero redundancy; defect mass is backward-looking, in four
clusters.

**Cluster A — dangling citations to deleted `scripts/analyze-otel-traces` (#33,
~32 lines):** `xtask/src/traces/analyze.rs` 16 hits
(`:84, :149, :162, :173, :199, :218, :228, :238, :260, :295, …`);
`traces/parse.rs:106, :135, :154, :163, :182, :200`;
`traces/render.rs:16, :457`; `traces/boot_phases.rs` 3 hits;
`audit_wasm.rs:46-48`; `nix_build.rs:12, :23` — drop the `(Node … :NNN)`
parentheticals / "old script" comparability claims, keep the rule.
Port-provenance headers: `traces/mod.rs:3-5`, `traces/parse.rs:4`,
`traces/run.rs:8`, `xtask/src/lib.rs:324, :356` — delete "Faithful Rust port of
…" phrase.

**Cluster B — provenance one-liners (delete clause, keep rest):**
`tools/devtool/src/provision.rs:14, :86`; `tools/devtool/src/pg.rs:2`;
`tools/coverage/src/pathnorm.rs:3`; `tools/devtool/src/csr_bundle.rs:10` (keep
"host and Nix cannot drift"); `tools/devtool/src/seed_e2e.rs:3` → "One list,
applied by both callers"; `xtask/src/steps/flaky.rs:12`; `xtask/src/git.rs:109`;
`xtask/src/files.rs:4-8` (keep "copies of one rule rot apart");
`xtask/src/steps/static_checks.rs:52` (delete — `scripts/` gone).

**Cluster C — "replaces that test" in step tests (rewrite present-tense, keep
issue numbers):** `sqlx_newtype_decode_check.rs:1815, :1910, :2093, :2481`;
`html_sink_check.rs:149, :355`; `raw_html_door_check.rs:157, :301`;
`server_fn_registrar_check.rs:494`; `adr_readme.rs:516, :686`;
`coverage/report.rs:157, :293` (CARE: cov:ignore marker text in string literals
nearby); `server_fn_coverage/extract.rs:279`.

**Cluster D — module-doc history paragraphs (rewrite present tense):**
`coverage/exempt.rs:20-26` (+ test comment `:176` → pointer);
`ident_gate.rs:25-29, :54-57, :66`;
`rendered_html_from_trusted_check.rs:26-42, :64-67`; `html_sink_check.rs:80-84`;
`server_fn_coverage/extract.rs:14-20`; `server_fn_coverage/snapshot.rs:126-130`;
`sqlx_newtype_bind_check.rs:25-28, :69-76`. Keep (fixture data is the subject):
`sqlx_newtype_bind_check.rs:411`, `server_fn_tracing_check.rs:499`,
`web_server_fns.rs:316`.

**Overlong:** `server_fn_coverage/snapshot.rs:205-220` (→ ADR);
`adr_readme.rs:998-1008` vs `:953-960` (dedupe to pointer);
`coverage/probe.rs:148-156` (→ ADR).

**ADR-worthy:** (1) `coverage/probe.rs:148-156` CI shallow-clone workaround
(dirty an excluded file so nix copies the working dir); (2)
`server_fn_coverage/snapshot.rs:205-220` deliberate removal of the
endpoint-drift check; (3) `steps/nix.rs:236-245` read-only nix-store EACCES +
remove-then-copy protocol. Borderline keep-in-place:
`server_fn_coverage/testdata/reduce-otel-capture.mjs:54-69` "THE TRAP".

**Redundant:** none found.

**Left alone (xtask/tools):** ADR-0085 honesty prose (`ident_gate.rs:73-120`
etc.); module `//!` length; all marker-adjacent comments and marker text in
string literals; fixture literals; `test_support.rs:14-23` (explains
grep-evasion assembly); the "moved/renamed tree can never quietly disable the
guard" formula (7 sites — present-tense fail-closed property); Cargo.toml
dependency rationale.

## end2end / test-support / elisp / scripts / CI / root (audit-misc)

**Backward-looking (rewrite/trim; full table in agent message):**
`.rustfmt.toml:1-12`;
`flake.nix:465, :476, :513, :629-632, :1032-1035, :1127, :1160-1161`;
`test-support/tests/cli.rs:8, :41-42`; `test-support/src/lib.rs:129`;
`end2end/tests/mail.ts:21-23`; `websub.ts:22-24`; `polling.ts:4-10`;
`capture-trace.ts:4-19, :525-529`; `selectors.ts:1-9`; `posts.ts:5-9, :85-88`;
`seed.ts:70-71`; `fixtures.ts:9, :428-430, :469-474, :586-588, :952-956`;
`helpers.ts:51`; `feeds.ts:31-32, :39-41`; `feeds.spec.ts:255-259`;
`auth.spec.ts:89-91, :187`; `audiences.spec.ts:59-61, :241`;
`posts.spec.ts:111, :318, :506, :554` ("Preview is gone (#24)" ×4 → "canonical
permalink is the only route"), `:625-627` (delete), `:1123-1126` (delete last
sentence), `:1321-1322`, `:1379-1383`;
`media.spec.ts:79-83, :275-278, :293-294`; `password_reset.spec.ts:35-38`;
`playwright.config.ts:6-7`.

**Overlong:** `bootBudget.ts:257-293` (37-line JSDoc → two failure kinds;
`:279-287` limitation essay → ADR); `boot-marks.spec.ts:102-117` (keep
silent-failure rationale, drop lab notes); `flake.nix:624-640, :1027-1043`; keep
`flake.nix:1336-1351`; keep `.githooks/pre-commit:2-11` (optional trim).

**ADR-worthy:** (1) lettre fork — `Cargo.toml:118-133` + `deny.toml:248-253`
(duplicated only records); (2) rejected `panic = "abort"` — `Cargo.toml:21-28`;
(3) `flake.nix:413-458` leptosfmt override (keep `REMOVE THIS OVERRIDE`
trigger + #420 in place); (4) `bootBudget.ts:279-287` orphan-allowance blind
spot. Leave in place: `flake.nix:1266-1271` (debuginfo SIGBUS).

**Redundant (delete):** `scripts/git-add:6, :11, :16`; `flake.nix:903, :911`
(keep `:888`); `admin-site.spec.ts:34, :37, :40, :49`;
`posts.spec.ts:335, :523, :793, :796, :802, :810, :1011, :1056, :1075, :1078, :1081, :1091, :1165, :1176, :1186, :1208, :1268, :1275`;
`feeds.spec.ts:68, :98`; `auth.spec.ts:182, :199`; `audiences.spec.ts:99, :113`.
Keep-despite-shape: `posts.spec.ts:532, :1400`; reword `:775`/`:1262` to "there
is no title input".

**Left alone (misc):** e2e-goto-wrapper:allow lines (no rewrap);
`deny.toml:1-246` vendored template commentary (stays diffable against
upstream); elisp entirely; `test-support/src/main.rs:25-96` (clap help text);
module docs; duplicated Nix/CI sibling blocks (needs code motion, out of scope);
`flake.nix:958` (only record of why combo ids can't renumber — keep);
`timeline-cls.spec.ts:13-15` (validity argument — keep).

## web / csr / client / host (audit-web)

Headline: ~75 backward-looking sites, mostly tense fixes of correct rationale; 5
overlong; 8 redundant; 3 ADR-worthy; no TODOs.

**Backward-looking (full per-site actions in agent message):**
`web/src/posts/component.rs:198-200, :256-257, :627-629, :744, :841, :885-886, :1111-1113, :1213-1218`;
`posts/api.rs:461-462`; `posts/mod.rs:33-34, :75`;
`posts/page_state.rs:389-390, :421-422`; `posts/compose_state.rs:9, :212`;
`app/component.rs:76-77`; `app/render.rs:56-65`; `auth/api.rs:10`;
`auth/server.rs:15-17`; `auth/component.rs:107-108`; `auth/marker.rs:8`;
`error/server.rs:92-94, :153-157, :454, :532-536`;
`forms/component.rs:19-26, :91, :169`; `forms/field.rs:107-109, :361-363`;
`taglist/mod.rs:4-10`; `tags/api.rs:5, :18, :34-35`; `tags/input_logic.rs:4-6`;
`tags/input_state.rs:4-6`; `timeline/mod.rs:7-9, :14`;
`timeline/state.rs:6-7, :20-22, :30-32, :54`; `timeline/api.rs:90-91`;
`timeline/server.rs:286` (delete); `cockpit/component.rs:6` (delete), `:64-65`;
`cockpit/mod.rs:4`; `home/component.rs:64-65`; `sidebar/component.rs:58`;
`media/api.rs:190, :254-255`; `media/component.rs:26`;
`media/format.rs:77-88, :165-169`; `media/upload_state.rs:291`;
`html.rs:7, :10-11`; `lib.rs:2`; `email/component.rs:83`;
`profile/component.rs:19-20`; `subscriptions/component.rs:112-114`;
`subscriptions/state.rs:35-39`; `route_segments.rs:2`;
`audiences/component.rs:84-85, :195, :296-297`; `feed_events.rs:34`;
`host/src/error.rs:308-310, :344-353, :386, :565-566, :670-673`;
`host/src/token.rs:86-88`; `client/src/lib.rs:18-19, :23, :40, :45`;
`client/src/dom.rs:29`; `client/src/upload.rs:2`.

**Overlong:** `app/render.rs:96-107` (→ ADR + 3-line pointer);
`media/format.rs:70-88`; `forms/component.rs:8-26` (delete retracting 3rd
paragraph); `host/src/error.rs:344-353`; `posts/component.rs:167-208` (prose
only — ADR-0094 markers at :191, :207).

**ADR-worthy:** (1) `app/render.rs:96-107` — no wasm `<link rel=preload>`
(measured, abort rule, rejected alternative); (2) `forms/component.rs:13-26` —
erased signals not `Field<T>`; (3) `tags/api.rs:32-39` — `PageSize` over
`TagLimit` newtype (#691).

**Redundant (delete):** `app/component.rs:70, :100`; `posts/api.rs:292, :501` (+
dedupe `:194-195` vs `:350`); `profile/api.rs:7, :16`; `client/src/dialog.rs:4`.

**Left alone (web):** all gate markers + neighbours (esp. `html.rs:60-67` —
would otherwise shorten, ADR-0094 forbids); all of `csr/`;
`app/render.rs:149-154`; dividers; `viewer.rs:32-36` (forward- looking ladder,
flagged); `posts/server.rs:176-181` (workaround note, borderline ADR, keep);
`error/mod.rs:79`; `client/` perf/storage/reactive exemplary.

## server (audit-server)

**Backward-looking — test tombstones:**

- `server/tests/storage/mod.rs:3904, :4058` — "lived here" → rewrite to
  present-tense pointer; `:5081` — delete (replacement directly above).
- `server/src/cli.rs:479` — delete "Replaces" sentence only (why-comment at
  `:482-485` stays); `:509` — delete whole; `:519` — rewrite present-tense.
- `server/src/commands.rs:855` — shorten to pointer; `server/src/main.rs:345` —
  drop replaces-narrative, keep intent; `server/tests/misc/commands.rs:104`
  (keep pointer), `:371` (rewrite).
- `server/src/mailer/smtp.rs:377` (rewrite, keep why), `:396` (delete), `:415`
  (delete 1st sentence).
- `server/tests/web/web_auth.rs:369, :930` — delete; `web_backup.rs:448` —
  delete sentence; `server/src/site.rs:348` — delete;
  `server/tests/feed/feed_worker.rs:166` — shorten;
  `server/src/feed/worker.rs:619` — rewrite present-tense.

**Backward-looking — history clauses (delete clause, keep intent):**
`server/tests/storage/mod.rs:711, :1099, :3459, :6055-6068` (14 lines → present
rule); `server/src/cli.rs:830`; `server/src/commands.rs:648` (inside a
`cov:ignore-start` block — trim within, never split), `:687`;
`server/src/observability.rs:418, :868`; `server/src/atompub/posts.rs:87, :152`;
`atompub/mapping.rs:376`; `smtp.rs:145`;
`server/src/media.rs:24, :158, :480, :690`; `server/src/site.rs:125, :255`;
`server/src/main.rs:37, :174`; `server/tests/main.rs:1` (keep `#![expect]`
justification); `web_posts.rs:97, :291, :346, :396` (also overlong),
`:927, :2165`; `atompub_posts.rs:719, :1432`; `web_auth.rs:478, :549`;
`audiences.rs:319, :457, :507`; `web_media.rs:332, :348, :366, :427, :647`;
`web_sessions.rs:202`; `web_backup.rs:395`; `web_account.rs:373`;
`web_email.rs:22, :42` + `web_password_reset.rs:34, :54` (drop "now", 4×);
`server_fn_wire.rs:98`; `backup_fixture.rs:138, :221`; `feed_worker.rs:176`;
`server/src/atompub/media.rs:30`.

**Overlong:** `atompub_rsd.rs:13-27` + `:76` (rstest_reuse spike essay — see
ADR-worthy); `server/src/media.rs:258-275` (→ rule + ADR-0080 pointer);
`web_posts.rs:1311-1322`; `server/src/site.rs:465-480` (keep "Deliberately
unguarded" para); `server/src/commands.rs:648-668` (ADR-0094 caution).

**Redundant (delete):** `tests/storage/mod.rs:4013, :4483, :4675` (bare "//
Expected"); `web_account.rs:461` (`// ...`); `src/main.rs:333-335`;
`web_posts.rs:1843-1846, :1730`; plus ~45 low-value step labels (itemized in the
audit-server message: tests/storage/mod.rs ×14, web_posts ×7, atompub_posts ×4,
atompub_media ×2, web_password_reset ×6, web_auth ×2, feed tests ×6,
src/feed/worker.rs ×2).

**States-no-intent rewrite:** `tests/storage/mod.rs:4433-4434` ("might fail or
succeed depending on implementation") + `:4420` (factually wrong "doesn't exist"
— the post is soft-deleted). Rewrite against the actual assertion.

**ADR-worthy:** rstest_reuse cross-module `#[template]`/`#[apply]` resolution
rule — `atompub_rsd.rs:13` (+ `:76`) and `tests/storage/mod.rs:32`. One draft,
point all sites.

**Left alone (server):** present-tense "no longer" type facts
(`tests/storage/mod.rs:359, :1364, :1900, :1939, :3667`,
`tests/helpers/mod.rs:125`); the 11-line helper rationale at
`tests/storage/mod.rs:52-62`; all gate markers (~30 guard:no-backend etc.);
section banners; `src/media.rs:206` milestone TODO (flagged for user, not a
defect class); `web_tags.rs:70` forward-looking scope note (possibly stale —
flagged); `cli.rs:393` + `commands.rs:755` ("legacy data" = present-tense data
fact); `atompub_media.rs:197` grep false positive; module `//!` docs clean of
backward-looking phrasing.

**Correction:** the "3943-3964 four restatements" calibration item belongs to
`storage/src/posts.rs` (found by audit-storage), not
`server/tests/storage/mod.rs`.

**Left alone (storage):** `// reason:` blocks; guard/cov markers;
`media.rs:780-788` ADR-0085 honesty prose; `posts.rs:3053-3056, :3756`;
`// Binds:` blocks; dividers; per-site-useful repeats ("hides scheduled posts"
9×, mockall `'a` 9×, ADR-0053 TempDir 6×, `Option::as_ref` 6×);
`sqlite/feed_events.rs:144-153` (old shape IS the #18 repro's subject);
`media.rs:522-528` (why a test does not exist);
`sqlite/mod.rs:193-201, :206-213`; `posts.rs:2930-2936, :3790-3796`;
`lib.rs:3-9`.
