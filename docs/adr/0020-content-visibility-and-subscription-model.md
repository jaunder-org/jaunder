# ADR-0020: Content Visibility and Subscription Model

* Status: accepted
* Deciders: mdorman, Claude
* Date: 2026-06-18

## Context and Problem Statement

Until now every published Jaunder post has been fully public: a post is either a
draft (`published_at IS NULL`) or visible to anyone, with no axis in between. We
want authors to control *who* sees each piece of content — from "anyone on the
web" down to "only me" (a private journal), with arbitrary named groups in
between ("Friends", "Family", "Coworkers").

This control must hold across surfaces that do not all exist yet: today's web
timelines and post pages, the published feeds (M8), outbound ActivityPub
delivery (M12), and eventually an email newsletter channel. It must also not
assume that every recipient is an ActivityPub follower — a long-range goal is to
treat "subscriber" as protocol-agnostic, so an email subscriber or a local
community member is as much a subscriber as a remote AP follower.

We therefore need a single conceptual model for *visibility* that every present
and future surface can consume, rather than a per-surface bolt-on.

## Decision Drivers

* **One rule everywhere.** A single access-resolution rule the web UI, feeds, and
  federation all share, so a post's audience means the same thing on every surface.
* **Protocol-agnostic subscription.** "Follower" must generalize beyond
  ActivityPub so email, local accounts, and future protocols are just channels.
* **Dynamic membership.** Audience membership resolves at read/delivery time; a
  post references its audience, never a frozen member list.
* **Schema integrity over application invariants.** Constraints belong in the
  database where practical (see ADR-0019's dual-backend discipline), not in
  application code that can drift.
* **Incremental delivery.** The model must admit a small, fully demonstrable
  first slice while the federation and authenticated-browsing pieces remain future work.

## Decision Outcome

Adopt a unified **Channel / Subscription / Audience** model with a single
access-resolution rule, delivered in three layers.

### Core concepts

1. **Channel** — a transport/identity-space a subscription lives in. The first
   and only channel built initially is `local` (a Jaunder account on this
   instance). Later channels: `activitypub`, `email`, `at`. A channel knows how
   to recognize an incoming authenticated viewer as one of its subscribers and
   (from Layer B on) how to *deliver* content. A channel has up to two optional
   capabilities: **authenticate** (prove a viewer's `subscriber_ref` in the
   browser — Layer C) and **deliver** (push content to the subscriber — Layer B).
   `local` has neither (it is in-app: normal login, in-app reading); `email` and
   `activitypub` each gain authenticate in Layer C and deliver in Layer B, sharing
   one subscription and `subscriber_ref` across both.

2. **Subscription** — "identity X subscribes to author A via channel C." This is
   the generalization of "follower." All audience math is done over
   subscriptions, never over raw protocol identities.

3. **Audience** — what a post is addressed to:
   * **Built-in**, synthesized rather than stored as user data: **Public**
     (everyone, including logged-out visitors), **Subscribers** (all of the
     author's active subscriptions, across every channel), and **Private**
     (no one but the author).
   * **Named** — user-defined labels ("Friends"), whose members are a subset of
     that author's subscriptions.

4. **Post targeting** — a post references one or more audiences. The audiences
   compose by **union**; `Public` admits everyone, and `Private` is the *empty*
   audience (a post addressed to no one). Targeting is stored as references and
   resolved dynamically.

### The single resolution rule

The viewer is a **channel identity** — a `(channel, subscriber_ref)` pair, where a
local account is `(local, user_id)` — or anonymous; never a bare local user id.
Modelling the viewer this way is what lets non-local channels (a remote
ActivityPub actor, an email subscriber) flow through the *same* rule as a local
account. See the Layer C design for how each channel proves its identity.

Given a viewer (a channel identity, or anonymous) and a post:

* The author always sees their own post.
* Otherwise the viewer sees the post **iff any** of the post's targeted
  audiences admits them — Public admits everyone (including anonymous),
  Subscribers admits the author's active subscribers, a named audience admits
  its members. A post targeting *no* audience admits no one (Private).

Because access is "OR over the targeted audiences," union semantics, dynamic
membership, and the Public/Private extremes all fall out of one rule with no
special cases. The rule **fails closed**: losing targeting data makes a post
more private, never less.

Subscription creation passes through a single named **admission seam** that
decides a new subscription's initial status. Today it always returns `active` (a
no-op auto-approve), so the local channel and every later channel subscribe
openly; M13 swaps in the per-author "open vs. invite-only (default)" policy that
returns `pending` for invite-only authors, plus the approval transitions — in this
one place. Because the resolution rule above admits only `active` subscriptions,
approval gating is latent and fails closed before M13 is built.

### The three layers

* **Layer A — local channel + enforcement on the website.** Adds the data model,
  the `local` channel, local subscription, named-audience management, the
  authoring picker, and read-time enforcement on the web UI and feeds. This is
  the implementable first slice (see the Layer A design spec) and exercises the
  whole Channel → Subscription → Audience → resolution chain end-to-end. It also
  directly serves the "single instance as a small specialized community" goal.

* **Layer B — federation and additional channels.** Maps each audience to AP
  addressing (`to`/`cc` naming the `Public` collection, the `followers`
  collection, or explicit actors) and Authorized Fetch / HTTP Signatures on
  pull, and adds AP and email *delivery* transports. Lands with M12 and an email
  channel. **Caveat:** push is point-in-time — dynamic membership is fully
  enforceable on pull/browse but only forward-looking on push, because an
  already-delivered `Create` cannot be recalled from a remote inbox. Newly added
  members see prior posts by browsing, not by backfill.

* **Layer C — authenticated browsing for non-local visitors.** Lets a visitor
  prove a channel identity in the browser (IndieAuth / "Sign in with Mastodon" /
  email magic-link) so the same resolution rule grants them their permitted
  content. There is no single turnkey standard for this; it is an aspiration
  sequenced after Layer B, and overlaps with Jaunder-as-OAuth-server (M21).

## Consequences

* Good: one access rule shared by every surface; new channels and surfaces plug
  in without changing the model.
* Good: Public and Private are not special types but the extremes of one
  spectrum (full union vs. empty), and the rule fails closed.
* Good: subscription is protocol-agnostic from day one, validated by the `local`
  channel before any federation exists.
* Bad / accepted: push-based delivery (Layer B) cannot honor membership changes
  retroactively; this asymmetry is inherent to federation and is documented, not
  solved.
* Bad / accepted: authenticated browsing (Layer C) depends on standards that are
  not universally adopted, so it remains a long-range goal rather than a committed
  milestone.

## Related

* ADR-0005 (Unified Content Model), ADR-0007 (Auth Mechanisms),
  ADR-0009 (Edit/Delete Policy), ADR-0014/0015 (AtomPub),
  ADR-0019 (Generic Storage Backend via Dialect).
* Roadmap: M8 (Published Feeds), M12 (ActivityPub — publish),
  M13 (Account management — follow approval), M21 (OAuth 2.0).
