# ADR-0016: Dependency Injection, the `AppState` Bundle, and the Composition Root

* Status: accepted
* Deciders: mdorman, Claude Opus
* Date: 2026-06-13

## Context and Problem Statement

`storage::AppState` (`storage/src/app_state.rs`) is a single struct holding thirteen
`Arc<dyn *Storage>` handles **plus** `websub: Arc<dyn WebSubClient>` — an outbound-HTTP
publisher with no storage role. It is built once by `open_database`/`open_existing_database`
(via `make_app_state` in `storage/src/{sqlite,postgres}/mod.rs`) and threaded into every
consumer.

The web layer is already well-segregated: server functions retrieve only the per-trait
context they need (`expect_context::<Arc<dyn UserStorage>>()`), not the whole bundle. The
remaining whole-`AppState` consumers are the feed worker, the media manager, and the backup
worker. The structural problem (analysis doc §1.9) is **drift**: `AppState` is *both* (a) a
type capable of holding any dependency *and* (b) routinely passed beyond the composition root.
Any type with both properties accretes junk — the `websub` field is the proof (it was added as
the lowest-friction way to reach the publisher from the feed worker, *despite* an in-source
comment that the mailer "deliberately lives outside this bundle").

This ADR resolves the dependency-injection shape so the dependent refactors — §1.1 (storage
dedup), §1.9 (dissolve the omnibus), §1.10 (relocate `websub` out of `common`) — proceed
against a settled direction rather than re-litigating it per bead. It is a decision spike;
it changes no production code.

### Verified facts grounding the decision (P0a/P0b spikes, 2026-06-13)

* **Non-serve entry points over-construct storage (decides the `Backend` factory's value).**
  Every non-serve command that touches the DB calls `open_database`/`open_existing_database`,
  which builds the full thirteen-handle bundle **and** calls `default_client_from_env()`
  (constructing a real `reqwest` `HttpWebSubClient`), regardless of need:
  `cmd_init` builds the bundle and *discards* it; `cmd_user_create` uses only `.users`;
  `cmd_user_invite` only `.invites`; `cmd_smtp_test` only `.site_config`. Only `cmd_serve`
  legitimately needs the whole bundle. (`cmd_backup`/`cmd_restore` already bypass `AppState`,
  operating on the pool directly; `cmd_create_pg_db` uses a bootstrap connection with no
  `AppState`.) The over-construction of *handles* is cheap (`Arc` clones), but the
  over-construction of the *websub HTTP client service* for CLI commands is the §1.9
  "gate services" smell made concrete.
* **No `*Storage` impl holds per-instance state (makes mint-on-demand safe).** All 22 concrete
  storage structs (11 traits × {sqlite, postgres}) hold exactly one field — `pool` — and
  nothing else (no cache, memoization, atomic, counter). Minting a fresh handle per consumer
  only clones an `Arc<Pool>` and cannot split state. A `Backend` factory that mints on demand
  is therefore sound.
* **`websub` is server-only and storage is never built for wasm.** The `storage` crate has
  zero `target_arch` cfgs; `web`'s dependency on `storage` is `ssr`-gated. The native-only
  `default_client_from_env()` is currently called *from inside `storage`* (the `AppState`
  builders), not just from `server`.

## Decision Drivers

* **Stop the drift structurally, not with prose.** The fix must change the gradient so the
  lowest-friction way to add a dependency is the correct one — a constructor parameter — not
  another "don't" comment.
* **Concentrate breadth at the composition root,** which is allowed to know everything; demote
  the God Object from "a type passed around" to "a few lines of wiring at the edge."
* **Don't trade one nuisance for a smaller one with the same disease** (no "services bundle",
  no service-locator).
* **Earn the abstraction.** A `Backend` factory must be justified by measured over-construction,
  not added speculatively.

## Decision Outcome

Adopt **constructor injection, with a storage `Backend` factory used only at the composition
root**, sequenced in two phases:

### Phase A — Constructor injection (the smallest proving step; do first)

Each subsystem declares its needs as constructor parameters; the signature *is* the dependency
declaration. Services (mailer, websub) are server-constructed and injected per-consumer — there
is no services bundle. Concretely:

* Convert the feed worker, media manager, and backup worker to take exactly the handles/services
  their constructors ask for (e.g. `FeedWorker::new(posts, feed_cache, feed_events, websub)`),
  rather than the whole `AppState`.
* Extract `websub` and the workers as server-constructed injected *services*. `AppState` stops
  holding `websub`; the field is removed and `make_app_state` no longer calls
  `default_client_from_env()`.
* The web layer is unchanged — it is already per-trait-segregated via Leptos contexts.

Phase A alone resolves the motivating instance (websub leaves the bundle), unblocks §1.10, and
demonstrates the pattern without committing to the factory.

### Phase B — Storage `Backend` factory (justified by the measured over-construction)

Introduce a `Backend` abstraction that owns the pool and mints `Arc<dyn *Storage>` handles on
demand, **used only at the composition root** and **never injected into a subsystem** (injecting
it would be a service locator and would re-broaden the coupling Phase A just fixed). It is typed
to produce *only* storage, so a non-pool-derived thing like `websub` has no natural slot.

Phase B replaces the monolithic `make_app_state` so each entry point mints only the handles it
needs: `cmd_user_create` mints `users`, `cmd_user_invite` mints `invites`, `cmd_smtp_test` mints
`site_config`, `cmd_init` mints nothing beyond what migration verification requires, and
`cmd_serve` mints the full set. This is sound because handles are pure pool-wrappers (verified).

Sequencing Phase B after Phase A means the factory is designed against an `AppState` already
shrunk of services, and gives §1.1's dedup a settled handle-construction story.

### (b) Final home of the `WebSubClient` trait

Once `AppState` no longer holds `websub` (Phase A), **nothing in `common` or `storage`
references the trait** — the sole consumer is the feed worker. Therefore the entire
`common/src/websub` module relocates to **`server`**: the `WebSubClient` trait, `WebSubError`,
and `NoopWebSubClient` (wasm-safe, now needed only by the feed worker and server test helpers),
alongside the native-only `HttpWebSubClient`, `FileCapturingWebSubClient`, and
`default_client_from_env`. `common` loses the module entirely, eliminating its only
`target_arch` cfgs (§1.10). This supersedes the analysis doc's tentative "trait → `storage`"
option: under Phase A, storage has no reason to know about websub, so server is the correct home.
**Because the native client is currently constructed inside `storage`, §1.10's move cannot land
without Phase A first** — sequence kq8w.10 after the feed-worker injection in kq8w.76lg.

### Decision on scope: minimal-first, factory-second (not a single big rewrite)

We do **both** constructor injection and the `Backend` factory, but in that order, rather than
either a minimal-only pass or a speculative full factory. The "measure first" question from
§1.9 is answered: non-serve entry points *do* over-construct, so the factory earns its keep;
but Phase A is independently valuable and de-risking, so it leads.

## The Durable Invariant (recorded in `CONTRIBUTING.md`)

> **No type may be both (a) a heterogeneous dependency holder and (b) passed beyond the
> composition root.** Dependencies are declared as constructor parameters on the component that
> uses them. A storage `Backend` factory may mint handles, but only the composition root may
> hold it — it is never injected into a subsystem (that would re-introduce the service-locator
> coupling). Services (mailer, websub client, background workers) are constructed at the root
> and injected per-consumer; there is no "services bundle."

This is the structural form of the intent the mailer's in-source comment only gestured at: with
no omnibus threaded everywhere, there is no bag to dump into, and the invariant enforces itself.

## Consequences

* Good: the websub drift is fixed structurally; the next contributor who needs a new dependency
  in a subsystem must add a constructor parameter, which a reviewer sees immediately.
* Good: non-serve commands stop building an unused `HttpWebSubClient`; each mints only what it
  uses.
* Good: `common` loses its only `target_arch` cfgs once websub moves to `server`.
* Neutral: breadth doesn't vanish, it concentrates at the composition root — which is its job.
* Bad: more wiring lines at the root, and a `Backend` abstraction to maintain; accepted because
  the over-construction is measured and the mint-on-demand safety is verified.
* Note: this ADR does not change the §1.1 decision to stay on runtime `sqlx::query`/`query_as`
  (not the `query!` macros); see ADR-0001 / the analysis doc §1.1.

## Addendum (2026-06-27): SSR context lifetime across `await`s (#89)

The decision above keeps the web layer's per-trait DI on Leptos context
(`expect_context::<Arc<dyn FooStorage>>()`). A hazard not addressed at the time: this is only
reliable **while the reactive owner that carries the context is alive.**

In `reactive_graph`, the thread-local "current owner" is a *weak* reference, and a context's
storage is freed when the owner's last *strong* reference is dropped. During SSR, a server
function invoked as a `Resource`/`Suspense` future can have its owner's strong refs dropped while
it is suspended at an `.await` (more likely under load, when a DB call is slow). A context read
**after** that await then resolves to `None`, and `expect_context` panics (#89) — a rare,
load-correlated race that surfaced as flaky hydration timeouts. Server functions had only worked
around this by an unwritten convention of reading context **before** any await; three fns that
violated it were the bugs.

**Resolution.** Every server-fn body routes through `server_boundary` (the `boundary!` macro).
`server_boundary` now runs the body inside a `reactive_graph` `ScopedFuture` (`new_untracked`)
**when an owner is current** — which holds a *strong* owner reference (keeping the context alive)
and re-applies it on each poll. This makes `expect_context` in a server function reliable
**regardless of await ordering**, so the per-trait Leptos-context DI is sound as written, and the
"read context first" convention is retired (its explanatory comments are corrected, not its code).
The wrap is guarded on `Owner::current().is_some()`: `ScopedFuture::new_untracked` captures
`Owner::current().unwrap_or_default()`, so wrapping with no current owner would capture an *empty*
owner and lose context deterministically — strictly worse than the race. The guarantee is proven
by the deterministic `owner_lifetime` tests in `web/src/error.rs` (the mechanism, the fix, and the
empty-owner trap), not by the flaky e2e.

## Addendum (2026-06-28): owner capture at the `Resource` layer (#124)

The #89 addendum made `expect_context` reliable in a server-fn body **when an owner is current
at the body's first poll** — true for the HTTP `/api` path (`handle_server_fns_with_context`
establishes the owner). It is **not** true for a server fn invoked from an **SSR `Resource`
fetcher**: by the time that future is first polled (on a tokio worker), the request/component
owner's last strong ref is already dropped, so `server_boundary`'s guard takes the unprotected
branch and *every* context read (storage trait objects **and** request `Parts`) resolves to
`None` — `require_auth`'s `use_context::<Parts>()` fails before any await, so read-ordering
cannot help. This is a known Leptos SSR + multi-thread + reactive-ownership rough edge (leptos
issues #2562, #2341, #3729), surfaced as a hard failure by the #93 e2e zero-panic gate
(ADR-0032).

The owner is alive only at **fetcher invocation**, and an `async fn` body has no synchronous
prologue, so the capture cannot live in the handler — it must be at the `Resource` layer.

**Resolution.** A single constructor `server_resource(source, fetcher)` (`web`) wraps the
fetcher's future in `ScopedFuture::new_untracked` at invocation — capturing the live owner and
holding a strong ref across every poll, exactly #89's mechanism applied one layer out. It is the
**only** sanctioned way to create a `Resource` in `web`; a static guard (clippy
`disallowed-methods` if it binds, else a scanning test) fails the gate on any raw `Resource::new`
in `web/src`, so the wrapper is non-optional with **zero per-handler boilerplate** — server fns
keep `boundary!` + `expect_context` unchanged. Proven by deterministic `owner_lifetime`-style
tests (context survives an owner strong-ref drop before first poll via `server_resource`; the raw
constructor loses it), not by the flaky e2e. `Action::new` is assessed for the same exposure and
given a sibling wrapper if it shares it.

## Addendum (2026-06-28): ancestor-owner contexts across an SSR `await` (#138)

The #124 addendum captured the **resource's own (leaf) owner** and held it strong across every
poll. That keeps alive contexts stored *in that leaf owner* — but the storage trait objects are
provided once at the SSR root by `provide_app_state_contexts` (`leptos_routes_with_context`), an
**ancestor** of every component/resource owner. `expect_context` resolves a value by walking the
owner ancestry; `reactive_graph` links a child to its parent with a **weak** ref. So when the SSR
runtime drops the root/ancestor owner while a server fn is suspended at an `.await`, the ancestor's
`OwnerInner` (and its context map) is freed even though the leaf is held — and a **post-await**
`expect_context::<Arc<dyn …Storage>>()` panics. This is why the authenticated `Sidebar`/
`BackupBanner` server fns (`backup/mod.rs`) panicked under page-render SSR while the ~75 sibling
sites that read storage **before** their first await did not: a pre-await read copies the `Arc` out
while the ancestry is still alive.

**Resolution.** `server_boundary` now holds the **full owner ancestry** strong for the future's
lifetime: at entry it walks `Owner::current()` to the root via `Owner::parent()` (which upgrades the
weak parent link to a strong `Owner`) and keeps the resulting handles alive alongside the
`ScopedFuture`. With the whole ancestry pinned, every post-await reactive-context read resolves —
**independent of read ordering**, so server-fn bodies need no read-before-await discipline and keep
`boundary!` + `expect_context` unchanged (**zero per-handler boilerplate**). This *eliminates* the
post-await-read failure class structurally rather than policing it with a lint. Proven by
deterministic `owner_lifetime` tests (`server_boundary_keeps_ancestor_context_alive_across_await` is
red before the fix, green after; `post_await_read_loses_ancestor_context_when_parent_owner_dropped`
characterizes the underlying leaf-only loss) and by the #93 e2e zero-panic gate (ADR-0032) on
authenticated `/` and `/posts/new`.
