use host::invite::InviteCode;

/// Parse `s` into a valid [`InviteCode`] for tests — the single place a test
/// invite-code literal is parsed. Lives here rather than `common::test_support`
/// because `InviteCode` is a `host` type (`common` cannot name it), and `storage`
/// depends on `host`, so this is reachable from every `storage` test module.
///
/// # Panics
///
/// Panics if `s` is not a validly-shaped invite code.
#[must_use]
pub fn parse_invite_code(s: &str) -> InviteCode {
    s.parse().expect("valid test invite code")
}
