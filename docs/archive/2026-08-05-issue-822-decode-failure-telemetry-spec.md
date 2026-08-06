# Spec — #822: arg-decode failures emit telemetry; ADR-0065's secret exception corrected

- Issue: [#822](https://github.com/jaunder-org/jaunder/issues/822)
- Milestone: Code quality ratchet
- Governing ADRs: [ADR-0011](../../adr/0011-unified-observability.md)
  (observability conventions, PII discipline),
  [ADR-0065](../../adr/0065-client-side-domain-validation.md) (amended in place
  by this issue)
- Date: 2026-08-05

## Problem

Two things, one root: typed wire args moved validation from the fn body to
arg-decode, and the ADR that mandated that move still describes the world before
it.

### 1. A failed arg decode emits no telemetry at all

`macros/src/server_fn.rs:159-172` emits `#[::leptos::server]` outermost and
`#[tracing::instrument]` inside it, so both the `web.<vertical>.<ident>` span
and `crate::error::server_boundary` live inside the generated `__server_<ident>`
fn. Arg deserialization happens earlier, in leptos's `from_req`. For a request
that fails decode:

- the instrument span is **never entered**;
- `emit_boundary_failure()` never runs — no `"server function failed"` event, no
  `metrics::error(kind, class)` counter;
- the client gets a generic `WebError::ServerFunction` body at HTTP 500.

Zero repo telemetry. A scanner probing malformed args leaves no trace.

**It is a measured regression.** #315 (`3e7087a0`, `b5c0b050`) flipped the auth
args from `String` to typed newtypes and deleted two named phase spans in the
process — `web.auth.login.parse_password` and
`web.auth.register.parse_password`. `rg parse_password` now returns nothing. No
password-_specific_ metric was lost, because none ever existed: `metrics::login`
/ `registration` / `password_reset` all sit downstream of the parse in both the
old and new bodies.

The gap is **not secret-specific** — it applies to every typed wire arg
(`Username`, `Slug`, `SessionLabel`, `PostSummary`, …).

### 2. ADR-0065's secret exception is factually stale

`docs/adr/0065-client-side-domain-validation.md:76-78` says a secret "cannot be
a typed wire arg; its arg stays `String`", and `:112-113` says secrets "still
parse in the body, so their rejection telemetry is unchanged". Both are false:
`ProfferedPassword` is `#[str_newtype(secret, serde)]` and validates on the
wire; all three password-taking server fns take it typed. A test already asserts
the opposite of the ADR (`server/tests/web/web_password_reset.rs:305`).

## Decisions

### D1 — Hook: `WebError::from_server_fn_error`, server-gated

`web/src/error.rs:67-73` is the only place that sees the **typed**
`ServerFnErrorErr` at the moment of decode. It is the real server-side funnel:
`PostUrl::from_req` (`server_fn/src/codec/url.rs:98-107`) constructs the error
via `into_app_error()`, which is `WebError::from_server_fn_error`, and that
value becomes the response body.

A middleware alternative (either an axum layer on `/api/{*fn_name}` or a
`server_fn` per-fn `Layer`) was rejected: both see only the `Response`,
degrading the discriminator to "HTTP 500 with a body tagged `server_function`"
and requiring the body to be buffered. A repo-local `FromReq` codec newtype
would isolate decode perfectly but would change every `input = …` site and force
the default codec everywhere — disproportionate.

**The `FromServerFnError` impl is compiled for the wasm client too** — `web`
compiles to wasm, and while `error.rs` already carries
`#[cfg(feature = "server")]` in several places (`:11-12`, `:82`, `:114`), the
`from_server_fn_error` impl itself is not gated. So the emit must be
`#[cfg(feature = "server")]`-gated: that feature is what pulls `dep:host`, and
`host::metrics` is native-only. Without the gate the wasm build fails to compile
— which `cargo xtask check`'s wasm clippy step catches (AC4).

### D2 — Fire on decode variants only

Emit for `ServerFnErrorErr::{Args, MissingArg, Deserialization}`; stay silent
for the rest.

- `Args` — every fn on the default `PostUrl` codec.
- `Deserialization` — the fns with `input = Json`.
- `MissingArg` — no producer in server_fn 0.8.12; matched anyway so a future one
  is covered.

**Why silence for the rest — not because they're unreachable.** `Request`,
`UnsupportedRequestMethod`, and `Serialization` are client-side, but
`ServerError`, `MiddlewareError`, and `Response` _are_ constructed on the server
(`response/http.rs:22,33,49`; `middleware/mod.rs:80,161,184`). They are excluded
because they are **not arg-decode failures** — they arise downstream of decode
and are already covered by the in-body boundary or are transport-level. The
predicate is "this request's arguments were malformed", not "this variant can
only happen on the client".

**`input = MultipartFormData` is not an arg-decode path.** `MultipartFormData`'s
`from_req` (`server_fn/src/codec/multipart.rs:84-90`) hands the body to the fn
as a `multer::Multipart` stream rather than deserializing typed args, so the
media-upload fn (`web/src/media/api.rs:191`) has no decode step to instrument.
Out of scope by construction, not by omission.

**Known impurity, accepted:** a truncated/non-UTF-8 request body also yields
`Deserialization` (`server_fn/src/request/axum.rs:48-61`), so a genuine
transport failure will emit this event. That is tolerable — a truncated body
_is_ a client-side failure, and `class = client` is honest for it. Matching
`Args` alone would be pure but would blind the `input = Json` fns entirely,
which is the worse trade.

### D3 — Reuse the existing error vocabulary; record the message

Construct the `InternalError` with **`validation_source`**
(`host/src/error.rs:241`):

```rust
pub fn validation_source(
    public_message: impl Into<String>,
    source: impl Error + Send + Sync + 'static,
) -> Self
```

It yields `ErrorKind::Validation` + `ErrorClass::Client` **and carries the
source** — which is the point. The sibling `InternalError::validation(msg)`
(`:141-149`) sets `source: None` and would emit `error.source = ""`, defeating
the whole purpose; `validation_source` is the one to use. `ServerFnErrorErr` is
a `thiserror` type, so it satisfies the `Error + Send + Sync + 'static` bound
and can be passed directly.

Then `.with_context("stage", "decode")` (`:256`) and `.emit_boundary_failure()`
(`:313`, `pub`). No new metric, no new enum variant, no new event name, no new
constructor.

The `public_message` is operator-facing only here — it lands in the event's
`error.public` field. Use a fixed, input-free string:
**`"invalid request arguments"`**.

**This emits telemetry only; it does not change the response.**
`from_server_fn_error` still returns
`WebError::server_function(value.to_string())`, so the wire shape a client sees
is untouched — which is what AC6 pins.

This satisfies ADR-0011's cardinality rule (`:96-100`) by construction — the
bounded enums are unchanged, and `stage=decode` rides in `error.context`, which
is a span field, not a metric attribute.

**`error.source` carries the deserializer's message, and that is PII-free by
construction** — verified, not assumed. The message is the newtype's own error
`Display` (serde calls `Error::custom` on the `FromStr` error), and essentially
every wire-arg newtype's error is a unit struct whose message interpolates only
constants. The three wire-arg types that wrap third-party text were each
checked:

| Type             | Wrapped error          | Verdict                                                                                 |
| ---------------- | ---------------------- | --------------------------------------------------------------------------------------- |
| `Email`          | `email_address::Error` | **Safe** — 17 unit variants; `Display` interpolates only consts                         |
| `Filename`       | own enum               | **Safe** — `TooLong` interpolates a byte count                                          |
| `BackupSchedule` | `croner::CronError`    | **Echoes input** — `InvalidPattern`/`IllegalCharacters`/`ComponentError` carry `String` |

`BackupSchedule` is the one exception: a malformed cron expression can put
fragments of the admin's own schedule string into `error.source`. That is **not
PII** under ADR-0011's list (email addresses, tokens, passwords, post bodies) —
a cron schedule is operator configuration, on an operator-only endpoint.
Recorded here as a known characteristic; making "wire-arg error messages never
echo input" an enforced invariant is out of scope (see below).

### D4 — The failing fn is identified by URI, not span name

ADR-0011 `:369-391` argues the failing fn's identity comes free from span
context because the boundary event is raised inside `web.<vertical>.<ident>`.
**That argument does not hold at decode time** — the instrument span lives
inside `__server_<ident>` and has not been entered.

The decode event will carry the outer request span from
`server/src/observability.rs:491-504`, which records `uri = %request.uri()` —
i.e. `/api/<vertical>/<ident>`. So the endpoint is recoverable, by URI rather
than span name. Accepted and documented rather than fixed: re-deriving the fn
identity at decode time would require one of the rejected D1 alternatives.

### D5 — ADR-0065 is amended **in place**

In-place amendment is the house convention for a local correction, and ADR-0065
has been edited in place twice without spawning a new ADR (#408 added the
Optional-fields bullet; #568 rewrote the Rendering and Coverage-boundary
bullets). Only **#568** used the full markup, so treat it as the single template
to follow: a header `- Note: amended <date> (#NNN) — <summary>` plus an inline
`_Amended by #NNN._` on each changed bullet. (#408 left no marker at all — not a
precedent worth copying.)

The `adr-format` gate constrains only the `# ADR-NNNN:` heading and the bare
`- Status:` token, so additional header lines are unconstrained.

Two passages change:

1. **The secret exception (`:76-78`).** The premise "a secret newtype has no
   serde bridge" is true of the _domain_ type (`Password`) and false of the
   _inbound twin_ (`ProfferedPassword`). What is actually distinctive about
   secrets is the **twin split** (ADR-0063's `Proffered`, generalized by
   ADR-0084): the domain type stays serde-free, and the twin carries the wire
   role. Secrets are otherwise ordinary typed wire args.
2. **The Consequences sentence (`:112-113`).** "Args that stay `String` (secrets
   like `password`) still parse in the body, so their rejection telemetry is
   unchanged" becomes a statement that secrets are decoded like every other
   typed arg — and, with D1-D3, that a decode failure is now observable for all
   args.

No new ADR. Nothing here is a novel decision: D1-D4 apply ADR-0011's existing
conventions, and D5 corrects prose to match code.

### D6 — Enforcing the "no echoed input" invariant is out of scope

D3 verifies the property holds today for every wire-arg type but one. Making it
a guarantee — an xtask gate asserting a wire-arg newtype's error interpolates
only constants — is a new static-analysis gate with its own design, and this
issue has already grown once from a docs fix. The plan's first task files it.

## Acceptance criteria

- **AC1** A failed arg decode emits the boundary event
  (`"server function failed"` with `error.kind`, `error.class`, `error.public`,
  `error.source`, `error.context`) and increments `metrics::error`. Pinned by a
  unit test in the style of
  `boundary_failure_event_carries_the_enclosing_instrument_span`
  (`web/src/error.rs:496-553`).
- **AC2** The event carries `error.kind = validation`, `error.class = client`,
  and a context entry `stage = decode`, distinguishing it from an in-body
  validation failure.
- **AC3** Only `Args`, `MissingArg`, and `Deserialization` emit. A test asserts
  a non-decode variant (e.g. `ServerFnErrorErr::Request`) emits **nothing**.
- **AC4** The emit is `#[cfg(feature = "server")]`-gated and the wasm build of
  `web` still compiles. Enforced by `cargo xtask check`'s **wasm clippy** step
  (`-p web --features csr --target wasm32-unknown-unknown`,
  `xtask/src/steps/static_checks.rs:59-90`). Note `check` does **not** run
  `build-csr` (that is e2e-only), but wasm clippy is enough to catch a cfg
  mistake here.
- **AC5** No new `ErrorKind`/`ErrorClass` variant, no new metric name, no new
  event name — the existing bounded vocabulary is reused (ADR-0011 `:96-100`).
- **AC6** An integration test pins the wire shape of a decode failure: HTTP 500
  and a body tagged `server_function`, distinguishing it from a
  `server_boundary` failure (which is tagged `validation`/`unauthorized`/…). The
  helpers already return the body, so no helper change is needed. Natural home:
  the too-short-password case in `server/tests/web/web_password_reset.rs` — the
  comment naming the decode path is at `:305`, and the assertion to strengthen
  is the `assert_ne!(status, StatusCode::OK)` at `:333`.
- **AC7** ADR-0065's secret exception describes the twin split, not "stays
  `String`", and its Consequences sentence describes decode-stage rejection —
  both amended **in place** with the established markup (header
  `- Note: amended …` + inline `_Amended by #822._`).
- **AC8** `rg 'stays?\s+`String`' docs/adr/0065-*.md` returns nothing — the
  pattern must match both offending phrasings: "its arg **stays** `String`"
  (`:77`) and "Args that **stay** `String`" (`:112`).
- **AC9** `cargo xtask validate` green.

## Out of scope

- An xtask gate enforcing that wire-arg newtype errors never echo input (D6 —
  filed).
- Changing `BackupSchedule`/`Email`/`Filename` error messages (D3 — verified
  safe or accepted).
- Restoring the deleted `parse_password` phase spans: they would wrap a
  `Password::try_from` that can no longer fail on the reachable path, since
  decode already rejected it — a span over an unreachable branch.
- Re-deriving the server-fn identity at decode time (D4).
- Any change to `metrics::login`/`registration`/`password_reset`, which never
  covered this path in any version.
