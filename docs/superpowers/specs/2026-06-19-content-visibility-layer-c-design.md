# Content Visibility — Layer C: Authenticated Browsing for Non-Local Visitors

Status: draft 2026-06-19
ADR: [ADR-0020](../../decisions/0020-content-visibility-and-subscription-model.md)
Builds on: [Layer A design](2026-06-18-content-visibility-design.md)
Beads: TBD (created during planning)

## Goal

Let a non-local visitor **prove a channel identity in the browser** so the single
ADR-0020 resolution rule grants them exactly the gated content they are permitted
to see. This is the "authenticated browsing" layer named in ADR-0020.

After this work a remote person can **Sign in with Mastodon** or via an **email
magic-link**, Subscribe to a local author, and read that author's Subscribers- or
named-audience content on the website — while remaining structurally read-only.

The design is **channel-extensible**: the authentication seam, the viewer-session
infrastructure, and the admission seam are generic; Mastodon and email are the two
concrete channels delivered here, and further channels are additive.

## Relationship to the other layers

* **Layer A** (in flight) builds the data model, the `local` channel, named
  audiences, per-post targeting, and read-time enforcement. Layer C consumes it
  and requires two small **reworks** to it (see "Reworks to existing documents").
* **Layer B** (federation + delivery, M12 + email newsletter) is *not* built here,
  but Layer C imposes forward-compatibility constraints recorded below.

Implementation is sequenced **after Layer A is built**; this spec is design plus
the consequent amendments to the Layer A / ADR-0020 documents.

## Scope

In scope:

* The `ChannelAuthenticator` seam and its server-side registry.
* The read-only **viewer-session** infrastructure and the `ViewerIdentity`
  extractor.
* Two concrete channels: **Sign in with Mastodon** (OAuth, identity-only) and
  **email magic-link**.
* The **admission seam** ("design for M13, NOOP now").
* Reworks to ADR-0020 and the Layer A spec; Layer B forward-compatibility notes.

Explicitly out of scope (noted, not built):

* **Author login via IndieAuth** — a login method for *existing local accounts*;
  belongs with M21/OAuth, a separate axis from the subscriber/viewer model.
* **Account-linking** — one human who is both a local user and a remote identity
  is, for now, simply two subscriptions in two channels.
* **Push backfill**, **remote-viewer interactions** (those arrive via AP / Layer B),
  follow **approval** proper (M13), block/mute (M15).

## Conceptual frame

### One viewer model, many channels

Every read is evaluated for a `ViewerIdentity`:

```
ViewerIdentity = Anonymous
               | Channel { channel_id, subscriber_ref }   // local viewer = Channel{local, user_id}
```

The `local` channel is **not special** — instance login is simply how the `local`
channel authenticates. Layer C adds *other* channels' authentication paths, and
the resolution rule is unchanged because it only ever asks "does an active
subscription exist for this `(channel, subscriber_ref)`?"

### Two authentication purposes, kept separate

* **Subscriber identity proof** (in scope) — a remote consumer proves a channel
  identity to *read* gated content. Produces a `Channel(channel, ref)` viewer.
  **Read-only**: structurally incapable of account/write actions.
* **Author account login** (out of scope) — a local user logging into their own
  account; IndieAuth-as-login lives with M21.

### Identity and subscription are decoupled

* **Sign in** = prove a channel identity → a **viewer session**. Instance-wide, no
  author involved.
* **Subscribe** = Layer A's existing per-author action, now also usable by a
  signed-in remote viewer. It runs the **admission seam**.

One identity → potentially many per-author subscriptions. A remote viewer signs in
once, then Subscribes to whichever authors they want, reusing Layer A's Subscribe
path verbatim.

## Reworks to existing documents

These are the entirety of the Layer A / ADR-0020 footprint. Both are cheap *now*
(the `local` channel is the only producer) and avoid a cross-cutting refactor
later.

### Rework 1 — `ViewerIdentity` replaces `Option<user_id>`

The Layer A spec currently states: *"The viewer (`Option<user_id>`) becomes a
parameter of the timeline and single-post queries."* Replace with a shared
`ViewerIdentity` type in `common`, consumed by the `PostStore` resolution paths
and constructed by the server extractor.

The resolution SQL generalizes its non-public joins from
`subscriber_ref = :user_id AND channel = 'local'` to
`channel_id = :viewer_channel AND subscriber_ref = :viewer_ref AND status = 'active'`.
The author-sees-own shortcut fires only when the viewer is
`Channel{local, author_user_id}` — a remote viewer is never the author. Anonymous
still reduces to `EXISTS(… kind = 'public')`.

**No table changes** — `subscriptions` already carries `channel_id` +
`subscriber_ref`; this is purely the trait signatures and the join predicate. In
Layer A only `Anonymous` and `Channel{local, …}` are ever constructed; the type is
deliberately wider than Layer A needs.

### Rework 2 — make admission an explicit, shared seam

Layer A auto-approves *implicitly* ("open subscription, no approval"). Promote that
to one named decision point that the local channel and every Layer C channel call:

```
SubscriptionPolicy::initial_status(author, channel_id, subscriber_ref) -> SubscriptionStatus
```

Layer A's implementation returns `Active` unconditionally (the NOOP). M13 later
swaps in the per-author "open vs. invite-only (default)" policy returning
`Pending`, plus approval transitions — in this one place. `subscription_statuses`
already reserves `pending`/`blocked`, and resolution already admits only `active`,
so this **fails closed** today with no further change.

### Consequent `SubscriptionStore` signature changes

`SubscriptionStore` becomes channel-parameterized rather than local-hardcoded:

* `subscribe(author, channel_id, subscriber_ref)` (calls `initial_status` internally)
* `is_subscriber(author, viewer: &ViewerIdentity)`

Layer A only ever calls these with `channel = local`, but the seam is what Layer C
channels reuse verbatim.

### ADR-0020 amendments

* State explicitly that the viewer is a **channel identity** (not a `user_id`) and
  that admission is a **named seam**, so ADR and spec agree.
* Add the **channel capability split**: a channel has up to two optional
  capabilities — **authenticate** (Layer C) and **deliver** (Layer B). `local` has
  neither (in-app); `email` gets authenticate now and deliver later; `activitypub`
  gets authenticate now (Sign in with Mastodon) and deliver later (M12).

## Layer C core infrastructure

### The `ChannelAuthenticator` seam

Keep the two halves of "channel" distinct:

* The **data channel** (storage: the `channels` lookup, `subscriptions`,
  recognizing a `subscriber_ref`) — Layer A.
* The **channel authenticator** (server-side: how a browser *proves* a
  `subscriber_ref` interactively) — new, the heart of Layer C.

A server-side registry keyed by channel, each implementing a two-phase interface:

```
initiate(identity_input, return_to) -> AuthChallenge      // redirect (OAuth) or "check your email"
complete(callback_params)           -> VerifiedIdentity { channel_id, subscriber_ref }
```

The `local` channel has **no** authenticator (it authenticates via normal login).
Mastodon and email each register one. A future channel is purely additive —
register an authenticator, no core changes.

### The `ViewerIdentity` extractor

Resolves every request with precedence:

1. Valid **account session** (ADR-0007) → `Channel{local, user_id}` (and the usual
   `AuthUser`).
2. Else valid **viewer session** → `Channel{that channel, ref}`.
3. Else `Anonymous`.

**Read-only guarantee:** the viewer-session path *never* yields an `AuthUser`, so
no write/account endpoint can be reached with a viewer cookie — it is structurally
read-only.

### Endpoints & cookie

* `GET/POST /signin/:channel` → `initiate` (Mastodon: redirect; email: send link).
* `GET /signin/:channel/callback` → `complete` → create viewer session, set cookie,
  redirect to `return_to`.
* `POST /signout` → clear viewer session.

Cookie: `HttpOnly; Secure; SameSite=Lax`, opaque token. Lifetime configurable
(`viewer_session.ttl_days`, default 30, sliding on `last_seen_at`).

## Channel: Sign in with Mastodon

An **identity-only** OAuth flow — prove who they are, then discard the token (no
ongoing access, minimal data retention).

1. **Input** — visitor enters their handle `@user@instance`; derive the instance
   base URL.
2. **App registration (per-instance wrinkle)** — on first contact with an
   instance, `POST /api/v1/apps` (client_name, our `redirect_uri`, least-privilege
   scope, website) → `client_id`/`client_secret`; cache in `oauth_client_apps` and
   reuse thereafter.
3. **Authorize redirect** — to `https://instance/oauth/authorize` with
   `response_type=code`, our `client_id`, `redirect_uri`, least-privilege scope
   (`profile` on modern Mastodon, else `read:accounts`), a random `state`, and
   **PKCE** `code_challenge` (S256). Persist `state`+`code_verifier`+`instance`+
   `return_to` in `viewer_auth_flows`. PKCE preferred; `state` is mandatory CSRF
   protection.
4. **Callback** — look up the flow by `state` (reject if missing/expired), exchange
   `code`+`code_verifier` at `https://instance/oauth/token` → access token.
5. **Identity** — `GET /api/v1/accounts/verify_credentials`, then canonicalize to
   the **AP actor URI** as `subscriber_ref`. This canonical form is deliberate:
   when M12/Layer B later sees a real ActivityPub Follow from the same actor, it
   lands on the *same* subscription row — one identity across browse and delivery.
6. **Discard the token**, run the generic completion (viewer session; per-author
   Subscribe via the admission seam).

**Scope note:** "Mastodon" means *Mastodon-API-compatible* (Pleroma, GoToSocial,
etc. — the `/api/v1/apps` + `/oauth` surface). Non-compatible AP servers are out of
scope for the browser flow; they can still arrive as subscribers via Layer B.

## Channel: Email magic-link

The simplest channel — stateless, reusing M3's email machinery.

1. **Input** — visitor enters an email address; normalize (trim + lowercase) so
   `subscriber_ref` is stable.
2. **Mint a signed token** — HMAC token carrying `{ purpose: viewer-signin, email,
   expiry }`, short TTL (~15 min). Reuses M3's email-verification token pattern; no
   `viewer_auth_flows` row needed (stateless).
3. **Send** — magic link `/signin/email/callback?token=…` via the existing
   `common::mailer`.
4. **Callback** — verify signature + expiry → `subscriber_ref` = the normalized
   email, `channel = email`. Run the generic completion.

**Hardening / notes:**

* **Single-use** — the short TTL bounds the replay window; optional single-use
  tracking can mirror whatever M3 does if we want it tighter.
* **Abuse control** — the sign-in endpoint is **rate-limited**
  (`viewer_signin.email_rate_limit`), and the response is identical whether or not
  the address is "known" (no enumeration).
* **Not auto-linked to local accounts** — a local user's account email is *not*
  automatically an `email`-channel identity; `email` (ref = address) and `local`
  (ref = user_id) are distinct channels.

## Layer B forward-compatibility constraints

Recorded in ADR-0020; not built here.

1. **Canonical `subscriber_ref` must converge** — Layer B's inbound AP `Follow`
   handling must canonicalize to the **same AP actor URI** Layer C uses, so a
   browse-time self-serve subscription and a real federated Follow land on one row.
2. **One rule, three identity producers** — Layer B adds **HTTP Signature /
   Authorized Fetch** verification as a third `ViewerIdentity` producer, yielding
   `Channel{activitypub, actor URI}` and feeding the *same* resolution rule used by
   browser viewer sessions. Full set: account session (local), viewer session
   (Layer C), signed fetch (Layer B).
3. **Push mapping** stays as ADR-0020 specifies (`to`/`cc` mapping with the
   point-in-time caveat).
4. **Channel capability split** — authenticate (Layer C) vs. deliver (Layer B);
   `email` and `activitypub` get authenticate now, deliver later, sharing one
   subscription + `subscriber_ref`.

## Data model & components

**New tables** (all dual-backend per ADR-0019):

| Table | Columns | Purpose |
|---|---|---|
| `viewer_sessions` | `id PK, token_hash, channel_id FK, subscriber_ref, created_at, expires_at, last_seen_at` | Remote viewer's browsing session. Token hashed at rest. |
| `viewer_auth_flows` | `state PK, channel_id FK, code_verifier, instance, return_to, expires_at` | OAuth transient state (Mastodon). Consumed once on callback. |
| `oauth_client_apps` | `instance PK, client_id, client_secret, created_at` | Cached per-instance Mastodon app creds. `client_secret` stored at rest (reusable). |

No table for email — stateless signed token reuses M3.

**No schema change** to Layer A tables (`subscriptions`, `subscription_statuses`,
`post_audiences`).

**Channel seed rows:** the `channels` lookup gains seeded `activitypub` + `email`
rows (their own seed migrations), with matching Rust enum variants.

**New storage traits** (dual-backend impls + parity tests):

* `ViewerSessionStore` — `create`, `resolve(token)`, `touch(last_seen)`, `delete`,
  `prune_expired`.
* `ViewerAuthFlowStore` — `create`, `take(state)` (consume), `prune_expired`.
* `OAuthClientAppStore` — `get(instance)`, `put(instance, creds)`.

**New shared type** (`common`): `ViewerIdentity` (+ the `SubscriptionPolicy`
admission seam).

**Server-side:**

* `ChannelAuthenticator` trait + registry (`MastodonAuthenticator`,
  `EmailAuthenticator`).
* `ViewerIdentity` extractor (precedence: account session → viewer session →
  anonymous; viewer path never yields `AuthUser`).
* Endpoints: `signin/:channel`, `signin/:channel/callback`, `signout`.

**New config:**

* `viewer_session.ttl_days` (default 30, sliding).
* `viewer_signin.email_rate_limit` (abuse control for magic-link sends).

## Web surfaces

* **Sign-in entry points** — on a gated page a logged-out/anonymous visitor sees a
  "Sign in to read" affordance offering the available channels (Mastodon handle
  input, email address input).
* **Profile Subscribe** — Layer A's per-author Subscribe action becomes available
  to a signed-in remote viewer (same action, same admission seam).
* **Viewer status / sign-out** — a minimal indicator of the signed-in remote
  identity and a sign-out control. No account area — remote viewers have no
  account.

## Testing

Per CONTRIBUTING's backend-parity + coverage discipline.

### Unit / resolution

* **Resolution matrix, extended** — authored against `ViewerIdentity`, adding
  remote rows: `activitypub` subscriber, `email` subscriber, non-subscriber
  remote, anonymous — against Public / Private / Subscribers / single-named /
  multi-named / Public+named posts.
* **Extractor precedence** — account session > viewer session > anonymous; and the
  **read-only guarantee**: a viewer cookie on a write/account endpoint is rejected
  (no `AuthUser` produced).
* **Admission seam** — `initial_status` returns `Active` (NOOP); resolution admits
  `active` and excludes a synthetic `pending` (fail-closed), proving M13-readiness.

### Channel authenticators (real network-level, mirroring M8's `HttpWebSubClient`)

* **Mastodon** against an in-process mock instance on a random port: app
  registration, authorize-URL construction (state + PKCE challenge), token exchange
  with `code_verifier`, `verify_credentials` → **actor-URI canonicalization**,
  `state` mismatch rejected, flow expiry, and **app-creds caching** (second sign-in
  reuses cached `client_id`, no re-register). One test locks the canonical
  `subscriber_ref` form to guard the Layer B convergence invariant.
* **Email** — token round-trip; tampered/expired rejected; normalization → stable
  `subscriber_ref`; rate-limit; non-enumeration; send captured via
  `JAUNDER_MAIL_CAPTURE_FILE`.

### Storage parity (SQLite + Postgres)

* The three new stores; viewer-session expiry/prune; auth-flow single-consume
  (`take` removes).

### Bijection

* The `channels` lookup gains seeded `activitypub` + `email` rows; the existing
  channels bijection test extends to require matching enum variants — fails CI if a
  channel is seeded without an enum variant or vice-versa.

### E2E (Playwright, Nix VM)

* Sign in with Mastodon against an in-VM mock instance → Subscribe → see a
  Subscribers-gated post → sign out → it disappears.
* Email magic-link via mail capture → Subscribe → see gated content.
* Read-only enforcement: viewer cookie cannot reach a write endpoint.
* Anonymous sees only Public (Layer A regression under the reworked extractor).

## Open items for the plan

* Confirm the SQLite pool sets `PRAGMA foreign_keys = ON` (already flagged by Layer
  A; the new FKs to `channels` depend on it).
* Decide the canonical AP actor-URI resolution path for Mastodon
  (`verify_credentials` fields vs. WebFinger) and lock it with the convergence
  test.
* Decide whether email single-use hardening is in this slice or deferred.
