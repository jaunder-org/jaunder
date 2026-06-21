# Content Visibility — Layer A: Design

Status: draft 2026-06-18
ADR: [ADR-0020](../../decisions/0020-content-visibility-and-subscription-model.md)
Beads: TBD (created during planning)

## Goal

Implement the first, fully demonstrable slice of the Channel / Subscription /
Audience model from ADR-0020: the `local` channel, local subscription, named
audiences, per-post audience targeting, and read-time enforcement on the
existing web surfaces and feeds. After this milestone a local user can mark a
post Public, Private, Subscribers, or to one-or-more named audiences, and the
website shows each viewer exactly what they are permitted to see.

This proves the entire Channel → Subscription → Audience → resolution chain with
local accounts only. ActivityPub/email delivery (Layer B) and authenticated
browsing for non-local visitors (Layer C) are out of scope.

## Data model

All tables ship as paired SQLite + Postgres migrations following ADR-0019's
dialect split. Every enumerated column is a lookup table referenced by FK (not a
CHECK), because under SQLite a CHECK cannot be altered in place — changing it
requires the 12-step table rebuild, whereas a lookup grows with a one-line
seed `INSERT` in both backends. Each lookup is guarded by a bijection test (see
Testing).

### Lookup tables

* **`channels`** — `(channel_id PK, name TEXT UNIQUE NOT NULL)`. Seeded with
  `'local'`. Future channels (`activitypub`, `email`, `at`) are added as seeded
  rows in their own migrations.
* **`subscription_statuses`** — `(status_id PK, name TEXT UNIQUE NOT NULL)`.
  Seeded with `'active'`. `'pending'` (M13 follow-approval) and `'blocked'`
  (M15 block/mute) are reserved for later seeds.
* **`target_kinds`** — `(kind_id PK, name TEXT UNIQUE NOT NULL)`. Seeded with
  `'public'`, `'subscribers'`, `'named'`. (There is no `'private'` kind — a
  Private post is one with no targeting rows; see below.)

### Core tables

* **`subscriptions`** — `(subscription_id PK, author_user_id FK→users,
  channel_id FK→channels, subscriber_ref TEXT NOT NULL, status_id FK→subscription_statuses,
  created_at)`. Unique `(author_user_id, channel_id, subscriber_ref)`. For the
  `local` channel, `subscriber_ref` is the subscriber's `user_id` (as text).
  Layer A only ever writes `status = active`.
  Carries a composite UNIQUE `(subscription_id, author_user_id)` to serve as an
  FK target (see `audience_members`).

* **`audiences`** — `(audience_id PK, author_user_id FK→users, name TEXT NOT NULL,
  created_at)`. Unique `(author_user_id, name)`. User-defined **named** audiences
  only; built-ins are never rows here. Carries a composite UNIQUE
  `(audience_id, author_user_id)` to serve as an FK target.

* **`audience_members`** — `(audience_id, subscription_id, author_user_id,
  PRIMARY KEY(audience_id, subscription_id))`. The `author_user_id` column is
  carried deliberately so that **two composite foreign keys** enforce the
  same-owner invariant *in the database*:
  * `FOREIGN KEY (audience_id, author_user_id) REFERENCES audiences(audience_id, author_user_id)`
  * `FOREIGN KEY (subscription_id, author_user_id) REFERENCES subscriptions(subscription_id, author_user_id)`

  Both point at the same `author_user_id` column, so it is structurally
  impossible to pair an audience with a subscription owned by a different author.

* **`post_audiences`** — `(post_id FK→posts, target_kind_id FK→target_kinds,
  audience_id FK→audiences NULL, PRIMARY KEY(post_id, target_kind_id, audience_id))`.
  The single targeting table. `audience_id` is non-null **iff** the kind is
  `named`. A `public` post has one `public` row; a `subscribers`/`named` post has
  the corresponding row(s); a **Private post has no rows at all**. `posts` gains
  no new column. Indexed `(target_kind_id, post_id)` so the hot anonymous-timeline
  filter stays cheap.

### Invariants

There are **no application-enforced invariants**. The same-owner rule is enforced
by composite FKs; audience composition is pure union (Public and named/subscribers
may freely coexist — adding a `public` row temporarily opens a post, and deleting
just that row reverts it to its prior audiences); Private is simply the absence of
rows. Composite FKs require `PRAGMA foreign_keys = ON` per SQLite connection — the
plan must confirm the SQLite pool sets this.

## Resolution

A single rule, expressed once and reused by every read path:

```
viewer may see post  ⇔  viewer == post.author
                        OR EXISTS a post_audiences row that admits viewer:
                             public      → always (incl. anonymous)
                             subscribers → viewer has an active subscription to post.author
                             named       → viewer is a member of that audience
```

In SQL this is an `EXISTS` over `post_audiences` (joined to `subscriptions` /
`audience_members` for the non-public kinds) folded into the `WHERE` clause of
the timeline and single-post queries. Anonymous viewers reduce to
`EXISTS (… target_kind = 'public')`.

The viewer is a **`ViewerIdentity`** (a shared type in `common`), not a bare
`Option<user_id>`:

```
ViewerIdentity = Anonymous
               | Channel { channel_id, subscriber_ref }   // local viewer = Channel{local, user_id}
```

The non-public joins match on `(channel_id = :viewer_channel AND subscriber_ref =
:viewer_ref AND status = 'active')`, and the author-sees-own shortcut fires only
when the viewer is `Channel{local, author_user_id}` (a remote viewer is never the
author). In Layer A only `Anonymous` and `Channel{local, …}` are ever constructed;
the type is deliberately wider than Layer A needs so that non-local channels (see
the Layer C design) are additive — no signature or query change when they land.
This is a schema-free change: `subscriptions` already carries `channel_id` and
`subscriber_ref`; only the trait signatures and the join predicate change.

## Storage traits

Mirroring the existing `PostStore` / `SiteConfigStore` shape, each with
dual-backend implementations and parity tests:

* **`SubscriptionStore`** — `subscribe(author, channel_id, subscriber_ref)`,
  `unsubscribe`, `list_subscribers(author)`, `is_subscriber(author, viewer: &ViewerIdentity)`.
  `subscribe` is channel-parameterized rather than local-hardcoded (Layer A only
  ever calls it with `channel = local`, but the same signature serves every later
  channel), and it routes through the **admission seam** below to decide the new
  subscription's status.
* **`AudienceStore`** — create/rename/delete named audience, add/remove member,
  `list_audiences(author)`, `list_members(audience)`.
* **`PostStore`** — extended to persist `post_audiences` on create/edit and to
  apply the resolution filter on every list/fetch path. The viewer
  (`ViewerIdentity`, see Resolution) becomes a parameter of the timeline and
  single-post queries.

### Subscription admission

Subscription creation goes through one named seam, conceptually:

```
SubscriptionPolicy::initial_status(author, channel_id, subscriber_ref) -> SubscriptionStatus
```

Layer A's implementation returns `Active` unconditionally — the no-op
auto-approve. This makes Layer A's "open subscription, no approval" an *explicit*,
single decision point shared by the local channel and every later channel, rather
than behaviour scattered per call site. M13 swaps in the per-author "open vs.
invite-only (default)" policy returning `Pending`, plus approval transitions, here
and nowhere else. `subscription_statuses` already reserves `pending`/`blocked`, and
resolution admits only `active`, so the gate is latent and **fails closed** today.

## Web surfaces

The live M5 surfaces gain viewer-aware filtering; new UI is added for
subscription and audience management.

* **Timelines & post pages** (site, user, tag, single post) — apply the
  resolution filter. Logged-out sees only Public; a logged-in local user
  additionally sees posts that admit them, plus their own.
* **Profile page** — a Subscribe / Unsubscribe action for the `local` channel.
  Subscription routes through the admission seam, which auto-approves (`active`) in
  Layer A; the approval workflow itself is M13.
* **Account area** — manage named audiences (create / rename / delete) and assign
  one's subscribers into them.
* **Post editor** — an audience picker: Public / Private / Subscribers and/or a
  multiselect of named audiences. The initial selection comes from
  `posts.default_audience`.

## Configuration

* **`posts.default_audience`** (`site_config`, default `public`) — the
  audience a post receives when none is specified, including via the AtomPub
  path.

## Feeds (M8) and AtomPub (ADR-0014/0015)

* **Published feeds** include only Public posts (the resolution rule for an
  anonymous viewer). Where M8 is already built, this is an added clause; where it
  is not yet built, M8 must adopt the rule.
* **AtomPub authoring** has no native visibility field, so posts created or edited
  via AtomPub take `posts.default_audience`. The author always sees their own
  posts in their AtomPub collection regardless of audience (owner access). Setting
  a non-default audience is done in the web UI for now; an Atom extension element
  is future work.

## Testing

Per CONTRIBUTING's backend-parity and coverage discipline:

* **Bijection tests** — for each lookup (`channels`, `subscription_statuses`,
  `target_kinds`): load all rows, map each name through its Rust enum's
  `TryFrom`, and assert the row-name set equals the enum-variant set in *both*
  directions. Adding an enum variant without a seed migration, or a seed row
  without an enum variant, fails CI.
* **Composite-FK enforcement** — a storage test asserting that inserting an
  `audience_members` row pairing a cross-author audience and subscription is
  rejected by the database (on both backends, with `foreign_keys` on for SQLite).
* **Resolution matrix** — unit tests over the resolution rule covering anonymous,
  author, subscriber, named-member, and non-member viewers against Public,
  Private, Subscribers, single-named, multi-named, and Public+named posts.
* **Parity** — `SubscriptionStore` / `AudienceStore` / extended `PostStore`
  exercised against both SQLite and Postgres.

## Out of scope (Layer B / C — ADR-0020)

ActivityPub `to`/`cc` emission and Authorized Fetch; email channel; follow
approval / locked accounts (M13); block/mute (M15); authenticated browsing for
non-local visitors; push backfill of prior posts to new audience members.
