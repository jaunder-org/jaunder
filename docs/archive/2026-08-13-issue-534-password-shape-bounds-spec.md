# Issue #534: Password shape bounds

## Problem

`common::password::validate_password_shape` currently enforces only
`s.len() >= 8`. `str::len` counts UTF-8 bytes, so a password containing fewer
than eight Unicode scalar values can pass when its encoded representation
occupies eight or more bytes. The validator also has no maximum, allowing
arbitrarily long submitted passwords to proceed through wire decoding and toward
password hashing.

`Password` and `ProfferedPassword` both delegate to this validator.
`ProfferedPassword` is also the validated client-to-server wire type, so the
shared rule is the correct single enforcement point for domain construction,
client-side field validation, serde decoding, registration, login, and
password-reset confirmation.

## Decisions

1. Password length is measured in Unicode scalar values with
   `str::chars().count()`. The rule deliberately does not claim to count
   grapheme clusters or user-perceived characters.
2. Valid plaintext passwords contain between 8 and 512 Unicode scalar values,
   inclusive.
3. Validation performs no Unicode normalization. Password identity and Argon2
   input remain the exact submitted UTF-8 bytes. Canonically equivalent
   spellings may remain distinct credentials and may have different scalar
   counts.
4. The 8–512 rule applies uniformly everywhere the shared types are constructed,
   including login. There is no legacy-password bypass, endpoint-specific
   validator, compatibility shim, or migration flow.
5. Too-short and too-long failures are distinct typed `PasswordError` variants.
   Their messages state the applicable bound in characters and never include
   submitted password material.
6. The existing shared-validator architecture remains. Both `Password::from_str`
   and `ProfferedPassword::from_str` continue to delegate to one private
   validator. Serde inherits the rule through `ProfferedPassword::from_str`;
   client `Field<Password>` validation inherits it through `Password::from_str`.
   Neither path duplicates the rule.

## Acceptance criteria

1. A password with exactly 8 Unicode scalar values is accepted, including when
   one or more values use multiple UTF-8 bytes.
2. A password with 7 Unicode scalar values is rejected even when its UTF-8
   representation is at least 8 bytes.
3. A password with exactly 512 Unicode scalar values is accepted.
4. A password with 513 Unicode scalar values is rejected as too long.
5. `Password` and `ProfferedPassword` produce the same accept/reject result at
   all four boundaries above.
6. Serde decoding of `ProfferedPassword` rejects the same under-minimum and
   over-maximum inputs, proving the wire boundary inherits the shared rule.
7. Client-side `Field<Password>` validated-input behavior remains derived from
   `Password::from_str`; no separate client-only length rule is introduced.
8. Error formatting identifies the minimum or maximum bound without
   interpolating or echoing submitted password material. The typed error
   variants carry no submitted password value.
9. Accepted passwords retain their exact submitted bytes. Seven decomposed
   `e`-plus-combining-acute pairs (14 submitted scalar values, 7 after NFC) are
   accepted and retained byte-for-byte, proving validation does not normalize
   before counting.
10. The applicable repository validation gate passes.

## Out of scope

- Grapheme-cluster counting.
- Unicode normalization or confusable-character policy.
- Password-strength scoring, breached-password lookup, or composition
  requirements.
- Endpoint-specific compatibility for credentials that the new uniform rule
  rejects.
- Request-body limits outside the password value itself.
