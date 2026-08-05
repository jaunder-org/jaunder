# Spec — #830: a blank `PostTitle` becomes unrepresentable

- Issue: [#830](https://github.com/jaunder-org/jaunder/issues/830)
- Milestone: Code quality ratchet
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (amended by this issue — see D6)
- Siblings: [#811](https://github.com/jaunder-org/jaunder/issues/811) (blank
  `PostBody`), [#754](https://github.com/jaunder-org/jaunder/issues/754)
  (`summary_label` stored vs derived), superseded
  [#758](https://github.com/jaunder-org/jaunder/issues/758)
- Date: 2026-08-05

## Problem

`PostTitle` is `#[str_newtype(infallible)]` but has an invariant — non-blank —
that the codebase enforces everywhere except in the type.
`PostTitle::from("   ")` is `PostTitle("")`, and the derived `Deserialize`
routes through that same door.

The full evidence is in the issue. What matters for the design:

- Three call sites re-derive the rule, one of which
  (`storage/src/posts.rs:111-119`) re-checks a title **already read back out of
  the database**, calling it "its one invariant gap".
- `PostSummary::truncated` carries `debug_assert!(!trimmed.is_empty())`
  (`common/src/post_summary.rs:59-62`) that it cannot enforce, and
  `fallback_summary_label` filters on its behalf.
- Two feed renderers don't guard at all (`common/src/feed/atom.rs:32`,
  `common/src/feed/rss.rs:21`) and would emit an empty `<title>`.

`None` (untitled post) is a legitimate domain state. `Some(PostTitle(""))` is
nonsense. Only the type can tell them apart, and today it doesn't.

## Decisions

### D1 — `PostTitle` becomes a validating newtype

Hand-written `FromStr` that **trims, then rejects empty / whitespace-only**,
with the derive's standard validating trailer and serde/sqlx bridges.
`SessionLabel` (`common/src/session_label.rs:35-45`) is the template.

`#[str_newtype(infallible)]` and the hand-written `From<String>` are removed.

**No length cap is added.** Non-blankness is the invariant this issue is about;
a maximum-length bound is a separate decision with its own migration and UX
questions. `PostTitle`'s `FromStr` therefore rejects on emptiness alone.
(Contrast `SessionLabel`, which caps at 255 — the difference is deliberate and
noted in the doc comment.)

### D2 — A blank _submitted_ title still means absent

Today a blank/whitespace title yields an untitled post (slug derived from the
body). That behavior is **preserved**: the wire shape is already
`Option<PostTitle>`, an untitled post is a legitimate state, and this matches
the `Option<SessionLabel>` idiom at `web/src/auth/api.rs:54`.

So the blank→`None` filters at `common/src/render.rs:607-609` and
`server/src/atompub/mapping.rs:87-89` **stay** — but they change meaning: they
are now a deliberate _presence policy_ (blank input means the user supplied no
title), not a workaround for a missing invariant. Their comments must say so.

This is not a user-visible behavior change.

**Construction must be respelled — `PostTitle::from` ceases to exist.** The
`Default` kind's trailer emits `TryFrom<String>` only; `From<String>` and the
`From<&str>` alias are both infallible-kind-only
(`macros/src/str_newtype.rs:273-280`). Every current construction site therefore
stops compiling, and this spec fixes the replacement rather than leaving the
implementer to invent one:

| Site                                | Today                            | After                                      |
| ----------------------------------- | -------------------------------- | ------------------------------------------ |
| `common/src/render.rs:621`          | `PostTitle::from(title.clone())` | `title.parse::<PostTitle>()`, fall through |
| `server/src/atompub/mapping.rs:89`  | `PostTitle::from(raw)`           | `raw.parse::<PostTitle>().ok()`            |
| `server/src/atompub/mapping.rs:572` | `.map(PostTitle::from)` (test)   | `.map(\|t\| parse_post_title(&t))`         |
| `common/src/test_support.rs:246`    | `PostTitle::from(...)`           | `.parse().expect(...)` (AC9)               |

**`Err` policy: never panic in production.** At `render.rs:621` the value is
provably non-blank (the explicit branch is filtered at `:607-609`; both
`extract_markdown_title` and `extract_org_title` guarantee a non-empty result),
but the compiler cannot see it. Rather than `expect`, a failed parse **falls
through to the untitled path** — the same outcome the filter already produces
for a blank explicit title. The invariant is then enforced by the type with no
reachable panic.

At `mapping.rs:89`, `parse().ok()` makes the surrounding
`(!raw.trim().is_empty())` guard redundant. Keep exactly one of them: **drop the
hand-rolled guard and let parse-or-`None` be the presence policy**, since that
is this issue's whole point — one rule, living in the type. D2's "the filter
stays" applies to `render.rs:607-609`, which does additional work (it decides
whether the body is consulted for a title at all, `:614`), not to `mapping.rs`'s
guard.

**Where titles actually come from** — this scopes D2 sharply, so it is recorded
rather than assumed. There is **no post-title input**; a title is never a field
the user fills in as a title. Three routes:

| # | Route                             | Outermost type carrying the title                       | Blank possible?                          |
| - | --------------------------------- | ------------------------------------------------------- | ---------------------------------------- |
| 1 | Web composer / editor             | **none** — `PostInputs` has no title field; `api.rs:181`/`:321` pass `title: None` | N/A — no title can be supplied |
| 2 | Derived from the body             | `PostBody`, via `extract_markdown_title` / `extract_org_title` | No — both reject empty-after-trim   |
| 3 | AtomPub `<title>`                 | atom `Text` → `&str` → `Option<PostTitle>` at `mapping.rs:89` | **Yes** — a blank element can be sent |

The web form collects body, format, summary, publish-at, tags, audience, and an
optional slug override — no `Field::<PostTitle>` exists anywhere in `web/`,
`csr/`, or `client/`. So the storage service's `title: Option<&'a str>`
(`storage/src/post_service.rs:271`) is non-`None` **only from AtomPub**
(`server/src/atompub/posts.rs:373`, `:495`).

Consequences for this spec:

- `PostTitle` is not an inbound wire-arg type, so AC2's strictness bites on
  response/seed _decode_, and AC4's "not a 400" is true because there is no title
  arg to reject — not because a rejection path was chosen.
- **A blank title is submittable only through raw AtomPub XML.** The Emacs client
  already filters blanks (`elisp/jaunder-org.el:136`) and omits the element
  (`elisp/jaunder-atom.el:48`), so this is a hand-rolled-client case.
- That single reachable route is why D2's presence policy belongs in the type:
  today it is enforced by two independent convention-based checks
  (`mapping.rs:89` and `render.rs:607-609`), and `render.rs`'s is already
  redundant-in-practice because AtomPub dropped the blank first. Two guards for
  one rule, on one reachable path, neither of them the type.

### D3 — Strict decode; no migration

Stored rows are decoded strictly through the validating bridge: a `title = ''`
row fails to decode rather than being leniently coerced. **No data migration and
no CHECK constraint** — confirmed with the owner that there is no production
data to accommodate.

This is the same posture #811 takes for `PostBody` ("there is no legacy data to
accommodate"). Note that migration `0010_nullable_post_titles.sql` shows blank
titles _did_ accumulate historically and were swept to `NULL`; with no live
deployment that history is not a constraint on this design, and it is recorded
here only so a future reader does not mistake the absence of a migration for an
oversight.

No lenient `from_lossy`-style door is added. There is no caller that needs one.

### D4 — `PostSummary::truncated` takes a typed non-blank seed

The `debug_assert` is replaced by a type. A new `SummarySeed` newtype carries
"this string is already known non-blank", and its constructors are **infallible
because each source proves the invariant**:

```rust
impl SummarySeed {
    /// A slug is non-blank by construction (`Slug::from_str` rejects empty).
    pub fn from_slug(slug: &Slug) -> Self;
    /// A `PostTitle` is non-blank by construction (D1).
    pub fn from_title(title: &PostTitle) -> Self;
    /// The first non-blank line of a body, pre-truncated. `None` when the body has
    /// no such line — the only fallible source, and its `None` is a real domain
    /// answer (an empty body), not a validation failure.
    pub fn first_body_line(body: &PostBody) -> Option<Self>;
}

impl PostSummary {
    pub fn truncated(seed: &SummarySeed) -> Self;  // no debug_assert
}
```

`SummarySeed` lives in `common/`, alongside `PostSummary` — it needs `Slug`,
`PostTitle`, and `PostBody`, all of which are in `common/`.

`first_body_line` also absorbs the first-non-blank-line search and the 100-char
pre-truncation currently inlined in `fallback_summary_label`
(`storage/src/posts.rs:104-110`), so the rule for "what part of a body can seed
a summary" lives next to the type that consumes it.

**Not a general-purpose `NonBlankStr`.** A shared non-blank wrapper would be
broader vocabulary than this issue should introduce (and would need its own
ADR); a validating constructor would also force the fallback chain to handle a
`Result` it can prove unreachable.

**There is no arbitrary-`&str` door**, so every existing `truncated(&str)`
caller must be re-seeded. All three:

1. `storage/src/posts.rs:121` — the production chain, rewritten by D5.
2. `storage/src/test_support.rs:1731` — `PostSummary::truncated("excerpt")`
   becomes
   `PostSummary::truncated(&SummarySeed::from_title(&parse_post_title("excerpt")))`.
3. `common/src/post_summary.rs:116-123` —
   `truncated_trims_and_caps_at_char_boundary` calls `truncated("  hi  ")` and
   `truncated(&"é".repeat(550))`; both re-seed via `from_title`.

**`from_title` is the only unbounded seed**, and that is load-bearing:
`first_body_line` pre-truncates at 100 chars and `Slug` is capped at 80, so a
title is the only source that can exceed `MAX_POST_SUMMARY_CHARS` (500). D1 adds
no title cap, so the 500-cap stays reachable and its test stays meaningful — it
must be re-seeded, not deleted.

**`first_meaningful_line` is deliberately left alone.**
`common/src/render.rs:630-636` is byte-identical in shape to the body-line
search `first_body_line` absorbs, but it answers a different question (what
seeds a _slug_, gating the empty-post check) and is consumed by
`derive_post_title`. Two rules that coincide today are not one rule; folding
them together would couple the slug seed to the summary seed. Left as-is, noted
so a reviewer does not read it as a missed dedup.

### D5 — `fallback_summary_label` loses its defensive filter

With D1 and D4, all three fallback candidates are non-blank by construction, so
`storage/src/posts.rs:111-119`'s `.filter(|t| !t.trim().is_empty())` — and the
comment naming the invariant gap — are deleted. The chain becomes a
`SummarySeed` selection: body line → title → slug, with the slug arm total.

### D6 — ADR-0063 is amended (draft this cycle)

Two paragraphs become false:

1. **§3 (`:345-351`)** defines the infallible kind as "a value whose invariant
   never rejects" and names `PostBody`/`PostTitle` as its first users. The
   definition is a test on the _constructor's signature_, while §2 (`:89`)
   frames it invariant-first ("fallible **when the value has an invariant**").
   That mismatch is what let `PostTitle` be mislabelled. The amendment corrects
   the definition to be invariant-first and drops both types from the
   first-users list.
2. **§2's truncating-door paragraph (`:281-294`)** describes `truncated` as a
   trust door guaranteeing "only the cap, not [non-emptiness]". D4 replaces that
   trust with a typed proof; the paragraph is updated to describe the seed
   pattern as the preferred alternative where the caller can supply one. The
   tail of that paragraph (`:290-294`) is the part D4 most directly invalidates
   — it cites the `debug_assert!` as the mechanism and names `PostSummary`
   (#545) as the first user, "whose derived fallback summary label is built from
   a post's body line, title, or slug".

The amendment covers **`PostBody` as well as `PostTitle`**, so #811 inherits a
correct ADR rather than re-deriving the same reasoning. Consequence, stated
plainly: between this issue landing and #811 landing, §3 will not list
`PostBody` as an infallible-kind user even though `PostBody` is still
infallible. That is the accepted cost of writing the correction once; #811's own
amendment obligation is thereby discharged.

Drafted numberless in `docs/adr/drafts/` per `jaunder-adr`; numbered at ship by
`cargo xtask adr promote`.

### D7 — #758 is closed, not implemented

A validating `PostTitle` moves onto the **fallible** bridge, which #746 already
gave a `&'r str` decode. The double allocation #758 described disappears with no
`normalizing` flag, no `BridgeSpec` change, and no derive change. This is
asserted as an acceptance criterion (AC7) so the benefit is pinned rather than
assumed.

## Acceptance criteria

- **AC1** `PostTitle` has a hand-written `FromStr` that trims and rejects empty
  / whitespace-only, and no `#[str_newtype(infallible)]`, no hand-written
  `From<String>`. Unit tests pin: trims outer whitespace, preserves inner
  whitespace and case, rejects `""`, `"   "`, `"\t\n"`.
- **AC2** A blank title is rejected **on the wire**:
  `serde_json::from_str::<PostTitle>("\"\"")` and `"\"  \""` are both errors,
  mirroring
  `session_label_serde_serializes_as_plain_string_and_validates_on_deserialize`.
- **AC3** A blank title is rejected **on decode**: a `title = ''` row fails to
  read, pinned by a test that forces the row in via raw SQL (the pattern of
  `reading_post_with_overlong_summary_in_db_errors`,
  `storage/src/posts.rs:3437`). No migration and no CHECK constraint are added.
- **AC4** A blank _submitted_ title still yields an untitled post, not a 400.
  Pinned on the one route that can submit one — **AtomPub**: `POST`ing an entry
  whose `<title>` is whitespace-only creates a post with `title: None` and a
  body-derived slug. Plus a `derive_post_title` unit test passing
  `Some("   ")` as `explicit_title` and asserting the untitled outcome. (There is
  no web-form equivalent to test: the composer has no title input — see D2.)
- **AC5** `SummarySeed` exists in `common/` with exactly the three constructors
  of D4; `from_slug` and `from_title` are infallible, `first_body_line` returns
  `Option`. `PostSummary::truncated` takes `&SummarySeed` and contains **no
  `debug_assert`**.
- **AC6** `fallback_summary_label` contains no emptiness filter and no
  "invariant gap" comment; its behavior is unchanged — the existing
  `fallback_summary_label_prefers_body_then_title_then_slug` test still passes
  for the body/title/slug precedence. Its case 2b (empty-after-trim title falls
  through to slug) is **deleted**, because the state it constructs is no longer
  representable; the spec's AC3 test replaces the coverage. The test's own
  fixtures at `:2954`, `:2981`, and `:2996` are converted per AC9.
- **AC7** `PostTitle`'s sqlx decode borrows `&'r str` — asserted in the macros
  unit tests alongside
  `validating_bridge_decodes_a_borrowed_str_without_allocating`, or by an
  equivalent assertion that `PostTitle` now uses the validating bridge. This is
  #758's benefit, pinned.
- **AC8** No production code path can produce `Some(PostTitle(""))`, so a
  **titled** feed entry cannot render an empty title. RSS is fully fixed:
  `rss.rs:21` is `.title(i.title.clone().map(String::from))`, so `None` omits
  the element. **Atom is only half fixed and deliberately so** — `atom.rs:32` is
  `Text::plain(i.title.clone().map(String::from).unwrap_or_default())`, so an
  _absent_ title still emits `<title></title>`. That is a pre-existing defect of
  the `None` case, independent of this issue's invariant, and it is out of scope
  (see below). AC8 is delivered when the `Some("")` case is unrepresentable —
  not when `atom.rs` stops emitting empty titles entirely.
- **AC9** `common/src/test_support.rs`'s `parse_post_title` becomes a validating
  helper (`.parse().expect(...)`), matching `parse_session_label` (`:120-129`),
  and its doc comment no longer claims `PostTitle` is infallible. The
  grep-checkable form of the criterion: **no `PostTitle` is constructed anywhere
  except via `FromStr`/`TryFrom` or `parse_post_title`** — no `PostTitle::from`,
  no `.map(PostTitle::from)`, no bare `.into()` producing a `PostTitle`. (~21
  fixture sites, not the ~42 the issue estimated; that figure counted
  `SiteTitle` and plain-`String` title fields too.)
- **AC10** An ADR-0063 amendment draft exists in `docs/adr/drafts/` covering
  both changes of D6 (§3 invariant-first definition dropping `PostTitle` _and_
  `PostBody`; §2's truncating-door paragraph describing the seed pattern).
- **AC11** `PostSummary::truncated`'s `truncated_debug_asserts_non_empty` test
  (`common/src/post_summary.rs:125-130`) is deleted — the state it asserts on is
  no longer constructible. Its neighbour
  `truncated_trims_and_caps_at_char_boundary` (`:116-123`) is **kept and
  re-seeded** via `from_title`, so the 500-scalar cap stays exercised.
- **AC12** `cargo xtask validate` green, including the coverage gate and all
  four `{sqlite,postgres}×{chromium,firefox}` e2e combos.

## Out of scope

- Any length cap on `PostTitle` (D1).
- `PostBody`'s blankness — that is #811.
- Whether `summary_label` becomes a stored column — that is #754. If it lands as
  _stored_, `SummarySeed` will need a read/decode counterpart; nothing here
  forecloses that.
- A general-purpose `NonBlankStr` newtype (D4).
- **What Atom emits for an _untitled_ entry.** `common/src/feed/atom.rs:32`'s
  `unwrap_or_default()` renders `<title></title>` when the title is `None`. RFC
  4287 requires `atom:entry/atom:title`, so "omit it" is not a free fix — the
  entry needs a substitute (permalink? summary label?), which is a
  feed-semantics decision, not a type-invariant one. Independent of this issue
  and unaffected by it; **the plan's first task files it** so the other half of
  the issue's empty-`<title>` observation is not lost.
- Adding `PostTitle`/`PostSummary` to more input/view structs — that is #694,
  and landing it first would enlarge this issue's conversion surface.

## Coordination note — #811

#811 is claimed and has a worktree, but **no commits** (only an uncommitted plan
file), so there is no code to conflict with today. Both issues touch
`common/src/render.rs`'s `derive_post_title` and `storage/src/post_service.rs`.
#811 also plans to make `derive_post_title` total, returning
`(Option<PostTitle>, Slug)`. Nothing in this spec blocks that — D2 keeps the
blank→`None` filter, which #811 can carry into the total signature unchanged.
