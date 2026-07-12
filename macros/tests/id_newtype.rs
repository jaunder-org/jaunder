//! Exercises the surface `#[derive(IdNewtype)]` generates for an `i64`-backed id newtype:
//! `From<i64>`/`From<Self> for i64`, `Display`, and a transparent-i64 serde bridge. `Copy`
//! and the other std traits are user-derived (ADR-0063 numeric-ID trailer).

use macros::IdNewtype;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IdNewtype)]
struct Id(i64);

#[test]
fn from_i64_and_into_i64() {
    let id = Id::from(42);
    assert_eq!(id, Id(42));
    let n: i64 = id.into();
    assert_eq!(n, 42);
}

#[test]
fn copy_semantics() {
    let a = Id(7);
    let b = a; // `Copy` — `a` remains usable below
    assert_eq!(a, b);
}

#[test]
fn display() {
    assert_eq!(format!("{}", Id(42)), "42");
}

#[test]
fn serde_transparent_roundtrip() {
    // Wire form is a bare integer, not a wrapper object.
    assert_eq!(serde_json::to_string(&Id(42)).unwrap(), "42");
    assert_eq!(serde_json::from_str::<Id>("42").unwrap(), Id(42));
}
