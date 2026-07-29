# Spec — #511: consistent `#[tracing::instrument]` on every `web` server fn

- Issue: [#511](https://github.com/jaunder-org/jaunder/issues/511)
- Milestone: Observability & diagnostics
- Date: 2026-07-29
- Status: awaiting approval

## Problem

`web/src` declares **55** `#[server]` functions. Only **11** carry a
`#[tracing::instrument]` span (auth 3, backup 3, registration 2, site 3); the
other **44** produce no top-level span, so a request into `create_post` or
`create_audience` is invisible to the tracing pipeline ADR-0011 established. The
issue named nine bare verticals; the survey found a tenth, `password_reset` (2
fns).

The 11 that exist are not a convention to extend — they disagree with each
other. Span names are derived inconsistently (`get_backup_settings` →
`web.backup.get_settings` strips the vertical noun, while
`get_registration_policy` → `web.registration.get_registration_policy` keeps
it), and two of the skip lists blank out every field on their span.

Nothing prevents the 56th server fn from shipping bare, which is how a 44-of-55
gap arose in the first place.

## Decisions

### D1 — Enforcement is an xtask gate, not a prose convention

A new static check, `server-fn-tracing`, runs alongside `server-fn-registrar` in
both `cargo xtask check` and `cargo xtask validate`. A convention note alone was
rejected: the existing gap is evidence that an unenforced convention does not
hold.

### D2 — The span name is fully derived, so it is checked by equality

`name = "web.<vertical>.<fn_ident>"`, where `<vertical>` is the **first path
segment under `web/src`** (so `web/src/posts/api/listing.rs` → `posts`) and
`<fn_ident>` is the function's identifier **verbatim**. The whole name is a pure
function of (directory, fn ident), so the gate asserts it by string equality
with no judgment left over.

A `#[server]` fn in a file directly under `web/src` (e.g. `web/src/foo.rs`) has
no vertical directory — the first path segment is a filename. That is a **hard
gate error** naming the file, not a guess at `web.foo.rs.…`. No such fn exists
today; the rule exists so the first one fails loudly instead of minting a
nonsense span name.

This produces some redundant names today — `web.audiences.create_audience`,
`web.site.get_site_identity`. That redundancy is real, and it lives in the **fn
idents**, not the span names: `create_audience` carried its noun when the
verticals shared one flat namespace, and `audiences::create_audience` now
restates it. Fixing it is a rename sweep blocked behind an ADR-0066 revision
(see D6), filed separately. Because the span name is _derived_, that sweep
improves every span name for free, with no span-name edit and no gate change.

Five existing span names change to conform:

| file               | old                          | new                                 |
| ------------------ | ---------------------------- | ----------------------------------- |
| `backup/api.rs:16` | `web.backup.warning_visible` | `web.backup.backup_warning_visible` |
| `backup/api.rs:29` | `web.backup.get_settings`    | `web.backup.get_backup_settings`    |
| `backup/api.rs:42` | `web.backup.update_settings` | `web.backup.update_backup_settings` |
| `site/api.rs:14`   | `web.site.get_identity`      | `web.site.get_site_identity`        |
| `site/api.rs:27`   | `web.site.update_identity`   | `web.site.update_site_identity`     |

The other six (`web.auth.login`, `.logout`, `.session`,
`web.registration.get_registration_policy`, `.register`,
`web.site.base_url_warning_visible`) already match the derived form.

### D3 — PII discipline is a type allowlist, default-deny

Every argument of an instrumented `#[server]` fn must be either named in
`skip(...)`/`skip_all`, or have a type on an explicit **recordable** allowlist.
An unlisted type is not recordable, so a newly-introduced argument type fails
the gate until someone classifies it — the PII decision is forced at the moment
it arises rather than deferred to a reviewer's attention.

The criterion is **"is this value already visible to the trace's reader, or
bounded by its own type?"** — not "did a user type it". Four recordable grounds:

**Bounded by the type itself** — the type admits no free text: `PostId`,
`AudienceId`, `SubscriptionId`, `ContentHash`, `PageSize`, `PageOffset`,
`RetentionCount`, `InviteTtlHours`, `UtcInstant`, `PostFormat`, `MediaSource`,
`BackupMode`, `u32`, `bool`.

**Operator configuration** — set by the operator, who _is_ the trace's audience;
ADR-0011 prohibits _user_ PII and secrets, and an operator's own settings are
neither: `DestinationPath`, `SiteTitle`, `AbsoluteUrl`, `BackupSchedule`.

**Already published** — the value is a component of a permalink that, by the
time these fns see it, addresses public content: `Slug` and `PermalinkDate`
(`get_post` resolves a _published_ post's permalink; the draft path,
`get_post_preview`, takes a `PostId` instead), and `Tag` (the public tag-listing
URL).

**Permitted outright by ADR-0011** — `Username`. The ADR states verbatim that
"usernames are public identifiers and acceptable" (`0011:47`). This is its own
ground rather than part of the published-URL one, because `login` and
`request_password_reset` take a username in a POST body, not a URL.

Everything else is skipped. Notably:

- **Secrets** — `ProfferedPassword`, `ProfferedInviteCode`, `RawToken`,
  `TokenHash`.
- **End-user data not published** — `Email` (named in ADR-0011), `Bio` and
  `DisplayName` (reachable only via `get_profile`/`update_profile`; no public
  page renders them), `AudienceName` (an author-private label), `SessionLabel`
  (the user's own device name), and `Filename`.
- **Bodies and unbounded text** — `CreatePostArgs`, `UpdatePostArgs`,
  `MultipartData`, and bare `String`, whose only two instances are `login`'s
  `label` and `list_tags`' autocomplete `prefix`.

`Filename` is skipped despite appearing in `media_url()`
(`/media/<source>/<p1>/<p2>/<sha256>/<filename>`). An earlier draft recorded it
on that ground, which does not survive the criterion: a media item's URL is only
_discoverable_ once a published post references it, so an
uploaded-but-unreferenced file's name is published nowhere — and a `Filename` is
arbitrary user text (`sanitize_filename` bounds its _shape_, not its content),
so `delete_media` would otherwise record something like `mri-results-2026.pdf`.
It fails the same test that skips `Bio`.

Two consequences recorded deliberately:

- `Bio`/`DisplayName` are skipped because nothing publishes them _today_. If a
  public `/@username` page later renders them, the classification warrants
  revisiting; the gate will not notice on its own. This caveat belongs in the
  ADR-0011 addendum (D5).
- Allowlisting `u32` leaves a narrow hole — a numeric OTP or PIN would pass.
  ADR-0063 says such a value arrives as a newtype (`OtpCode`), not a bare `u32`,
  so the newtype convention closes it. Stated in the note rather than solved
  with gate machinery.

Type matching is **syntactic**: strip `Option<…>` (repeatedly) and any leading
`&`, then take the last path segment, so `common::media::Filename` and
`Option<PostId>` reduce to `Filename` and `PostId`.

The allowlist is a `const` in the gate module, each entry carrying a one-line
justification naming its ground. The ADR-0011 addendum states the _rule_; the
gate holds the _list_, so adding a type is a code change that shows up in a
diff. Like ADR-0066's registrar guard, the requirement is **mandatory with no
per-fn opt-out** — a fn that genuinely cannot be instrumented is a reason to
revisit this spec, not to add an escape hatch.

Spans are emitted at `#[tracing::instrument]`'s default **INFO** level; no
`level` is specified. This is stated because it means operator configuration
(`destination_path`, `title`, `base_url`) reaches trace backends at INFO.
ADR-0011 permits it — that data is the operator's own — but the choice should be
explicit rather than inherited.

### D4 — The gate also inspects `fields(...)`

`skip(email)` paired with `fields(who = %email)` satisfies an argument-level
check while recording the email anyway. The gate therefore collects the
identifiers appearing in each `fields(...)` entry's **value expression** and
fails if one names an argument whose type is not recordable. This is
deliberately conservative — `fields(len = args.body.len())` is refused rather
than analyzed — so that the allowlist is the single place PII policy is decided.

The **field-name position (left of `=`) is excluded** from collection. A field
may be _named_ after a skipped argument as long as its value does not read it,
so `fields(label = "redacted")` and `fields(bio = tracing::field::Empty)` pass.
Collecting the LHS would reject both, which record nothing.

### D4a — What the gate recognizes as the attribute

- The attribute matches when its path's **last segment is `instrument`**, so
  both `#[tracing::instrument(…)]` and a bare `#[instrument(…)]` (via
  `use tracing::instrument;`) count. The repo uses the qualified form at all 11
  existing sites and this spec adds the qualified form everywhere; the bare form
  is accepted so an import-style change is not a silent gate bypass.
- A `#[cfg_attr(…, tracing::instrument(…))]` wrapper is a **hard error**: a
  conditionally-present span is exactly the inconsistency this issue exists to
  remove.
- `level`, `target`, and `parent` arguments are **tolerated** and ignored.
- `err` and `ret` are **rejected**, with a message pointing at D6 — both change
  what is recorded and `err` needs its own PII review of the `WebError`
  `Display` chain.
- A `#[tracing::instrument]` carrying **no `name`** is its own failure ("span
  name is required; tracing would default it to the fn ident, which lacks the
  `web.` prefix"), distinct from the wrong-name failure, because the defaulted
  name is invisible in source.

### D5 — The convention is recorded as an ADR-0011 addendum

ADR-0011 owns observability and its PII-discipline section is exactly what this
codifies, so this is an amendment to it (as with the metrics, CLI-flush, and
facade-relocation addenda), not a new ADR. The addendum states the span-name
rule, the four recordable grounds, the caveats from D3, and points at the gate.
Gate mechanics live in the new step's module doc.

### D6 — Two separable concerns are filed, not folded in

1. **Drop vestigial vertical nouns from `#[server]` fn idents**, which first
   requires revising **ADR-0066** to match server fns by module path rather than
   leaf type name. The rename collides broadly under leaf matching — `Create`
   (audiences, posts, invites), `Delete` (audiences, posts, media), `List`
   (sessions, invites, tags), `Update` and `Get` (posts, profile), `ListMine`
   (audiences, media) — and each collision is a hard failure of the existing
   registrar gate. The re-export problem that motivated leaf matching
   (`web/src/posts/api.rs:16` does `pub use listing::*;`) has to be solved for
   path matching to work.

   That issue must also fix a drift between ADR-0066's prose and its code: the
   ADR's _Consequences_ call leaf collision an "accepted limitation … benign"
   that could let an unregistered fn slip through — i.e. a pass — while
   `server_fn_registrar_check.rs:205–222` hard-fails it. The code is the current
   truth and this spec follows it; the ADR text is stale.

2. **`auth::login` takes `label: Option<String>`** while
   `sessions::create_app_password` takes `label: SessionLabel` — the same
   concept, one newtyped and one not (an ADR-0063 gap). Both are skipped either
   way, so this does not change the gate.

Out of scope, and _not_ filed as part of this cycle unless the user wants it:
`#[instrument(err)]` to record server-fn failures as span errors. It needs its
own PII review of the `WebError` `Display` chain, which is a different question
from span presence.

## Acceptance criteria

Each is stated so a conformance review can tell delivered from not.

1. **Every `#[server]` fn in `web/src` carries `#[tracing::instrument]`.**
   Enumerating `web/src/**/*.rs` for `ItemFn`s with a `#[server]` attribute
   yields 55 functions, and all 55 also carry a `#[tracing::instrument]`
   attribute.
2. **The attribute follows `#[server]`.** For all 55, the
   `#[tracing::instrument]` attribute appears _after_ the `#[server]` attribute
   in source order. The reason is consistency with proven-working code, not a
   claimed expansion hazard: all 11 existing sites use this order and build for
   both targets today, so it is the arrangement known to produce a server-side
   span.
3. **Every span name equals
   `web.<first path segment under web/src>.<fn ident>`.** Checked by string
   equality for all 55, including the five renames in D2.
4. **Every argument is skipped or recordable.** For all 55, each parameter is
   either named in `skip(...)`, covered by `skip_all`, or has a type whose
   reduced leaf (per D3) is on the allowlist.
5. **The two currently-blank operator write paths record their settings.**
   `update_backup_settings`'s span carries `destination_path`, `schedule`,
   `retention_count`, `mode`, and `update_site_identity`'s carries `title`,
   `base_url` — the D3 operator-configuration ground applied to the two spans
   that today record nothing.
6. **`login` and `register` skip lists are unchanged** — `skip(password, label)`
   and `skip(password, invite_code)`.
7. **A `server-fn-tracing` step exists and runs in both gates.** It appears in
   the step list of `cargo xtask check` and `cargo xtask validate`, next to
   `server-fn-registrar`, and its name appears in `.xtask/last-result.json`'s
   `steps[]`.
8. **The gate fails on a missing span.** Deleting any one
   `#[tracing::instrument]` attribute makes `cargo xtask check` fail with a
   message naming that file, line, and fn.
9. **The gate fails on a wrong span name.** Changing any span name to one not
   equal to the derived form fails with a message naming the expected and actual
   names.
10. **The gate fails on an unclassified argument type.** A `#[server]` fn whose
    argument type is neither skipped nor on the allowlist fails with a message
    naming the argument, its type, and both remedies (add to `skip(...)`, or add
    to the allowlist).
11. **The gate fails on a `fields(...)` bypass.** `skip(x), fields(y = %x)`
    where `x`'s type is not recordable fails with a message naming `x`.
12. **The gate fails loudly on an unreadable or unparseable source file**,
    rather than skipping it — a file that cannot be enumerated could hide a bare
    `#[server]` fn.
13. **The gate's pure decision function is unit-tested**, covering at minimum:
    the pass case, the failure cases of criteria 8–11, the D4a rules (bare
    `instrument` accepted, `cfg_attr` rejected, `err`/`ret` rejected, missing
    `name` its own failure), a `fields(...)` LHS named after a skipped arg
    _passing_, `Option<…>` and path-qualified type reduction, `skip_all`
    equivalence, a zero-argument fn, and a `#[server]` fn directly under
    `web/src`. Criterion 12's unreadable-file path is I/O and lives in `run()`,
    mirroring `server_fn_registrar_check.rs`; it is verified by inspection of
    that structure, not by the pure function's tests.
14. **The enumeration of `#[server]` fns is shared, not duplicated.** One helper
    yields, per fn: source path, line, raw ident, parameter names and types, and
    the attribute list. `server-fn-registrar` derives its existing
    `ServerFn { name: PascalCase(ident), line }` by mapping over that helper's
    output, so its public behaviour, failure messages, and unit tests are
    untouched; `server-fn-tracing` consumes the richer form. A conformance
    reviewer checks that neither gate walks `web/src` itself.
15. **At least one automated test observes a real span at runtime** — it invokes
    an instrumented `#[server]` fn's server-side body under a capturing
    subscriber and asserts the emitted span's name is the derived
    `web.<vertical>.<fn_ident>` and that a skipped argument's value does not
    appear among its fields. Criteria 1–14 are all source-static and would every
    one pass even if the attribute expanded somewhere that never wraps the
    server-side body; this is the criterion that would catch that.
16. **ADR-0011 carries an addendum** stating the span-name rule, the recordable
    grounds, the `Bio`/`DisplayName` and `u32` caveats, the INFO-level note, and
    a pointer to the gate.

Two further criteria are **not judgable from the diff alone** and must be
checked against their external artifacts, not skipped:

17. **Two issues are filed** per D6, with the ADR-0066 dependency stated in the
    first — verified in the tracker.
18. **`cargo xtask validate --no-e2e` is green**, and
    `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` is
    clean (the attributes compile into the wasm client stubs too) — verified by
    running them.

## Per-site outcome

The complete verdict for all 55 sites — `E` marks one of the 11 already
instrumented.

```
  audiences/api.rs:51    name = "web.audiences.create_audience", skip_all
  audiences/api.rs:65    name = "web.audiences.rename_audience", skip(name)
  audiences/api.rs:79    name = "web.audiences.delete_audience"
  audiences/api.rs:90    name = "web.audiences.list_my_audiences"
  audiences/api.rs:111   name = "web.audiences.list_my_subscribers"
  audiences/api.rs:146   name = "web.audiences.add_subscriber_to_audience"
  audiences/api.rs:163   name = "web.audiences.remove_subscriber_from_audience"
  audiences/api.rs:180   name = "web.audiences.list_audience_members"
E auth/api.rs:40         name = "web.auth.login", skip(password, label)
E auth/api.rs:113        name = "web.auth.logout"
E auth/api.rs:130        name = "web.auth.session"
E backup/api.rs:16       name = "web.backup.backup_warning_visible"
E backup/api.rs:29       name = "web.backup.get_backup_settings"
E backup/api.rs:42       name = "web.backup.update_backup_settings"
  email/api.rs:26        name = "web.email.request_email_verification", skip_all
  email/api.rs:70        name = "web.email.verify_email", skip_all
  invites/api.rs:40      name = "web.invites.create_invite", skip(recipient_email)
  invites/api.rs:90      name = "web.invites.list_invites"
  media/api.rs:65        name = "web.media.list_my_media"
  media/api.rs:103       name = "web.media.media_usage"
  media/api.rs:126       name = "web.media.delete_media", skip(filename)
  media/api.rs:200       name = "web.media.upload_media", skip_all
  password_reset/api.rs:24  name = "web.password_reset.request_password_reset"
  password_reset/api.rs:86  name = "web.password_reset.confirm_password_reset", skip_all
  posts/api.rs:161       name = "web.posts.create_post", skip_all
  posts/api.rs:243       name = "web.posts.get_post"
  posts/api.rs:283       name = "web.posts.get_post_preview"
  posts/api.rs:308       name = "web.posts.update_post", skip_all
  posts/api.rs:401       name = "web.posts.default_audience_selection"
  posts/api.rs:415       name = "web.posts.post_audience_selection"
  posts/api.rs:437       name = "web.posts.list_drafts"
  posts/api.rs:484       name = "web.posts.publish_post"
  posts/api.rs:542       name = "web.posts.delete_post"
  posts/api.rs:575       name = "web.posts.unpublish_post"
  posts/api/listing.rs:119  name = "web.posts.list_user_posts"
  posts/api/listing.rs:142  name = "web.posts.list_local_timeline"
  posts/api/listing.rs:163  name = "web.posts.list_home_feed"
  posts/api/listing.rs:284  name = "web.posts.list_posts_by_tag"
  posts/api/listing.rs:307  name = "web.posts.list_user_posts_by_tag"
  profile/api.rs:41      name = "web.profile.get_profile"
  profile/api.rs:66      name = "web.profile.update_profile", skip_all
  profile/api.rs:85      name = "web.profile.get_default_post_format"
  profile/api.rs:96      name = "web.profile.set_default_post_format"
E registration/api.rs:40 name = "web.registration.get_registration_policy"
E registration/api.rs:52 name = "web.registration.register", skip(password, invite_code)
  sessions/api.rs:33     name = "web.sessions.list_sessions"
  sessions/api.rs:63     name = "web.sessions.create_app_password", skip_all
  sessions/api.rs:77     name = "web.sessions.revoke_session", skip_all
E site/api.rs:14         name = "web.site.get_site_identity"
E site/api.rs:27         name = "web.site.update_site_identity"
E site/api.rs:54         name = "web.site.base_url_warning_visible"
  subscriptions/api.rs:21  name = "web.subscriptions.subscribe_to"
  subscriptions/api.rs:39  name = "web.subscriptions.unsubscribe_from"
  subscriptions/api.rs:59  name = "web.subscriptions.is_subscribed_to"
  tags/api.rs:31         name = "web.tags.list_tags", skip(prefix)
```

Line numbers are as of `wt-base-issue-511` and point at each `#[server]`
attribute (the same key the per-site table and the D2 rename table use); they
will drift as attributes are inserted, so the file/fn pairing is the identifying
key.

`skip_all` appears at 9 sites and has **no existing precedent in this repo** —
the 11 current spans all enumerate `skip(...)`. It is valid in the pinned
`tracing-attributes` 0.1.31 (`skip_all` landed in 0.1.18) and is used where
every argument is skipped, so the alternative would be restating the full
parameter list.

## Non-goals

- Instrumenting anything other than `#[server]` fns in `web/src` (vertical
  `server.rs` helpers, `host`, `storage`).
- `#[instrument(err)]` / `ret` — see D6.
- Renaming `#[server]` fn idents or their `endpoint = "…"` values — see D6.
- Changing the `boundary!` macro or the structured error boundary.
