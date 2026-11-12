//! Pure helpers for sequence-numbered identifier files (ADRs, migrations):
//! parsing the leading number, detecting duplicate prefixes, checking backend
//! parity. No I/O — unit-tested in isolation.

/// The leading integer of a filename like `0034-foo.md` or `0023_create_x.sql`.
/// `None` when the name does not start with a digit.
pub fn leading_number(filename: &str) -> Option<u32> {
    let digits: String = filename.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}
