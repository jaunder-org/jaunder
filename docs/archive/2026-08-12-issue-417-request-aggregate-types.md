# Issue #417: Request-Aggregate Server-Function Inputs

## Purpose

Replace cohesive multi-parameter Leptos server-function submissions with typed
request aggregates assembled from already parsed client fields. This removes
same-primitive transposition hazards and avoids re-harvesting validated browser
state as strings before dispatch.

## Decisions

1. A server function whose caller supplies multiple values forming one cohesive
   request accepts exactly one operation- or domain-shaped aggregate. Ambient
   inputs such as authentication, cookies, headers, and injected services are
   not aggregate fields.
2. Aggregate names describe their meaning. Operations with different meanings
   use operation-specific `*Request` types; operations sharing the same fields
   and meaning may share one domain-shaped type.
3. Wasm forms construct generated server-action inputs manually from
   `Field::parsed()` values and dispatch through `ServerAction`. They retain a
   native `<form>` submit event, Enter-key submission, default prevention,
   pending disablement, validation, and existing outcome rendering.
4. Secret inputs are parsed directly into their inbound wire types. Login and
   registration therefore use `Field<ProfferedPassword>`, not a client-side
   `Password` followed by a submit-time conversion.
5. Single-value commands and functions whose parameters are genuinely
   independent remain direct-parameter server functions. No syntactic xtask gate
   attempts to decide semantic cohesion.
6. The convention and its trade-offs are recorded in
   `docs/adr/0129-request-aggregate-server-function-inputs.md` and projected
   into `docs/ARCHITECTURE.md`. It does not alter Jaunder's ubiquitous domain
   language, so `CONTEXT.md` is unchanged.

## Scope

Migrate every current cohesive multi-field `ActionForm` submission. Each type
lives beside its server function in the named vertical's `api.rs`, derives the
wire traits required by the generated input, and reuses the existing domain
newtypes required by ADR-0063:

| Operation(s)                         | Aggregate                     | Exact fields                                                                                    |
| ------------------------------------ | ----------------------------- | ----------------------------------------------------------------------------------------------- |
| `auth::login`                        | `LoginRequest`                | `username: Username`, `password: ProfferedPassword`, `label: Option<SessionLabel>`              |
| `registration::register`             | `RegistrationRequest`         | `username: Username`, `password: ProfferedPassword`, `invite_code: Option<ProfferedInviteCode>` |
| `invites::create`                    | `CreateInviteRequest`         | `expires_in_hours: Option<InviteTtlHours>`, `recipient_email: Email`                            |
| `password_reset::confirm`            | `ConfirmPasswordResetRequest` | `token: RawToken`, `new_password: ProfferedPassword`                                            |
| `audiences::rename`                  | `RenameAudienceRequest`       | `audience_id: AudienceId`, `name: AudienceName`                                                 |
| `audiences::{add,remove}_subscriber` | `AudienceMembershipRequest`   | `audience_id: AudienceId`, `subscription_id: SubscriptionId`                                    |
| `media::delete`                      | `DeleteMediaRequest`          | `sha256: ContentHash`, `filename: Filename`, `source: MediaSource`, `force: Option<bool>`       |

No field may be flattened to a primitive merely to ease serialization. Any
exception to an existing domain newtype requires the explicit approval ADR-0063
already demands.

Login is the tracer bullet. No later migration begins until its aggregate wire
shape and UI behavior have passed the proof gate below.

Out of scope:

- migrating single-value forms merely for consistency;
- changing endpoint paths, response types, authorization, storage behavior, or
  user-visible workflows;
- a repository-wide static rule banning multi-argument server functions;
- progressive enhancement, already outside the pure-CSR architecture.

## Acceptance criteria

### Tracer-bullet gate

1. Login accepts one `LoginRequest` wire parameter containing every
   caller-supplied login value; the browser supplies `label: None`, while Rust
   callers can supply a label.
2. The login component parses username and `ProfferedPassword`, constructs the
   generated action input only when both fields are valid, and manually
   dispatches it from a native form submit handler.
3. Backend-parametric integration tests post canonical nested keys
   `request[username]`, `request[password]`, and `request[label]` to the
   existing login endpoint. Distinct sentinel values prove exact mapping into
   the authentication/session calls. Separate invalid-username and
   invalid-`ProfferedPassword` payloads return Leptos's decode-boundary HTTP 500
   with a `server_function` error, and unchanged storage state proves the
   handler did not authenticate or create a session.
4. Targeted login Playwright coverage proves successful login and error
   rendering. Empty or invalid username/password submission prevents the native
   default, dispatches zero requests, and exposes the existing touched-gated
   field errors. A controlled delayed response proves controls are disabled
   while pending and that two submit attempts—including Enter while pending—
   produce exactly one network request.
5. `cargo xtask check` passes after the login tracer bullet. Only then may the
   remaining request shapes be migrated.

### Complete migration

6. Each in-scope operation accepts one cohesive aggregate, and no aggregate
   contains ambient server context.
7. Add/remove audience membership share one request type; all other in-scope
   requests are operation-specific unless the implementation demonstrates the
   same semantic identity.
8. Every migrated UI dispatches parsed typed values manually and preserves the
   operation-specific observable behavior below. “Pending” means the submit
   control is disabled from dispatch until resolution; a second click or Enter
   attempt produces no second request.

   | UI path                     | Required preserved observations                                                                          |
   | --------------------------- | -------------------------------------------------------------------------------------------------------- |
   | Login                       | field errors; Enter submit; pending; error/success rendering; redirect and session-marker update         |
   | Registration                | field errors; invite-only hidden code; pending; error rendering; redirect/session establishment          |
   | Invite creation             | email/expiry validation; pending; success/error outcome; successful list revalidation                    |
   | Password-reset confirmation | query token mapping; password validation; pending; error rendering; success redirect                     |
   | Audience rename             | name validation; pending; error rendering; successful audience-list revalidation                         |
   | Audience add/remove         | exact audience/subscription mapping; pending; error rendering; successful membership-only revalidation   |
   | Media ordinary delete       | complete identity mapping; confirmation; pending; refusal/success outcome and list revalidation          |
   | Media force delete          | same identity plus `force: Some(true)`; force confirmation; pending; success/error and list revalidation |

9. Existing backend-parametric endpoint tests are adapted to the aggregate
   input. Required fields use distinct sentinels so a transpose fails. Every
   optional field is exercised as both `Some` and `None`, and storage/handler
   observations assert exact values. Ordinary and forced media deletion are
   separate cases. The generic nested-transport rejection proof is not
   duplicated mechanically after login.
10. Existing relevant Playwright scenarios pass, with focused additions where no
    test currently observes a required preserved behavior.
11. The inspected `ActionForm` operations outside this migration remain direct:
    audience create/delete, email verification request, password-reset request,
    post publish/delete, and subscription subscribe/unsubscribe each submit one
    domain value. They do not gain wrapper requests merely for consistency.
12. The final `cargo xtask validate` passes across static checks, coverage, and
    all SQLite/PostgreSQL × Chromium/Firefox end-to-end combinations.

## Delivery sequence

The login tracer bullet lands as its own focused commit. Its commit message or
PR evidence records the backend integration command, targeted Playwright
command, and successful `cargo xtask check` run. Remaining migrations begin in
later commits only after that evidence is green, making the sequence reviewable
from history rather than inferred from the final tree.
