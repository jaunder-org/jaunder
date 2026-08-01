//! Exercises the full positive surface `#[derive(StrNewtype)]` generates, against a
//! fixture newtype with a hand-written validating/normalizing `FromStr`. The derive
//! owns everything below except `FromStr` and the std `#[derive]`s (ADR-0063 §3).

use macros::StrNewtype;
use std::collections::{BTreeSet, HashSet};
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

#[test]
fn ordering_agrees_with_the_inner_str() {
    let a = Code::from_str("aaa").unwrap();
    let b = Code::from_str("bbb").unwrap();
    // `<` is the discriminator. `a.cmp(&b)` is not: it resolves through `Deref<str>`
    // even without an `Ord` on the newtype, so it would pass before the derive emitted
    // anything. The operator does not auto-deref.
    assert!(a < b);
    assert!(b > a);
    assert_eq!(a.cmp(&b), "aaa".cmp("bbb"));
}

#[test]
fn sorts_and_keys_a_btreeset() {
    let mut v = vec![
        Code::from_str("c").unwrap(),
        Code::from_str("a").unwrap(),
        Code::from_str("b").unwrap(),
    ];
    v.sort();
    assert_eq!(v[0], Code::from_str("a").unwrap());
    assert_eq!(v[2], Code::from_str("c").unwrap());

    let set: BTreeSet<Code> = v.into_iter().collect();
    assert_eq!(set.len(), 3);
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

// --- secret + serde variant -----------------------------------------------------------
// `#[str_newtype(secret, serde)]` is the secret surface plus the validating serde bridge:
// redacting `Debug` and `AsRef`, and (de)serialization, but still no Display/Deref/etc.
// It is for a secret that must cross the wire *inbound*.

#[derive(Clone, StrNewtype)] // no `Debug` derive — the macro generates a redacting one.
#[str_newtype(secret, serde)]
struct SecretWire(String);

impl FromStr for SecretWire {
    type Err = BadSecret;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(BadSecret);
        }
        Ok(SecretWire(s.to_owned()))
    }
}

#[test]
fn secret_serde_roundtrips_and_validates_on_the_wire() {
    let s = SecretWire::from_str("hunter2").unwrap();
    assert_eq!(serde_json::to_string(&s).unwrap(), "\"hunter2\"");
    let back: SecretWire = serde_json::from_str("\"hunter2\"").unwrap();
    assert_eq!(back.as_ref(), "hunter2");
    // Deserialize routes through FromStr, so invalid input is rejected on the wire.
    assert!(serde_json::from_str::<SecretWire>("\"\"").is_err());
}

#[test]
fn secret_serde_debug_still_redacts() {
    let s = SecretWire::from_str("hunter2").unwrap();
    let d = format!("{s:?}");
    assert_eq!(d, "SecretWire([redacted])");
    assert!(!d.contains("hunter2"));
}

// --- infallible variant ---------------------------------------------------------------
// `#[str_newtype(infallible)]` construction never rejects, so `From<String>` replaces
// `FromStr`. Ordering is emitted here exactly as for the default trailer (#761).

#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
#[str_newtype(infallible)]
struct Label(String);

impl From<String> for Label {
    fn from(s: String) -> Self {
        Label(s)
    }
}

#[test]
fn infallible_trailer_orders() {
    let a = Label::from("aaa");
    let b = Label::from("bbb");
    assert!(a < b);

    let mut v = vec![b.clone(), a.clone()];
    v.sort();
    assert_eq!(v[0], a);

    let set: BTreeSet<Label> = v.into_iter().collect();
    assert_eq!(set.len(), 2);
}

// --- no_ord opt-out -------------------------------------------------------------------
// `#[str_newtype(no_ord)]` suppresses only the ordering half, for a type that
// deliberately derives no `PartialEq`/`Eq` (`RawToken`, the bearer-token profile).
// The *absence* of ordering is locked by a `compile_fail` doctest on the derive; what
// this fixture proves is that nothing else in the trailer went with it.

#[derive(Clone, Debug, StrNewtype)]
#[str_newtype(no_ord)]
struct Unordered(String);

// A rejecting `FromStr` (not `Infallible`): the derive's `TryFrom<String>` routes through
// it, and an infallible error type would make that impl trip `clippy::infallible_try_from`.
impl FromStr for Unordered {
    type Err = BadCode;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(BadCode);
        }
        Ok(Unordered(s.to_owned()))
    }
}

#[test]
fn no_ord_keeps_the_rest_of_the_trailer() {
    let u = Unordered::from_str("x").unwrap();
    assert_eq!(u.to_string(), "x"); // Display
    let read: &str = &u; // Deref
    assert_eq!(read, "x");
    assert!(u == "x"); // PartialEq<str>
    assert_eq!(serde_json::to_string(&u).unwrap(), "\"x\"");
}
