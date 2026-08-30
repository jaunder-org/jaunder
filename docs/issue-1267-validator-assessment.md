# Issue #1267 — `validator` assessment

## Conclusion — reject repository-wide adoption

**Decision: reject.** The deterministic rule in the approved specification is:

- a family qualifies only if its aggregate net ceiling is positive **and** it
  has no dependency/target, security, maintenance, or accepted-ADR disqualifier;
- broad adoption requires every technically supported family to qualify;
- selective adoption requires at least one qualifying and one non-qualifying
  family; otherwise reject.

No family qualifies. The repository greenfield ceiling is **gross 101 physical
Rust SLOC, replacement 116, net −15**. The field-predicate ceiling is negative
after its required 50 SLOC of shared support, and its secret-bearing uses remain
independently disqualified: they must not expose validator's value-bearing
errors and must preserve cheap-before-expensive and timing behavior. Domain
newtypes have a raw positive subtotal but reverse accepted typed-boundary ADRs;
cross-field checks are negative; every parser, registry, and stateful family has
zero validator-owned behavior. The `common`/web path also has no explicit wasm
support guarantee. Thus there is neither a qualifying family nor a basis for
selective use.

These numbers are a **greenfield, validator-native ceiling**, not migration
feasibility, implementation effort, or an architecture-compatible proposal. They
intentionally exclude dependency acquisition, test work, compatibility
migration, and any accepted-ADR reversal. This assessment authorizes no
migration.

**Assessed HEAD:** `83815df56ceb40d991a317d08fe2417694eeb0fa`.

## Evaluated dependency and fit

The assessment evaluates
`validator = { version = "0.21.0", features = ["derive"] }`. Release metadata
declares Rust **1.88** MSRV (matching the workspace fact at the assessed HEAD).
The source companion is `validator_derive` **0.20.1**; the published `validator`
manifest depends on `validator_derive ^0.20`. It is std-based (no `no_std`
feature) and makes no explicit wasm target guarantee; proc macros execute on the
host. Its API is synchronous:
`Validate::validate(&self) -> Result<(), ValidationErrors>` and context
validation uses synchronous `ValidateArgs::validate_with_args`. It supplies no
parser/canonical-value result, normalization, async I/O, storage, transaction,
atomic-write, cryptographic, multipart, rate-limit, or process-state facility.

Primary sources:
[0.21.0 release record](https://index.crates.io/va/li/validator),
[`validator` manifest](https://github.com/Keats/validator/blob/68f2e33d236f579ae7bf42c82cf2ca7986f176f6/validator/Cargo.toml),
[`validator_derive` manifest](https://github.com/Keats/validator/blob/68f2e33d236f579ae7bf42c82cf2ca7986f176f6/validator_derive/Cargo.toml),
[README attributes](https://github.com/Keats/validator/blob/68f2e33d236f579ae7bf42c82cf2ca7986f176f6/README.md#built-in-validators),
[`Validate`/collection traits](https://github.com/Keats/validator/blob/68f2e33d236f579ae7bf42c82cf2ca7986f176f6/validator/src/traits.rs),
[error types](https://github.com/Keats/validator/blob/68f2e33d236f579ae7bf42c82cf2ca7986f176f6/validator/src/types.rs),
[URL predicate](https://github.com/Keats/validator/blob/68f2e33d236f579ae7bf42c82cf2ca7986f176f6/validator/src/validation/urls.rs),
[email predicate](https://github.com/Keats/validator/blob/68f2e33d236f579ae7bf42c82cf2ca7986f176f6/validator/src/validation/email.rs),
and
[length semantics](https://github.com/Keats/validator/blob/68f2e33d236f579ae7bf42c82cf2ca7986f176f6/validator/src/validation/length.rs).

Exact usable APIs are `#[derive(Validate)]`; `#[validate(length(...))]`,
`range(...)`, `regex(path = *STATIC_REGEX)`, `must_match(other = "...")`,
`email`, `url`, `nested`, `required`, and synchronous
`custom(function = "...")`/`schema(function = "...")`. `length` counts Unicode
scalar values, not graphemes. `url` and email's IDNA work are predicates, not
returned canonical values. Schema checks default to skipping after a field error
and report under `__all__`. Validation errors are HashMap-backed (no stable
presentation order); built-ins add the field value as an error parameter. The
last point is a direct security disqualifier for passwords, tokens, and secret
configuration unless an adapter discards those errors.

## Method, population, and reconciliation

**Population.** Every tracked production Rust item in workspace packages and
build scripts was reviewed: root members `client`, `common`, `csr`, `host`,
`macros`, `server`, `storage`, `web`; standalone `xtask`; and
`tools/{coverage, devtool,doctests}`. Included target/feature-gated production
source is treated as production. `test-support` (the e2e seed/fixture target),
`#[cfg(test)]` bodies, test/bench/example/fixture targets, generated output,
vendored code, non-Rust sources, comments, and parsing with no data-validity
decision are excluded. `server/build.rs` is included;
`server/src/build_staging.rs` is reviewed but excluded because it is filesystem
staging/error sequencing, not a data-validity decision.

**Repeatable mechanical census.** Run this sole mechanical census command from
the repository root. Its declaration union is deliberately narrow: it finds
fallible conversion implementations and explicitly named validation,
parse/normalization/canonicalization function declarations. Full-module manual
review below is the completeness backstop. Standard production exclusions are
part of the command. It produced **147 raw records and 147 deduplicated
`path:line` records** at the assessed HEAD; the appendix reconciles all 147.

```sh
rg -n --no-heading -g '*.rs' -g '!**/tests/**' -g '!**/test_support/**' -g '!test-support/**' \
  -g '!**/benches/**' -g '!**/examples/**' -g '!**/fixtures/**' \
  -e '^[[:space:]]*impl[[:space:]]+(FromStr|TryFrom)' \
  -e '^[[:space:]]*(pub([^()]*)?[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+(validate|parse|normalize|canonical|from_str|try_from)[[:space:]]*\(' \
  common/src host/src storage/src web/src server/src macros/src xtask/src \
  tools/coverage/src tools/devtool/src tools/doctests/src server/build.rs \
  | sort -t: -k1,1 -k2,2n -u
```

The population-manifest command below is not a second search: it enumerates the
manual-review checklist. Broad ranges were re-read and split before counting
positive rows. Adapter twins and calls into an already owned parser are
references, never additive candidates.

**Manual disposition.** Every included module was hand-reviewed after the
mechanical pass. Candidate-bearing modules appear in the ledger. Named
non-candidate disposition is:
`common/{ids,seed,revision_history,trace_field, list_state,mailer,session_user,registration,smtp_tls_mode,user_facing_message, lib}.rs`,
`common/feed/mod.rs`, `common/test_support/**`, all `client/src/**`, all
`csr/src/**`, `server/src/build_staging.rs`, `xtask/src/test_support.rs`,
`xtask/src/elisp_coverage/tests.rs`, `macros/tests/**`, and
`tools/devtool/tests/provision_cli.rs`. All remaining matches in reviewed
modules are `duplicate/reference <ledger owner>` when they invoke a candidate
owner; otherwise they are `E-NODV` (process/filesystem/JSON/trace/coverage
parsing, serialization, rendering, DTO/state, or module wiring without a
data-validity decision). The following deterministic command reproduces the
per-path manifest and is the authoritative disposition key: a path appearing in
the ledger is `candidate <ID>`; a path in the preceding list is its named
exclusion; every other manifest path is `E-NODV` unless its call is explicitly
marked `duplicate/reference` in the ledger.

```sh
git ls-files -- '*.rs' ':!**/tests/**' ':!**/test_support/**' ':!test-support/**' \
  ':!**/benches/**' ':!**/examples/**' ':!**/fixtures/**' | sort
```

SQLite/PostgreSQL atomic-operation twins are one H20 behavior. Candidate-table
references to `common`, `storage`, `MediaManager`, SQLx, and generated macro
implementations are duplicate/reference hits, not new rows.

**Counting.** “Current” is only physical, nonblank, non-comment, non-test Rust
lines that the greenfield design removes wholesale. Mixed lines and any line
whose handwritten responsibility remains are zero. `Gross = Current`;
`Net = Gross − Replacement`. Replacement counts physical lines in the complete
sketches below plus allocated per-row lines. Current/replacement support is
charged once only. This avoids treating parser recognition, normalization,
canonicalization, closed registries, I/O, storage state, transaction semantics,
and custom schema functions as declarative predicate savings.

Primary-family assignment is exclusive and uses this precedence:
**stateful/storage > protocol/grammar > configuration registry > cross-field >
domain newtype > field predicate**. Tags are non-additive secondary behavior.

## Countable validator-native sketches

Only full/partial rows cite a sketch. Line numbers are physical SLOC; allocation
is stated in the ledger so each line is charged exactly once.

### S1 — independent scalar predicates (40 physical SLOC)

```rust
use validator::Validate;
#[derive(Validate)]
struct Nonblank<'a> {
    #[validate(length(min = 1))]
    value: &'a str,
}
#[derive(Validate)]
struct MaxBio<'a> {
    #[validate(length(min = 1, max = 1000))]
    value: &'a str,
}
#[derive(Validate)]
struct Max255<'a> {
    #[validate(length(min = 1, max = 255))]
    value: &'a str,
}
#[derive(Validate)]
struct MaxSummary<'a> {
    #[validate(length(min = 1, max = 500))]
    value: &'a str,
}
#[derive(Validate)]
struct ValidSmtpPort {
    #[validate(range(min = 1, max = 65535))]
    value: u16,
}
```

The five independently instantiable DTOs are 26 shared-support SLOC. The
following 14 one-line adapters are charged once to the named rows (and retain
the surrounding trim, parse, and type construction):

```rust
Nonblank { value: trimmed }.validate().map_err(|_| InvalidAudienceName)?;
MaxBio { value: trimmed }.validate().map_err(|_| InvalidBio)?;
Max255 { value: trimmed }.validate().map_err(|_| InvalidDisplayName)?;
Nonblank { value: trimmed }.validate().map_err(|_| InvalidIdempotencyKey)?;
MaxSummary { value: trimmed }.validate().map_err(|_| InvalidPostSummary)?;
Nonblank { value: trimmed }.validate().map_err(|_| InvalidPostTitle)?;
Nonblank { value: trimmed }.validate().map_err(|_| InvalidSiteTitle)?;
Max255 { value: trimmed }.validate().map_err(|_| InvalidSessionLabel)?;
Nonblank { value: s }.validate().map_err(|_| InvalidPgRolePassword)?;
Nonblank { value: s }.validate().map_err(|_| InvalidSmtpHost)?;
Nonblank { value: s }.validate().map_err(|_| InvalidSmtpUsername)?;
Nonblank { value: s }.validate().map_err(|_| InvalidSmtpPassword)?;
ValidSmtpPort { value: port }.validate().map_err(|_| reject("a port must not be zero".to_owned()))?;
Nonblank { value: value.trim() }.validate().map_err(|_| InvalidSubscriberRef)?;
```

### S2 — independent regex predicates (17 physical SLOC)

```rust
use std::sync::LazyLock;
use regex::Regex;
use validator::Validate;
static USERNAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]+$").unwrap());
static HASH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{64}$").unwrap());
#[derive(Validate)]
struct UsernameInput<'a> {
    #[validate(length(min = 1), regex(path = *USERNAME_RE))]
    value: &'a str,
}
#[derive(Validate)]
struct ContentHashInput<'a> {
    #[validate(regex(path = *HASH_RE))]
    value: &'a str,
}
```

The imports/statics are five shared-support SLOC. C03 owns the five
`UsernameInput` DTO lines plus
`UsernameInput { value: s }.validate().map_err(|_| InvalidUsername)?;` before
the retained lowercase/type construction. C28 owns the five `ContentHashInput`
DTO lines plus
`ContentHashInput { value: s }.validate().map_err(|_| InvalidContentHash)?;`.

### A1 — password predicate and secret-safe adapter (13 physical SLOC; C04)

```rust
use validator::Validate;
#[derive(Validate)]
struct PasswordShape<'a> {
    #[validate(length(min = 8))]
    too_short: &'a str,
    #[validate(length(max = 512))]
    too_long: &'a str,
}
fn validate_password_shape(s: &str) -> Result<(), InvalidPassword> { match (PasswordShape { too_short: s, too_long: s }.validate()) {
    Ok(()) => Ok(()),
    Err(errors) if errors.errors().contains_key("too_short") => Err(InvalidPassword::PasswordTooShort),
    Err(_) => Err(InvalidPassword::PasswordTooLong),
} }
```

`ValidationErrors` never crosses the existing secret-safe boundary.

### A2 — token predicate and secret-safe adapter (10 physical SLOC; C06)

```rust
use std::sync::LazyLock;
use regex::Regex;
use validator::Validate;
static TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]+$").unwrap());
#[derive(Validate)]
struct TokenInput<'a> {
    #[validate(length(min = 1), regex(path = *TOKEN_RE))]
    value: &'a str,
}
fn validate_shape(s: &str) -> Result<(), InvalidTokenShape> { TokenInput { value: s }.validate().map_err(|_| InvalidTokenShape) }
```

### S3 — tag-count predicate (12 physical SLOC)

```rust
use validator::Validate;
#[derive(Validate)]
struct TagsInput<'a> {
    #[validate(length(max = 256))]
    tags: &'a [TagLabel],
}
```

This six-SLOC support is charged once. After retained parsing and deduplication,
C22 replaces its six current lines with:

```rust
let count = out.len();
TagsInput { tags: &out }.validate().map_err(|_|
    TagValidationError::TooMany {
        count,
        max: MAX_TAGS_PER_POST,
    })?;
```

It does not claim Tag/TagLabel canonicalization.

### S4 — host nonblank predicate (12 physical SLOC)

```rust
use validator::Validate;
#[derive(Validate)]
struct Nonblank<'a> {
    #[validate(length(min = 1))]
    value: &'a str,
}
```

This six-SLOC support is charged once. The concrete one-line adapters are:

```rust
Nonblank { value }.validate().map_err(|_| InvalidFeedTitle)?;
Nonblank { value }.validate().map_err(|_| InvalidFeedDescription)?;
Nonblank { value }.validate().map_err(|_| InvalidWorkspaceTitle)?;
Nonblank { value }.validate().map_err(|_| InvalidCollectionTitle)?;
Nonblank { value }.validate().map_err(|_| InvalidCollectionFeedTitle)?;
Nonblank { value: s }.validate().map_err(|_| InvalidStoredPasswordHash)?;
```

Each H6/H7 adapter follows `let value = value.trim();`; H11 deliberately does
not trim. All discard `ValidationErrors` before the existing valueless error.

### S5 — feed-cache equality (12 physical SLOC)

```rust
use validator::Validate;
#[derive(Validate)]
struct FeedCacheFormats {
    #[validate(must_match(other = "representation_format"))]
    path_format: Option<FeedFormat>,
    representation_format: Option<FeedFormat>,
}
```

The seven-SLOC support is charged once. H15 replaces only current line 79 with:

```rust
if (FeedCacheFormats {
    path_format,
    representation_format: Some(representation_format),
}).validate().is_err()
{
```

Its `None` recovery and `MismatchedFeedCacheRowFormat` construction remain.

## Candidate ledger

Abbreviations: **Own** = validator-owned classification (`F` full, `P` partial,
`N` none); **C/R/G/N** = current/replacement/gross/net SLOC; **Retain** is the
complete non-validator responsibility in terse form; **K** = security
constraint; **A** = accepted ADR conflict/constraint; **D** = fit disqualifier.
Every `N` row has `0/0/0/0`, no sketch, and retains all named behavior.

### Common

| ID  | Source (path:range; symbol)                                                         | Family; tags               | Own; sketch              |   C/R/G/N | Retain; K; A; D                                                                              |
| --- | ----------------------------------------------------------------------------------- | -------------------------- | ------------------------ | --------: | -------------------------------------------------------------------------------------------- |
| C01 | `common/src/text.rs:19-30`; `truncate_by_graphemes`                                 | domain; grapheme/shared    | N; —                     |   0/0/0/0 | grapheme truncation; no split; —; no truncation API                                          |
| C02 | `common/src/text.rs:41-59`; `non_empty*`                                            | domain; trim/shared        | N; —                     |   0/0/0/0 | trim/allocation; —; 0063/0065; no normalization                                              |
| C03 | `common/src/username.rs:27-33` (7); `Username::from_str`                            | domain; regex/lowercase    | P; S2 username           |   7/6/7/1 | lowercase/type door; ASCII-only; 0063/0065/0134; no canonical result                         |
| C04 | `common/src/password.rs:29-36` (8); `validate_password_shape`                       | field; secret/cheap        | F; A1                    | 8/13/8/−5 | wrappers/hash/order; validate before hash/no values; 0018/0022/0063/0065; secret errors      |
| C05 | `common/src/password.rs:51-58`; `ProfferedPassword::from_str`                       | domain; secret             | N; —                     |   0/0/0/0 | redacted inbound type; no values; 0063; representation                                       |
| C06 | `common/src/token.rs:28-35` (8); `validate_shape`                                   | field; base64url/secret    | F; A2                    | 8/10/8/−2 | typed doors; cheap/no raw errors; 0018/0022; secret semantics                                |
| C07 | `common/src/token.rs:121-158`; `RawToken`/`TokenHash`                               | domain; redaction          | N; —                     |   0/0/0/0 | hash/type separation; credential safety; 0063; trust boundary                                |
| C08 | `common/src/invite.rs:29-35`; `ProfferedInviteCode`                                 | domain; secret             | N; —                     |   0/0/0/0 | secret inbound type; no leakage; 0063/0065; representation                                   |
| C09 | `common/src/invite.rs:44-52`; `InviteTtlHours`                                      | domain; range              | N; —                     |   0/0/0/0 | generated `NumNewtype` parse/SQL invariant; 0063/0071; no removable predicate                |
| C10 | `common/src/auth.rs:35-56`; `parse_basic_auth`                                      | protocol; Base64/UTF-8     | N; —                     |   0/0/0/0 | all wire grammar; opaque errors/order; 0014/0018/0022; parser                                |
| C11 | `common/src/audience.rs:28-30` (3); `AudienceName`                                  | domain; trim               | P; S1 Nonblank adapter   |   3/1/3/2 | trim/spelling; —; 0063/0065; typed boundary                                                  |
| C12 | `common/src/bio.rs:34-36` (3); `Bio`                                                | domain; trim/cap           | P; S1 MaxBio adapter     |   3/1/3/2 | trim/optionality; bound; 0063/0065; typed boundary                                           |
| C13 | `common/src/display_name.rs:37-39` (3); `DisplayName`                               | domain; trim/cap           | P; S1 Max255 adapter     |   3/1/3/2 | trim/case; bound; 0063/0065; typed boundary                                                  |
| C14 | `common/src/email.rs:52-74`; `Email`                                                | protocol; RFC/domain norm  | N; —                     |   0/0/0/0 | parser/canonical output; bounded error; 0063/0065; predicate only                            |
| C15 | `common/src/mailbox.rs:58-135`; `Mailbox`                                           | protocol; RFC/render       | N; —                     |   0/0/0/0 | grammar/quoted render; avoid ambiguity; 0063; parser                                         |
| C16 | `common/src/etag.rs:13-80`; ETag                                                    | protocol; wire/storage     | N; —                     |   0/0/0/0 | quoted grammar/canonicality; conditional write; —; parser                                    |
| C17 | `common/src/idempotency_key.rs:27-29` (3); key                                      | domain; trim/write         | P; S1 Nonblank adapter   |   3/1/3/2 | trim/type/atomic coupling; atomic write; 0063; typed boundary                                |
| C18 | `common/src/post_body.rs:59-70`; `PostBody`                                         | domain; multiline/verbatim | N; —                     |   0/0/0/0 | nonblank-line semantics; no trim; 0105; not built-in                                         |
| C19 | `common/src/post_summary.rs:38-40` (3); summary                                     | domain; trim/cap           | P; S1 MaxSummary adapter |   3/1/3/2 | trim/derived truncation; Unicode; 0063/0105; typed boundary                                  |
| C20 | `common/src/post_title.rs:39-41` (3); title                                         | domain; trim               | P; S1 Nonblank adapter   |   3/1/3/2 | trim; —; 0063/0065; typed boundary                                                           |
| C21 | `common/src/slug.rs:13-115`; `Slug`                                                 | protocol; NFC/grapheme     | N; —                     |   0/0/0/0 | normalization/grammar; public identity; 0063; no normalization                               |
| C22 | `common/src/tag.rs:143-148` (6); Tag/list                                           | domain; canonical/list     | P; S3 adapter            |   6/6/6/0 | parsing/dedup/canonicality; stable identity; 0063/0065/0068; typed boundary                  |
| C23 | `common/src/site.rs:46-48` (3); `SiteTitle`                                         | domain; trim               | P; S1 Nonblank adapter   |   3/1/3/2 | trim; —; 0063/0065; typed boundary                                                           |
| C24 | `common/src/session_label.rs:40-42` (3); label                                      | domain; trim/lossy         | P; S1 Max255 adapter     |   3/1/3/2 | trusted fallback; bound; 0063; typed boundary                                                |
| C25 | `common/src/root_relative_url.rs:1-45`; URL                                         | protocol; injection        | N; —                     |   0/0/0/0 | URL grammar; reject authority; 0073; no parse output                                         |
| C26 | `common/src/tagged_url.rs:38-120`; URL                                              | protocol; role/canonical   | N; —                     |   0/0/0/0 | role/parser/canonicality; URL role safety; 0073/0112; predicate only                         |
| C27 | `common/src/time.rs:8-110`; time/date                                               | protocol; RFC3339          | N; —                     |   0/0/0/0 | parse/canonical/date; —; 0063/0072; parser                                                   |
| C28 | `common/src/media.rs:150-154` (5), `common/src/media.rs:544-549` (6); `ContentHash` | field; path safety         | F; S2 hash               | 11/6/11/5 | trusted producer only; validate before path; 0080/0084; current-door conflict                |
| C29 | `common/src/media.rs:308-533`; `Filename`                                           | protocol; path/encoding    | N; —                     |   0/0/0/0 | safe leaf/encode/budget/truncate; traversal safety; 0080/0084/0140; grammar                  |
| C30 | `common/src/media.rs:711-875`; media form                                           | protocol; URL/ownership    | N; —                     |   0/0/0/0 | URL/layout/identity; no userinfo; 0073/0080/0140; parser                                     |
| C31 | `common/src/pagination.rs:20-29`; `PageSize`                                        | domain; range/clamp        | N; —                     |   0/0/0/0 | generated `NumNewtype` invariant, clamp/+1 semantics; 0019/0071/0092; no removable predicate |
| C32 | `common/src/pagination.rs:55-62`; offset                                            | domain; range              | N; —                     |   0/0/0/0 | generated `NumNewtype` invariant/sqlx type; 0019/0071; no removable predicate                |
| C33 | `common/src/pagination.rs:79-137`; limit                                            | domain; coupled            | N; —                     |   0/0/0/0 | generated `NumNewtype` invariant/has-more helpers; 0019/0092; no removable predicate         |
| C34 | `common/src/backup.rs:20-193`; backup config                                        | registry; cron/path        | N; —                     |   0/0/0/0 | registry/grammar/policy; safe config; 0102; registry                                         |
| C35 | `common/src/pg_identifier.rs:14-50`; ID                                             | registry; SQL grammar      | N; —                     |   0/0/0/0 | contextual grammar; injection boundary; —; parser                                            |
| C36 | `common/src/pg_role_password.rs:34-36` (3); password                                | domain; secret             | P; S1 Nonblank adapter   |   3/1/3/2 | redaction; no values; 0063; typed boundary                                                   |
| C37 | `common/src/smtp_host.rs:37-39` (3); host                                           | domain; config             | P; S1 Nonblank adapter   |   3/1/3/2 | config type; —; 0063; typed boundary                                                         |
| C38 | `common/src/smtp_username.rs:28-30` (3); user                                       | domain; config             | P; S1 Nonblank adapter   |   3/1/3/2 | config type; credential care; 0063; typed boundary                                           |
| C39 | `common/src/smtp_password.rs:46-48` (3); password                                   | domain; secret             | P; S1 Nonblank adapter   |   3/1/3/2 | redaction; no values; 0063; typed boundary/security                                          |
| C40 | `common/src/smtp_port.rs:69-71` (3); port                                           | domain; integer            | P; S1 port adapter       |   3/1/3/2 | retained parse and actionable `InvalidSmtpPort { value, reason }`; —; 0063                   |
| C41 | `common/src/smtp_sender.rs:11-58`; sender                                           | protocol; mailbox          | N; —                     |   0/0/0/0 | mailbox parser/spelling; syntax; 0063; parser                                                |
| C42 | `common/src/visibility.rs:97-99` (3); `SubscriberRef`                               | domain; nonblank/enum      | P; S1 Nonblank adapter   |   3/1/3/2 | enum/sqlx/ref semantics; identity; 0102/0151; typed boundary                                 |
| C43 | `common/src/visibility.rs:232-301`; audience transforms                             | cross-field; nonwidening   | N; —                     |   0/0/0/0 | private/default projection; never widen; 0151; policy                                        |
| C44 | `common/src/org.rs:115-329`; Org helpers                                            | cross-field; normalization | N; —                     |   0/0/0/0 | parse/lifecycle/body/audience; explicit order; 0101/0105; custom schema retains all          |
| C45 | `common/src/render.rs:1-248`; format/HTML                                           | registry; trust            | N; —                     |   0/0/0/0 | sanitizer/trust door; HTML safety; 0079/0123/0105; not validation                            |
| C46 | `common/src/client_telemetry.rs:9-72`; event                                        | registry; closed           | N; —                     |   0/0/0/0 | protocol vocabulary; bounded telemetry; 0102; registry                                       |
| C47 | `common/src/local_storage_key.rs:1-27`; key                                         | registry; closed           | N; —                     |   0/0/0/0 | key vocabulary; —; 0102; registry                                                            |
| C48 | `common/src/feed/grammar.rs:1-53`; format/surface                                   | protocol; closed URL       | N; —                     |   0/0/0/0 | token/map/URL generation; canonical URL; 0073/0102; parser                                   |

Common shared support: **SS-C-1 = S1 DTOs 26 + S2 imports/statics 5 + S3 DTO 6 =
C/R/G/N 0/37/0/−37**; allocated once.

### Host/storage

| ID  | Source (path:range; symbol)                                                                                                 | Family; tags                  | Own; sketch     |  C/R/G/N | Retain; K; A; D                                                                    |
| --- | --------------------------------------------------------------------------------------------------------------------------- | ----------------------------- | --------------- | -------: | ---------------------------------------------------------------------------------- |
| H1  | `host/src/config_key.rs:47-57,69-160,176-186,192-223`; registry                                                             | registry; parser-discard      | N; —            |  0/0/0/0 | dispatch/parser/errors; secret-safe; 0102; no dynamic parser                       |
| H2  | `host/src/auth.rs:60-69,92-150`; credential                                                                                 | protocol; auth                | N; —            |  0/0/0/0 | header/cookie/precedence; malformed explicit rejects; 0018; HTTP grammar           |
| H3  | `host/src/capture.rs:67-107,116-139`; capture/stream                                                                        | protocol; env/I/O             | N; —            |  0/0/0/0 | path trim/I/O/enum; prepare first; —; I/O/parser                                   |
| H4  | `host/src/feed/feed_path.rs:30-70,105-141`; path                                                                            | protocol; canonical           | N; —            |  0/0/0/0 | grammar/child parse/identity; stable cache key; 0063/0071; no result               |
| H5  | `host/src/feed/settings.rs:14-41`; settings                                                                                 | domain; numeric/raw config    | N; —            |  0/0/0/0 | generated `NumNewtype` parse/SQL invariant; 0063/0071/0102; no removable predicate |
| H6  | `host/src/feed/metadata.rs:32-34` (3), `host/src/feed/metadata.rs:68-70` (3); titles                                        | domain; trim                  | F; S4 adapters  |  6/2/6/4 | trim/types; map safely; 0063/0071; boundary                                        |
| H7  | `host/src/atompub/title.rs:22-24` (3), `host/src/atompub/title.rs:51-53` (3), `host/src/atompub/title.rs:86-88` (3); titles | domain; trim                  | F; S4 adapters  |  9/3/9/6 | trim/types; protocol spelling; 0063/0071; boundary                                 |
| H8  | `host/src/invite.rs:30-47`; invite                                                                                          | domain; secret                | N; —            |  0/0/0/0 | parser/type; cheap/no disclosure; 0022; no parser                                  |
| H9  | `host/src/password.rs:15-31,49-118`; password                                                                               | domain; Argon2                | N; —            |  0/0/0/0 | secret/crypto; cost/order; 0018/0022; no crypto                                    |
| H10 | `host/src/token.rs:19-75`; token                                                                                            | domain; randomness/hash       | N; —            |  0/0/0/0 | generate/decode/hash; no raw token; —; no decoder                                  |
| H11 | `host/src/stored_password_hash.rs:29-31` (3); hash                                                                          | domain; secret                | F; S4 adapter   |  3/1/3/2 | type/sqlx; discard value errors; 0063/0071; boundary                               |
| H12 | `storage/src/db.rs:28-30,44-115,178-221`; DB                                                                                | registry; URL/redact/I/O      | N; —            |  0/0/0/0 | parse/routing/redaction/I/O; no password leak; —; no result                        |
| H13 | `storage/src/instance_identity.rs:32-64`; identity                                                                          | stateful; UUID                | N; —            |  0/0/0/0 | UUID/atomic singleton; atomicity; —; no DB                                         |
| H14 | `storage/src/audiences.rs:102-132,241-291`; targets                                                                         | stateful; ownership           | N; —            |  0/0/0/0 | lookup/conflicts; foreign=absent; —; async                                         |
| H15 | `storage/src/feed_cache.rs:79` (1); row                                                                                     | cross-field; must-match       | P; S5 condition | 1/5/1/−4 | recovery/error/SQL decode; reject corrupt row; —; larger                           |
| H16 | `storage/src/feed_events.rs:46-152,157-169,304-438`; events                                                                 | stateful; corrupt/claim       | N; —            |  0/0/0/0 | parse/partition/claim; atomic dialect claim; —; async                              |
| H17 | `storage/src/media_manager.rs:139-149,187-363`; manager                                                                     | stateful; quota/files         | N; —            |  0/0/0/0 | sanitize/quota/dedup/finalize; atomic cleanup; 0080/0084; I/O                      |
| H18 | `storage/src/media.rs:45-290`; store                                                                                        | stateful; ownership/locks     | N; —            |  0/0/0/0 | DB state/locks; preserve state; —; async                                           |
| H19 | `storage/src/posts.rs:598-655,3490-3733`; posts                                                                             | stateful; idempotency         | N; —            |  0/0/0/0 | policy/tags/locks/tx; atomic write; 0125; transaction                              |
| H20 | `storage/src/atomic.rs:1-131`; generic + adapter twins                                                                      | stateful; secret/tx/duplicate | N; —            |  0/0/0/0 | claims/writes/tx; cheap check/lock; 0021/0022; async                               |
| H21 | `storage/src/email.rs:1-180`; verify store                                                                                  | stateful; token claim         | N; —            |  0/0/0/0 | SQL token state; atomic claim; —; database                                         |
| H22 | `storage/src/sessions.rs:1-180`; sessions                                                                                   | stateful; auth                | N; —            |  0/0/0/0 | hash/lookup/touch; secret-safe; 0018; database                                     |
| H23 | `storage/src/users.rs:211-337,383-425,432-439,495-505`; users                                                               | stateful; timing/shared       | N; —            |  0/0/0/0 | Argon2/SQL/conflict; dummy timing; 0018/0114; crypto/async                         |
| H24 | `storage/src/site_config.rs:1-500`; config                                                                                  | stateful; typed/secret        | N; —            |  0/0/0/0 | decode/default/upsert; defensive/no secret; 0102; storage                          |
| H25 | `storage/src/helpers.rs:240-615`; helpers                                                                                   | stateful; dummy/shared        | N; —            |  0/0/0/0 | decode/classify/dummy hash; timing; 0018; no primitive                             |
| H26 | `storage/src/backup/restore_validation.rs:83-121,129-175,204-214,234-460`; restore                                          | stateful; generated rows      | N; —            |  0/0/0/0 | typed parse/report/write safety; validate before writes; —; parser/state           |

Host/storage replacement support: **SS-HS-1 = S4 0/6/0/−6** and **SS-HS-2 = S5
0/7/0/−7**. They are each allocated exactly once; H20 generic behavior, adapter
variants, H23 generic logic, and H25 helpers are not duplicated.

### Boundary (`web`/`server`; client and csr have no candidate)

All rows are `N; —; 0/0/0/0`; the final field gives retained responsibility,
security, ADR, and disqualifier.

| ID  | Source; exclusive family; tags                                               | Retain; K; A; D                                                                                 |
| --- | ---------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| B01 | `web/src/forms/field.rs:11-17,45-157`; domain; wasm/client                   | reactive `FromStr` UI/presence; server authority; 0063/0065/0070/0072; no parser/wasm guarantee |
| B02 | `web/src/forms/submit_gate.rs:7-24`; cross-field; pending                    | atomic request/pending gate; server authority; 0113; no reactive async                          |
| B03 | `web/src/posts/parse.rs:39-61`; protocol; route                              | segments/date/typed route; no-round-trip 404; 0063; no parser                                   |
| B04 | `web/src/posts/edit_state.rs:63-100`; cross-field; DST                       | edited-state/timezone conversion; revalidate server; 0072; no temporal semantics                |
| B05 | `web/src/posts/api.rs:67-83`; cross-field; lifecycle                         | clock/lifecycle construction; pre-write; —; custom schema retains all                           |
| B06 | `web/src/posts/api.rs:88-96`; protocol; Org                                  | Org normalization/error map; pre-write; —; no normalization                                     |
| B07 | `web/src/posts/api.rs:99-107`; stateful; audience                            | async author target auth; pre-mutation; —; async I/O                                            |
| B08 | `web/src/posts/api.rs:475-477,479-524,526-543`; cross-field; Org             | defaults/lifecycle policy; pre-write; —; conditional policy                                     |
| B09 | `web/src/tags/input_logic.rs:47-55`; domain; wasm/canonical                  | TagLabel→identity/label; server authority; 0063/0065/0068; no canonical result                  |
| B10 | `web/src/email/status.rs:24-27`; domain; token                               | RawToken/error collapse; no token detail; 0063/0065; secret parameters                          |
| B11 | `web/src/subscriptions/server.rs:13-26`; stateful; auth                      | lookup/self policy; before write; —; async state                                                |
| B12 | `web/src/auth/server.rs:88-129,139-228,272-280`; stateful; auth              | transport/session/metrics/operator policy; timing/no values; 0018/0114; async/security          |
| B13 | `web/src/media/api.rs:269-291,296-341`; stateful; multipart                  | framing/stream/manager; limits/auth/atomic write; —; I/O                                        |
| B14 | `server/src/media.rs:102-150`; protocol; media path                          | serde segments/fanout/encoding; reject drift before open; 0080/0063; no parser                  |
| B15 | `server/src/runtime_file.rs:19-30,38-58,77-82,105-150`; stateful; procfs     | I/O/liveness/atomic rename; fail closed; 0035; state/I/O                                        |
| B16 | `server/src/atompub/mapping.rs:75-81,99-165`; protocol; AtomPub              | entry/media/leniency normalization; preserve sole-body failure; 0023; parser                    |
| B17 | `server/src/atompub/posts.rs:126-132`; protocol; header                      | decode/fallback/idempotency; retry semantics; —; no header parser                               |
| B18 | `server/src/atompub/posts.rs:138-164,177-235`; stateful; audience            | tag/Org/async authorization; pre-write; 0023; async/parser                                      |
| B19 | `server/src/atompub/posts.rs:237-264`; cross-field; lifecycle                | Atom fallback policy; pre-write; 0023; custom schema                                            |
| B20 | `server/src/atompub/guards.rs:13-22,48-55`; stateful; auth/config            | user match/base URL; forbidden/precondition; 0023/0073; async config                            |
| B21 | `server/src/cli.rs:62-68,98-105,158-171`; registry; CLI                      | PG URL/ID/secret elision; scheme before SQLx; —; no parser result                               |
| B22 | `server/src/client_telemetry.rs:184-207,218-270`; stateful; cookie/body/rate | cookie-only/bounds/limiter; cheap before deserialize; 0107; async/body                          |
| B23 | `server/src/feed/handlers.rs:22-29`; protocol; extension                     | closed token mapping; soft malformed route; —; no enum parser                                   |

### Tooling/build

All rows are `N; —; 0/0/0/0`. Source SLOC is reviewed context, not removable
SLOC; it is included to make the non-additive inventory auditable.

| ID          | Source (physical context SLOC); family; tags                                                                                 | Retain; K; A; D                                                                            |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| M-SHARED    | `macros/src/lib.rs:568-612,638-657` (73); protocol; shared macro shape                                                       | syn shape/diagnostics; fail compilation; 0062/0063; runtime validator cannot inspect input |
| M-NUM       | `macros/src/num_newtype.rs:30-69,365-386,392-448` (109); field; macro options                                                | option/type/bound grammar; reject contradictory bounds; 0063; proc-macro grammar           |
| M-STR       | `macros/src/str_newtype.rs:284-362` (76); registry; secret/bridges                                                           | flag grammar/API surface; fail closed secret traits; 0063/0071; compile config             |
| M-TEXT      | `macros/src/text_enum.rs:249-274,277-332` (73); protocol; enum                                                               | option grammar/named errors; fail closed; 0091; TokenStream                                |
| M-SQLX      | `macros/src/sqlx_bridge_derive.rs:18-93,101-113` (57); registry; SQLx                                                        | bridge generation/validating decode; corrupt rows rejected; 0071/0091; codegen             |
| M-SERVER    | `macros/src/server_fn.rs:43-80,91-134,136-155,157-215` (146); protocol; wire/telemetry                                       | source/meta/route policy; default deny/no secrets; 0011/0070/0082/0120; compile-time       |
| X-IDS       | `xtask/src/ids.rs:7-12,24-39,43-54,58-63` (44); protocol; collection                                                         | prefix parse/duplicate/parity; deterministic failure; 0036/0085; collection policy         |
| X-ADR       | `xtask/src/adr_readme.rs:77-105,107-130,132-147,176-207,210-216,220-238,241-260,263-280,282-293` (153); protocol; projection | Markdown/FS/spans/rewrite; fail closed; 0127/0048/0152; parser/I/O                         |
| B-BUILD     | `server/build.rs:36-63` (15); stateful; release                                                                              | artifact decision; declared release fails closed; 0003/0008/0142; build state              |
| D-PROVISION | `tools/devtool/src/provision.rs:21-45,54-61` (23); registry; OS path                                                         | flag/env/non-UTF8 policy; early failure; —; OS/path                                        |
| D-CHECK     | `tools/devtool/src/check.rs:165-316,341-355` (164); registry; commands                                                       | registry/cardinality/side effects; before spawn; —; command policy                         |
| D-CSR       | `tools/devtool/src/csr_bundle.rs:79-120,232-242` (53); protocol; JS/Wasm                                                     | glue/binary shape; check before mutation; 0106 context; binary grammar                     |

## Arithmetic and rule application

| Exclusive primary family   | Rows / allocated support                                                                                            | Current | Replacement |   Gross |     Net | Qualification                                                        |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------: | ----------: | ------: | ------: | -------------------------------------------------------------------- |
| Stateful/storage           | B07,B11–B13,B15,B18,B20,B22; H13–H26; B-BUILD                                                                       |       0 |           0 |       0 |       0 | no validator-owned async/state/I/O/transaction behavior              |
| Protocol/grammar           | C10,C14–C16,C21,C25–C27,C29–C30,C41,C48; B03,B06,B14,B16–B17,B23; H2–H4; M-SHARED,M-TEXT,M-SERVER,X-IDS,X-ADR,D-CSR |       0 |           0 |       0 |       0 | parser/canonicalization/codegen disqualifier                         |
| Configuration registry     | C34,C35,C45–C47; B21; H1,H12; M-STR,M-SQLX,D-PROVISION,D-CHECK                                                      |       0 |           0 |       0 |       0 | closed registry/dispatch disqualifier                                |
| Cross-field                | C43,C44; B02,B04,B05,B08,B19; H15                                                                                   |       1 |           5 |       1 |      −4 | equality adapter is larger; other policy remains custom              |
| Domain newtype             | C01–C03,C05,C07–C09,C11–C13,C17–C24,C31–C40,C42; B01,B09,B10; H5–H11                                                |      73 |          32 |      73 |      41 | positive raw net, but typed-boundary/normalization/ADR disqualifiers |
| Field predicate            | C04,C06,C28; M-NUM                                                                                                  |      27 |          29 |      27 |      −2 | required secret-safe adapter/error work is larger                    |
| Shared replacement support | SS-C-1; SS-HS-1,SS-HS-2                                                                                             |       0 |          50 |       0 |     −50 | complete independently constructible DTO support                     |
| **Repository total**       | every ledger row and support row above                                                                              | **101** |     **116** | **101** | **−15** | **reject**                                                           |

The table is the canonical reconciliation. Common contributes **82** current
SLOC; host/storage contributes **19**. Complete replacement support is 50 SLOC
and row-local replacement is 66 SLOC, totaling 116. No import, derive, type,
brace, static, or adapter line is unallocated.

```text
current = common 82 + host/storage 19 + boundary 0 + tooling 0 = 101
replacement = row-local 66 + shared support 50 = 116
net = 101 − 116 = −15
```

The raw positive domain-newtype subtotal and field-predicate ceiling cannot
qualify because of independent accepted-ADR, security, and target disqualifiers.
The repository result is therefore rejection under the approved rule.

## Explicit ambiguities and non-counted costs

- `validator`'s `email` and `url` checks may be useful predicates, but they do
  not return Jaunder's parsed/canonical domain values; no grammar/normalization
  SLOC is credited.
- A `custom` or `schema` attribute that contains the existing parser, policy, or
  async-adapter logic owns no behavior itself. It is not a partial replacement.
- Full means no **validation predicate** remains custom in the row's removed
  slice. Retained secret representation, trusted construction, parsing, I/O, and
  storage behavior is expressly outside that predicate, never silently
  discarded.
- Dependency download, binary size, compile-time cost, tests, error-contract
  adaptation, accepted-ADR reversal, and compatibility migration are real costs
  but deliberately are not converted to LOC. Adding them cannot improve the
  ceiling.

Relevant accepted decisions that a validator-centric cutover would contradict
include ADR-0063 (single `FromStr` normalizing boundary and secret profiles),
ADR-0065 (shared typed client prevalidation), ADR-0071 (validating SQLx bridge),
ADR-0073 (typed URL normalization), ADR-0079/0123 (RenderedHtml trusted doors),
ADR-0080/0084/0140 (media canonical spelling and strict address), ADR-0102
(closed config key registry), ADR-0105 (format-aware nonblank Post body),
ADR-0114/0018 (timing equalization), ADR-0022 (cheap validation before expensive
secret work), and ADR-0021/0092 (transaction and bounded SQLite behavior). Those
qualitative constraints are separate from, and stricter than, the negative
greenfield LOC ceiling.

## Mechanical-hit reconciliation appendix

The exact declaration-union command below produced **147 raw records and 147
deduplicated `path:line` records** at the assessed HEAD. Each output record
appears once in this table; no default disposition is applied outside this
table. `E-NODV` is the named exclusion for a declaration that is
DTO/state/render/transport/process/coverage machinery without a data-validity
decision.

```sh
rg -n --no-heading -g '*.rs' -g '!**/tests/**' -g '!**/test_support/**' -g '!test-support/**' -g '!**/benches/**' -g '!**/examples/**' -g '!**/fixtures/**' -e '^[[:space:]]*impl[[:space:]]+(FromStr|TryFrom)' -e '^[[:space:]]*(pub([^()]*)?[[:space:]]+)?(async[[:space:]]+)?fn[[:space:]]+(validate|parse|normalize|canonical|from_str|try_from)[[:space:]]*\(' common/src host/src storage/src web/src server/src macros/src xtask/src tools/coverage/src tools/devtool/src tools/doctests/src server/build.rs | sort -t: -k1,1 -k2,2n -u
```

| path:line                                                 | matched construct                                                                      | disposition            |
| --------------------------------------------------------- | -------------------------------------------------------------------------------------- | ---------------------- |
| `common/src/audience.rs:23`                               | `impl FromStr for AudienceName {`                                                      | candidate C11          |
| `common/src/audience.rs:26`                               | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C11          |
| `common/src/backup.rs:99`                                 | `impl FromStr for BackupSchedule {`                                                    | candidate C34          |
| `common/src/backup.rs:102`                                | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C34          |
| `common/src/backup.rs:149`                                | `impl FromStr for DestinationPath {`                                                   | candidate C34          |
| `common/src/backup.rs:152`                                | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C34          |
| `common/src/bio.rs:29`                                    | `impl FromStr for Bio {`                                                               | candidate C12          |
| `common/src/bio.rs:32`                                    | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C12          |
| `common/src/display_name.rs:32`                           | `impl FromStr for DisplayName {`                                                       | candidate C13          |
| `common/src/display_name.rs:35`                           | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C13          |
| `common/src/email.rs:52`                                  | `impl FromStr for Email {`                                                             | candidate C14          |
| `common/src/email.rs:55`                                  | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C14          |
| `common/src/etag.rs:60`                                   | `impl FromStr for ETag {`                                                              | candidate C16          |
| `common/src/etag.rs:63`                                   | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C16          |
| `common/src/idempotency_key.rs:22`                        | `impl FromStr for IdempotencyKey {`                                                    | candidate C17          |
| `common/src/idempotency_key.rs:25`                        | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C17          |
| `common/src/invite.rs:29`                                 | `impl FromStr for ProfferedInviteCode {`                                               | candidate C08          |
| `common/src/invite.rs:32`                                 | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C08          |
| `common/src/mailbox.rs:58`                                | `impl FromStr for Mailbox {`                                                           | candidate C15          |
| `common/src/mailbox.rs:61`                                | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C15          |
| `common/src/media.rs:146`                                 | `impl FromStr for ContentHash {`                                                       | candidate C28          |
| `common/src/media.rs:149`                                 | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C28          |
| `common/src/media.rs:308`                                 | `impl FromStr for Filename {`                                                          | candidate C29          |
| `common/src/media.rs:311`                                 | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C29          |
| `common/src/media.rs:711`                                 | `impl FromStr for MediaReferenceForm {`                                                | candidate C30          |
| `common/src/media.rs:714`                                 | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C30          |
| `common/src/media.rs:952`                                 | `impl FromStr for ContentType {`                                                       | named exclusion E-NODV |
| `common/src/media.rs:955`                                 | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `common/src/media.rs:2020`                                | `fn canonical(raw: &str) -> Filename {`                                                | named exclusion E-NODV |
| `common/src/org.rs:594`                                   | `fn normalize(source: &str) -> OrgNormalization {`                                     | named exclusion E-NODV |
| `common/src/password.rs:51`                               | `impl FromStr for ProfferedPassword {`                                                 | candidate C05          |
| `common/src/password.rs:54`                               | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C05          |
| `common/src/pg_identifier.rs:23`                          | `impl FromStr for PgRoleName {`                                                        | candidate C35          |
| `common/src/pg_identifier.rs:26`                          | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C35          |
| `common/src/pg_identifier.rs:43`                          | `impl FromStr for PgDatabaseName {`                                                    | candidate C35          |
| `common/src/pg_identifier.rs:46`                          | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C35          |
| `common/src/pg_role_password.rs:30`                       | `impl FromStr for PgRolePassword {`                                                    | candidate C36          |
| `common/src/pg_role_password.rs:33`                       | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C36          |
| `common/src/post_body.rs:59`                              | `impl FromStr for PostBody {`                                                          | candidate C18          |
| `common/src/post_body.rs:65`                              | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C18          |
| `common/src/post_body.rs:77`                              | `fn parse(s: &str) -> PostBody {`                                                      | named exclusion E-NODV |
| `common/src/post_summary.rs:33`                           | `impl FromStr for PostSummary {`                                                       | candidate C19          |
| `common/src/post_summary.rs:36`                           | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C19          |
| `common/src/post_title.rs:34`                             | `impl FromStr for PostTitle {`                                                         | candidate C20          |
| `common/src/post_title.rs:37`                             | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C20          |
| `common/src/root_relative_url.rs:32`                      | `impl FromStr for RootRelativeUrl {`                                                   | candidate C25          |
| `common/src/root_relative_url.rs:35`                      | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C25          |
| `common/src/session_label.rs:35`                          | `impl FromStr for SessionLabel {`                                                      | candidate C24          |
| `common/src/session_label.rs:38`                          | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C24          |
| `common/src/site.rs:41`                                   | `impl FromStr for SiteTitle {`                                                         | candidate C23          |
| `common/src/site.rs:44`                                   | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C23          |
| `common/src/slug.rs:39`                                   | `impl FromStr for Slug {`                                                              | candidate C21          |
| `common/src/slug.rs:42`                                   | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C21          |
| `common/src/smtp_host.rs:33`                              | `impl FromStr for SmtpHost {`                                                          | candidate C37          |
| `common/src/smtp_host.rs:36`                              | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C37          |
| `common/src/smtp_password.rs:32`                          | `impl FromStr for SmtpPassword {`                                                      | candidate C39          |
| `common/src/smtp_password.rs:35`                          | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C39          |
| `common/src/smtp_port.rs:60`                              | `impl FromStr for SmtpPort {`                                                          | candidate C40          |
| `common/src/smtp_port.rs:63`                              | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C40          |
| `common/src/smtp_sender.rs:47`                            | `impl FromStr for SmtpSender {`                                                        | candidate C41          |
| `common/src/smtp_sender.rs:50`                            | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C41          |
| `common/src/smtp_username.rs:24`                          | `impl FromStr for SmtpUsername {`                                                      | candidate C38          |
| `common/src/smtp_username.rs:27`                          | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C38          |
| `common/src/tagged_url.rs:109`                            | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C26          |
| `common/src/tag.rs:26`                                    | `impl FromStr for Tag {`                                                               | candidate C22          |
| `common/src/tag.rs:29`                                    | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C22          |
| `common/src/tag.rs:68`                                    | `impl FromStr for TagLabel {`                                                          | candidate C22          |
| `common/src/tag.rs:71`                                    | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C22          |
| `common/src/time.rs:63`                                   | `impl FromStr for UtcInstant {`                                                        | candidate C27          |
| `common/src/time.rs:66`                                   | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C27          |
| `common/src/token.rs:121`                                 | `impl FromStr for RawToken {`                                                          | candidate C07          |
| `common/src/token.rs:124`                                 | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C07          |
| `common/src/token.rs:152`                                 | `impl FromStr for TokenHash {`                                                         | candidate C07          |
| `common/src/token.rs:155`                                 | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C07          |
| `common/src/username.rs:23`                               | `impl FromStr for Username {`                                                          | candidate C03          |
| `common/src/username.rs:26`                               | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate C03          |
| `common/src/visibility.rs:93`                             | `impl FromStr for SubscriberRef {`                                                     | candidate C42          |
| `common/src/visibility.rs:96`                             | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | candidate C42          |
| `host/src/atompub/title.rs:17`                            | `impl FromStr for WorkspaceTitle {`                                                    | candidate H7           |
| `host/src/atompub/title.rs:20`                            | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | candidate H7           |
| `host/src/atompub/title.rs:46`                            | `impl FromStr for CollectionTitle {`                                                   | candidate H7           |
| `host/src/atompub/title.rs:49`                            | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | candidate H7           |
| `host/src/atompub/title.rs:81`                            | `impl FromStr for CollectionFeedTitle {`                                               | candidate H7           |
| `host/src/atompub/title.rs:84`                            | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | candidate H7           |
| `host/src/capture.rs:131`                                 | `pub fn parse(key: &str) -> Option<Self> {`                                            | named exclusion E-NODV |
| `host/src/config_key.rs:104`                              | `pub fn validate(self, raw: &str) -> Result<(), InvalidSiteConfigValue> {`             | named exclusion E-NODV |
| `host/src/config_key.rs:219`                              | `pub fn validate(self, raw: &str) -> Result<(), InvalidUserConfigValue> {`             | named exclusion E-NODV |
| `host/src/feed/feed_path.rs:38`                           | `pub fn canonical(surface: &FeedSurface, format: FeedFormat) -> Self {`                | candidate H4           |
| `host/src/feed/feed_path.rs:62`                           | `impl FromStr for FeedPath {`                                                          | candidate H4           |
| `host/src/feed/feed_path.rs:65`                           | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate H4           |
| `host/src/feed/feed_path.rs:105`                          | `pub fn parse(path: &str) -> Option<(FeedSurface, FeedFormat)> {`                      | named exclusion E-NODV |
| `host/src/feed/metadata.rs:27`                            | `impl FromStr for FeedTitle {`                                                         | candidate H6           |
| `host/src/feed/metadata.rs:30`                            | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | candidate H6           |
| `host/src/feed/metadata.rs:63`                            | `impl FromStr for FeedDescription {`                                                   | candidate H6           |
| `host/src/feed/metadata.rs:66`                            | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | candidate H6           |
| `host/src/invite.rs:30`                                   | `impl FromStr for InviteCode {`                                                        | candidate H8           |
| `host/src/invite.rs:33`                                   | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate H8           |
| `host/src/invite.rs:39`                                   | `impl TryFrom<ProfferedInviteCode> for InviteCode {`                                   | candidate H8           |
| `host/src/invite.rs:45`                                   | `fn try_from(p: ProfferedInviteCode) -> Result<Self, Self::Error> {`                   | candidate H8           |
| `host/src/password.rs:15`                                 | `impl FromStr for Password {`                                                          | candidate H9           |
| `host/src/password.rs:18`                                 | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate H9           |
| `host/src/password.rs:24`                                 | `impl TryFrom<ProfferedPassword> for Password {`                                       | candidate H9           |
| `host/src/password.rs:27`                                 | `fn try_from(password: ProfferedPassword) -> Result<Self, Self::Error> {`              | candidate H9           |
| `host/src/stored_password_hash.rs:25`                     | `impl FromStr for StoredPasswordHash {`                                                | candidate H11          |
| `host/src/stored_password_hash.rs:28`                     | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate H11          |
| `macros/src/id_newtype.rs:67`                             | `fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {`                    | named exclusion E-NODV |
| `macros/src/num_newtype.rs:184`                           | `fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {`                    | named exclusion E-NODV |
| `macros/src/num_newtype.rs:213`                           | `fn try_from(v: #inner) -> ::core::result::Result<Self, Self::Error> {`                | named exclusion E-NODV |
| `macros/src/server_fn.rs:94`                              | `fn parse(input: ParseStream<'_>) -> syn::Result<Self> {`                              | named exclusion E-NODV |
| `macros/src/str_newtype.rs:157`                           | `fn try_from(s: ::std::string::String) -> ::core::result::Result<Self, Self::Error> {` | named exclusion E-NODV |
| `macros/src/str_newtype.rs:277`                           | `fn try_from(s: ::std::string::String) -> ::core::result::Result<Self, Self::Error> {` | named exclusion E-NODV |
| `server/src/cli.rs:98`                                    | `impl FromStr for BootstrapDb {`                                                       | candidate B21          |
| `server/src/cli.rs:101`                                   | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate B21          |
| `server/src/cli.rs:158`                                   | `impl FromStr for AppTarget {`                                                         | candidate B21          |
| `server/src/cli.rs:161`                                   | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | candidate B21          |
| `server/src/cli.rs:450`                                   | `fn parse(args: &[&str]) -> Cli {`                                                     | named exclusion E-NODV |
| `server/src/soft_path.rs:27`                              | `pub fn parse(s: &str) -> Self {`                                                      | named exclusion E-NODV |
| `storage/src/backup/catalog.rs:59`                        | `impl FromStr for CatalogNullability {`                                                | named exclusion E-NODV |
| `storage/src/backup/catalog.rs:62`                        | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | named exclusion E-NODV |
| `storage/src/backup/catalog.rs:115`                       | `impl FromStr for BackupRowJson {`                                                     | named exclusion E-NODV |
| `storage/src/backup/catalog.rs:118`                       | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | named exclusion E-NODV |
| `storage/src/backup/restore_validation.rs:167`            | `fn validate(&self, report: &mut RestoreValidationReport);`                            | named exclusion E-NODV |
| `storage/src/backup/restore_validation.rs:241`            | `fn validate(&self, report: &mut RestoreValidationReport) {`                           | named exclusion E-NODV |
| `storage/src/backup/restore_validation.rs:277`            | `fn validate(&self, report: &mut RestoreValidationReport) {`                           | named exclusion E-NODV |
| `storage/src/backup/restore_validation.rs:397`            | `fn validate(&self, report: &mut RestoreValidationReport) {`                           | named exclusion E-NODV |
| `storage/src/backup/restore_validation.rs:427`            | `fn validate(&self, report: &mut RestoreValidationReport) {`                           | named exclusion E-NODV |
| `storage/src/db.rs:178`                                   | `impl FromStr for DbConnectOptions {`                                                  | named exclusion E-NODV |
| `storage/src/db.rs:181`                                   | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `storage/src/helpers.rs:249`                              | `impl FromStr for SerializedPostTags {`                                                | candidate H25          |
| `storage/src/helpers.rs:252`                              | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | candidate H25          |
| `storage/src/instance_identity.rs:32`                     | `impl FromStr for InstanceId {`                                                        | candidate H13          |
| `storage/src/instance_identity.rs:35`                     | `fn from_str(value: &str) -> Result<Self, Self::Err> {`                                | candidate H13          |
| `xtask/src/census/orchestrate.rs:178`                     | `fn validate(mut report: CellReport) -> CellReport {`                                  | named exclusion E-NODV |
| `xtask/src/steps/duration_budget.rs:38`                   | `fn validate(&self, source: &str) -> Result<(), String> {`                             | named exclusion E-NODV |
| `xtask/src/steps/duration_budget.rs:407`                  | `fn validate(report: &str, manifest: &str) -> Result<String, String> {`                | named exclusion E-NODV |
| `xtask/src/steps/error_swallowing_inventory_check.rs:169` | `fn validate(markdown: &str, expected_baseline_count: usize) -> Vec<String> {`         | named exclusion E-NODV |
| `xtask/src/steps/server_fn_tracing_check.rs:87`           | `fn parse(input: ParseStream<'_>) -> syn::Result<Self> {`                              | named exclusion E-NODV |
| `xtask/src/steps/server_fn_wire_arg_error_check.rs:1100`  | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `xtask/src/steps/server_fn_wire_arg_error_check.rs:1122`  | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `xtask/src/steps/server_fn_wire_arg_error_check.rs:1143`  | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `xtask/src/steps/server_fn_wire_arg_error_check.rs:1162`  | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `xtask/src/steps/server_fn_wire_arg_error_check.rs:1183`  | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `xtask/src/steps/server_fn_wire_arg_error_check.rs:1207`  | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `xtask/src/steps/server_fn_wire_arg_error_check.rs:1239`  | `fn from_str(s: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `xtask/src/steps/server_fn_wire_arg_error_check.rs:1291`  | `fn from_str(_: &str) -> Result<Self, Self::Err> {`                                    | named exclusion E-NODV |
| `xtask/src/steps/server_fn_wire_arg_error_check.rs:1308`  | `fn from_str(_: &str) -> Result<Self, Self::Err> { todo!() }`                          | named exclusion E-NODV |
| `xtask/src/steps/sqlx_newtype_bind_check.rs:68`           | `fn parse(label: &str) -> Option<Self> {`                                              | named exclusion E-NODV |
