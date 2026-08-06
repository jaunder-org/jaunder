# ADR-0102: A post body has a non-blank invariant, and normalization is format-aware

- Status: accepted
- Date: 2026-08-05
- Issue: [#811](https://github.com/jaunder-org/jaunder/issues/811)

## Context

`PostBody` was introduced under `#[str_newtype(infallible)]` (#402), with the
rustdoc claim "no length bound — any body is valid". A body consisting entirely
of blank lines was therefore a constructible, storable value, and the system
compensated downstream instead of rejecting it at the door:

- `derive_post_title` carried an outer `Option` whose only job was the
  empty-post gate, which **both** call sites immediately re-raised as
  `PerformCreationError::EmptyPost` / `PerformUpdateError::EmptyPost`. A
  validation decision was being returned as `None` and converted to an error by
  every caller.
- `canonicalize_org_body(&str) -> String` was stringly-typed on both ends, so
  the canonicalization seam could not express failure at all. Markdown had no
  equivalent seam; Org was special-cased inline in `storage`.

Nonsense input was being routed around rather than made unrepresentable.

Two further forces shaped the decision:

**`PostBody` does not know its format.** The format lives beside the body as
`PostFormat`. Whether trailing whitespace is content depends on the format, so a
format-agnostic constructor cannot normalize — it can only validate.

**Whitespace in a body is not uniformly insignificant.** Measured against the
real renderers, a blanket `trim()` is destructive: `"    fn main() {}\n"` as
Markdown is a CommonMark indented code block, and trimming turns
`<pre><code>fn main() {}</code></pre>` into `<p>fn main() {}</p>`. A code block
silently becomes prose.

## Decision

### 1. `PostBody` carries a non-blank invariant, through one door

A `PostBody` contains at least one non-blank line. `FromStr` validates; the
`StrNewtype` derive routes serde **and** sqlx through it, so the wire door and
the decode door are the same door. There is deliberately **no
`from_trusted`-style bypass for stored rows**: no blank-body rows exist, so none
need accommodating. A blank body in the database would fail to decode, and that
is the intended reading of "unrepresentable".

Any body _length_ remains valid. This narrows the #402 claim without introducing
a length bound.

**This reasoning does not transfer to `PostTitle`** (#830, since merged).
Migration `0010_nullable_post_titles.sql` shows blank titles demonstrably did
accumulate, so the no-trusted-bypass decision here is specific to bodies — #830
reached the same invariant-first conclusion for titles without inheriting this
read policy.

### 2. Validation lives in `PostBody`; normalization lives in `canonicalize_body`

Two layers, because the constructor is format-agnostic and normalization is not:

- `PostBody::from_str` — rejects a body with no non-blank line, then stores
  **verbatim**. Never trims.
- `canonicalize_body(&PostBody, &PostFormat) -> Result<PostBody, InvalidPostBody>`
  — the format-aware seam, replacing Org's inline special-case in `storage`.

The stored body is the canonicalized one; re-decoding it only re-checks
non-blankness, so the layers compose without a second normalization pass.

### 3. The normalization

```
Html            => verbatim, exempt
Markdown | Org  => 1. drop leading all-whitespace lines
                   2. trim_end()
                   3. re-append one '\n' if non-empty
Org             => additionally strip the title source (ADR-0024)
empty result    => Err(InvalidPostBody)
```

Three constraints are load-bearing and were each established by measurement:

- **Never strip leading _horizontal_ whitespace from a line that has content** —
  that is the indented-code-block case above.
- **Never touch interior blank lines** — they control CommonMark loose-vs-tight
  lists (`"- a\n\n- b\n"` renders `<li><p>a</p></li>`; `"- a\n- b\n"` renders
  `<li>a</li>`).
- **Step 3 is not optional.** A bare `trim_end()` eats the body's _terminating_
  newline, which is significant inside `<pre><code>` and inside Org paragraphs.

**HTML is exempt** because `PostFormat::Html` is verbatim passthrough: any
whitespace edit is a byte change, and an unclosed `<pre>` fails exactly like the
fence case below.

### 4. One accepted lossy case

`trim_end()` is lossy when the body ends _inside an unclosed code region_, where
the trailing blank lines are content:

````
body = "```\ncode\n\n"      (unclosed fence)
  raw        => <pre><code>code\n\n</code></pre>
  normalized => <pre><code>code\n</code></pre>
````

Detecting this requires a format parser, not a whitespace rule. We accept it:
the input is malformed, and the loss is trailing blank lines inside it. This is
pinned by a deliberately-named test so it reads as a decision, not a defect.

### 5. A title-only Org post is rejected

ADR-0024 canonicalization strips the title-source line, so `* My Title` with no
content normalizes to nothing. Previously this was accepted and stored an empty
body; it is now a 400. This is a deliberate, user-visible behaviour change.

## Consequences

- `derive_post_title` becomes total — no outer `Option` — because there is no
  nothing-to-store case left to report. `PerformCreationError::EmptyPost` and
  `PerformUpdateError::EmptyPost` retire from the service layer.
- **Markdown bodies are normalized on write for the first time.** Stored bytes
  may differ from submitted bytes, so a body with leading blank lines or
  trailing whitespace gets a different content ETag and stops round-tripping
  byte-identically through AtomPub. Stored Org bodies also shift slightly — they
  regain a terminating newline, fixing a latent inconsistency in the previous
  `canonicalize_org_body`.
- The AtomPub and web create/update boundaries must surface the rejection as a
  400, not a 500.
- Adding a format means extending one match in `canonicalize_body`, not editing
  two call sites in `storage`.
- **This ADR does not license normalizing whitespace elsewhere.** The rule was
  derived from CommonMark and Org rendering behaviour specifically; it is not a
  general "tidy the input" precedent.

## Amendment to ADR-0063

`0063-domain-value-newtype-convention.md` §3 lists `PostBody` as a first user of
`#[str_newtype(infallible)]`. That entry is corrected there, along with the
framing defect #830 identified: §3 discriminates on the constructor's signature
("an invariant that never rejects") where §2 frames the choice invariant-first
("fallible when the value has an invariant"). The signature is a consequence of
the invariant, not the test for it — a type declared `infallible` that turns out
to have a rejecting invariant was mis-declared, which is exactly what happened
to `PostBody` here and to `PostTitle` in #830.
