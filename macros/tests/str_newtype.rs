//! Exercises the full positive surface `#[derive(StrNewtype)]` generates, against a
//! fixture newtype with a hand-written validating/normalizing `FromStr`. The derive
//! owns everything below except `FromStr` and the std `#[derive]`s (ADR-0063 §3).

use macros::StrNewtype;
use std::collections::HashSet;
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
struct Code(String);

/// `FromStr` is the one hand-written part: it normalizes (lowercase) and rejects the
/// empty string, so the derived serde/`TryFrom` paths inherit that validation.
#[derive(Debug, PartialEq)]
struct BadCode;

impl std::fmt::Display for BadCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("bad code")
    }
}

impl FromStr for Code {
    type Err = BadCode;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        if s.is_empty() {
            return Err(BadCode);
        }
        Ok(Code(s))
    }
}

#[test]
fn try_from_string_ok_and_err() {
    assert_eq!(Code::try_from("AB".to_owned()), Ok(Code("ab".to_owned())));
    assert_eq!(Code::try_from(String::new()), Err(BadCode));
}

#[test]
fn from_self_for_string() {
    assert_eq!(String::from(Code::from_str("ab").unwrap()), "ab".to_owned());
}

#[test]
fn as_ref_str() {
    let c = Code::from_str("ab").unwrap();
    let r: &str = c.as_ref();
    assert_eq!(r, "ab");
}

fn take_str(_: &str) {}

#[test]
fn deref_and_coercion() {
    let c = Code::from_str("ab").unwrap();
    assert_eq!(c.len(), 2); // a `str` method reached through `Deref`
    take_str(&c); // `&Code` coerces to `&str`
}

#[test]
fn borrow_probes_hashset_with_str() {
    let mut set: HashSet<Code> = HashSet::new();
    set.insert(Code::from_str("ab").unwrap());
    // A `&str` key with no allocation — needs `Borrow<str>` + coherent `Hash`.
    assert!(set.contains("ab"));
}

#[test]
fn display() {
    assert_eq!(format!("{}", Code::from_str("ab").unwrap()), "ab");
}

#[test]
fn partial_eq_str_and_ref_str() {
    let c = Code::from_str("ab").unwrap();
    assert!(c == "ab"); // PartialEq<&str>
    let s: &str = "ab";
    assert!(c == *s); // PartialEq<str>
}

#[test]
fn serde_roundtrip_and_wire_validation() {
    let c = Code::from_str("ab").unwrap();
    assert_eq!(serde_json::to_string(&c).unwrap(), "\"ab\"");
    assert_eq!(
        serde_json::from_str::<Code>("\"AB\"").unwrap(),
        Code("ab".to_owned())
    );
    // Invalid input is rejected on the wire because deserialize routes through FromStr.
    assert!(serde_json::from_str::<Code>("\"\"").is_err());
}

// --- secret variant -------------------------------------------------------------------
// `#[str_newtype(secret)]` emits only redacting `Debug`, `AsRef<str>`, and
// `TryFrom<String>`. The *absence* of Display/serde/Deref/owned-String/PartialEq is
// locked by the `compile_fail` doctests on the derive (it can't be asserted at runtime).

#[derive(Clone, StrNewtype)] // NOTE: no `Debug` derive — the macro generates a redacting one.
#[str_newtype(secret)]
struct Secret(String);

#[derive(Debug, PartialEq)]
struct BadSecret;

impl std::fmt::Display for BadSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("bad")
    }
}

impl FromStr for Secret {
    type Err = BadSecret;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(BadSecret);
        }
        Ok(Secret(s.to_owned()))
    }
}

#[test]
fn secret_debug_redacts() {
    let s = Secret::from_str("hunter2").unwrap();
    let d = format!("{s:?}");
    assert_eq!(d, "Secret([redacted])");
    assert!(!d.contains("hunter2"));
}

#[test]
fn secret_as_ref_and_try_from() {
    let s = Secret::try_from("hunter2".to_owned()).unwrap();
    let bytes: &str = s.as_ref();
    assert_eq!(bytes, "hunter2");
    assert!(Secret::try_from(String::new()).is_err());
}
