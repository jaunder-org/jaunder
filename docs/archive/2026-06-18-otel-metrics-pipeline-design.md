# OTel metrics pipeline (jaunder-kq8w.21, §4.1)

* Status: approved (brainstorming, 2026-06-18)
* Bead: jaunder-kq8w.21
* Follow-ups: jaunder-kq8w.24 (saturation gauges), jaunder-kq8w.25 (CLI metric export)

## Context & goal

Jaunder emits OpenTelemetry **traces** today (OTLP, optional, enabled by an OTLP
endpoint env var; see `server/src/observability.rs` and ADR-0011). There are no
**metrics**. This change adds an OTel metrics pipeline plus a curated set of
event counters/histograms for operational visibility — detecting auth abuse,
silent email/WebSub failures, backup health, capacity pressure (uploads), and an
overall error rate.

Acceptance (from the bead): metrics pipeline wired; a set of key
counters/histograms emitted; documented; `scripts/verify` clean.

## Scope

**In scope:** the event-based instruments below (cheap `.add()`/`.record()` at
call sites), the OTLP meter pipeline, and a cardinality-safe emit facade.

**Out of scope (own beads):**
- Saturation **gauges** via async observable callbacks (feed-event queue depth,
  time-since-last-successful-backup, DB pool utilization, media-storage bytes) —
  **jaunder-kq8w.24**. They need a different instrument lifecycle (periodic
  callbacks that query live state).
- Making one-shot **CLI** emits actually export (init + `force_flush`) —
  **jaunder-kq8w.25**. CLI emit *sites* are added here but are no-ops without a
  `MeterProvider` (only the long-running server exports).
- Per-route **HTTP RED** metrics — derive from the spans already exported; native
  per-route metrics would be higher-cardinality and duplicative.
- Process CPU/memory — a host/runtime/collector concern.

## The metric catalog

Modeling: **dimensional instruments** — one instrument per concept with a
low-cardinality attribute, not many boolean counters. **Hard rule: every
attribute is a bounded enum** — never a username, id, URL, or free string.
Names are dotted `jaunder.<area>.<thing>`; histograms carry a unit.

### Auth & abuse
- `jaunder.auth.logins` (counter) — `outcome` ∈ {`success`, `invalid_credentials`, `internal_error`}
- `jaunder.auth.session_validations` (counter) — `outcome` ∈ {`ok`, `invalid_token`, `session_not_found`, `internal`}
- `jaunder.auth.registrations` (counter) — `source` ∈ {`web`, `cli`}, `policy` ∈ {`open`, `invite_only`, `cli_bypass`}, `result` ∈ {`ok`, `rejected`}
- `jaunder.auth.invites` (counter) — `event` ∈ {`created`, `redeemed`}
- `jaunder.auth.password_resets` (counter) — `event` ∈ {`requested`, `completed`}

### Errors
- `jaunder.errors` (counter) — `error.kind` + `error.class` (the §3.1bc bounded
  enums), emitted once at the `InternalError` boundary.

### Email
- `jaunder.email.sent` (counter) — `kind` ∈ {`verification`, `password_reset`}, `result` ∈ {`success`, `failure`}
- `jaunder.email.send_duration` (histogram, ms)

### Media
- `jaunder.media.uploads` (counter) — `outcome` ∈ {`stored`, `deduplicated`, `quota_exceeded`, `too_large`, `invalid`}
- `jaunder.media.upload_bytes` (histogram, bytes) — accepted upload sizes
- `jaunder.media.served` (counter) — `result` ∈ {`ok`, `not_found`, `not_modified`}

### Feeds & worker
- `jaunder.feed.regenerations` (counter) — `result` ∈ {`ok`, `error`}
- `jaunder.feed.regeneration_duration` (histogram, ms)
- `jaunder.feed.websub_pings` (counter) — `outcome` ∈ {`success`, `failed`, `exhausted`, `no_hub`}
- `jaunder.feed.cache` (counter) — `result` ∈ {`hit`, `miss`}

### Backups
- `jaunder.backup.runs` (counter) — `result` ∈ {`success`, `failure`}
- `jaunder.backup.duration` (histogram, ms)
- `jaunder.backup.bytes` (histogram, bytes)
- `jaunder.backup.pruned` (counter)

### Content
- `jaunder.posts` (counter) — `event` ∈ {`created`, `updated`, `published`, `deleted`}
- `jaunder.atompub.requests` (counter) — `op` ∈ {the ~10 handler names}, `result` ∈ {`ok`, `client_error`, `server_error`}

## Architecture

### Facade home: `common::metrics` (feature-gated)

Emit sites span `web` (login/session/registration/posts/error boundary/email),
`server` (media/feed/backup workers, AtomPub), **and** the `server` CLI
(`cmd_user_create`, `cmd_user_invite`). The dependency order is
`server → web → common`, so the only crate reachable by all emitters is
`common`.

Add the facade as `common::metrics` behind a new **`metrics` feature** that
pulls an **optional** `opentelemetry` dependency. `server` and `web`'s `ssr`
feature enable it; `hydrate`/wasm never does. This is a *feature* gate, not a
`target_arch` gate, so it keeps `common` free of the `target_arch` cfgs that
jaunder-kq8w.10 removed, and the wasm build never pulls `opentelemetry`.

The **meter pipeline** (exporter + `MeterProvider`) stays in
`server::observability` — the binary owns exporter setup. `common::metrics` only
reads `opentelemetry::global::meter("jaunder")`. When no `MeterProvider` is set
(OTLP endpoint unset, or any non-server process), the global meter is a no-op, so
instruments are free — exactly like traces today.

### Meter pipeline (`server/src/observability.rs`)

Mirror the existing trace path:
- `build_otel_meter(endpoint) -> Result<(), String>`: OTLP `MetricExporter`
  (tonic) + `PeriodicReader` + `SdkMeterProvider` (with the `jaunder` resource) +
  `opentelemetry::global::set_meter_provider(provider)`.
- Wire it into `init_tracing_impl` behind the **same** OTLP-endpoint gate (one
  endpoint feeds traces and metrics). Setup failure logs to stderr and continues,
  like the tracer. `init_tracing` keeps its name to minimize caller churn (it
  already owns propagator + tracing + OTel setup).
- First implementation step: confirm the OTLP **metrics** feature/API in the
  pinned `opentelemetry-otlp`/`opentelemetry_sdk` versions; adjust the builder
  calls to match.

### Emit facade shape (cardinality-safe by construction)

`common::metrics` lazily builds all instruments once from
`global::meter("jaunder")` (a `LazyLock`) and exposes typed helpers whose
arguments are **small enums, not strings** — e.g.
`metrics::login(LoginOutcome::InvalidCredentials)`, `metrics::upload_bytes(n)`,
`metrics::websub_ping(PingOutcome::Exhausted)`. Each enum maps to a `&'static
str` attribute value, so a call site cannot pass an unbounded label — the
cardinality rule is enforced by the type system.

### Emit sites

One helper call each (broad but shallow):
- `web/src/auth/server.rs` — logins, session validations
- `web/src/auth/server.rs` — logins, session validations
- `web/src/account` / `web/src/invites` server fns — registrations (web), invites
- `web/src/password_reset` / `web/src/email` server fns — password resets, and `jaunder.email.sent{kind, result}` + `send_duration` (emitted **here**, where the email `kind` is known — not inside the generic mailer, which doesn't know `kind`; this also keeps the emit to one layer)
- `web/src/error.rs` boundary — `jaunder.errors{error.kind, error.class}`
- `web/src/posts/server.rs` — posts events
- `server/src/commands.rs` — `cmd_user_create` (registrations, `source=cli`), `cmd_user_invite` (invites)
- `server/src/media_manager.rs`, `server/src/media.rs`, `server/src/atompub/*` — media uploads/served, atompub ops
- `server/src/feed/{worker,handlers}.rs` — regenerations, regeneration_duration, websub_pings, cache hit/miss
- `server/src/backup.rs` — runs, duration, bytes, pruned

## Testing

- **Facade unit tests**: install a `MeterProvider` backed by `opentelemetry_sdk`'s
  in-memory metric reader, call a helper, collect, and assert the
  counter/histogram value and attributes — real values, real coverage, no live
  exporter.
- **Pipeline branches**: mirror the existing exhaustive `observability.rs` tracer
  tests (`build_otel_meter` accepts a valid endpoint; init with metrics enabled;
  json/pretty × endpoint-present/absent), to hold the file's ~99% coverage.
- **Emit lines**: covered incidentally by the existing path tests (login, upload,
  backup, feed-worker, etc.) that already exercise those code paths.
- **Coverage re-baseline** at the end via `scripts/update-coverage-baseline`
  (new lines in `observability.rs`, `common/src/metrics.rs`, and the emit sites).
- `scripts/verify` clean.

## Documentation

Addendum to ADR-0011 (unified observability): the metric catalog, the
naming/bounded-attribute conventions, the `common/metrics` feature + env-driven
enablement, and the in-memory-reader test pattern.

## Risks / notes

- **Coverage breadth.** Touching ~10 files of emit sites changes coverage; the
  re-baseline (Nix) absorbs it. Keep emit lines on already-tested paths so they
  stay covered.
- **OTLP metrics API drift.** The exact `opentelemetry-otlp` metrics builder API
  varies by version; the first plan step pins it down before wiring the rest.
- **No double-count.** Where a web server fn and a deeper helper both touch the
  same event (e.g. email send), emit at exactly one layer.
