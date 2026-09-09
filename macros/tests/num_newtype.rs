//! Exercises the ordering half of the `#[derive(NumNewtype)]` trailer (#761). The rest of
//! the numeric-value surface is covered by the doctest on the derive and by the real types
//! in `common`.

use macros::NumNewtype;
use std::collections::BTreeSet;
use std::str::FromStr;

// Every option this fixture declares is exercised below. Adding an unused `default =` or
// `max =` would emit a `Default` impl / bound assertion that nothing calls — a
// self-inflicted uncovered region, in the change whose coverage attribution is the point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, NumNewtype)]
#[num_newtype(inner = u32, min = 1)]
struct Count(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, NumNewtype)]
#[num_newtype(
    inner = u32,
    min = 1,
    max = 10,
    default = 4,
    error = "percentage must be between one and ten",
    clamp
)]
struct Percentage(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, NumNewtype)]
#[num_newtype(inner = i16)]
struct AnyInteger(i16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, NumNewtype)]
#[num_newtype(inner = u8, max = 10)]
struct AtMostTen(u8);

#[test]
fn ordering_agrees_with_the_inner_integer() {
    let a = Count::from_str("3").unwrap();
    let b = Count::from_str("7").unwrap();
    assert!(a < b);
    assert!(b > a);
    assert_eq!(a.cmp(&b), 3u32.cmp(&7));
}

#[test]
fn ranged_newtype_validates_every_construction_door() {
    assert_eq!(Percentage::from_str(" 10 ").unwrap().value(), 10);
    assert_eq!(Percentage::try_from(1).unwrap().value(), 1);
    assert!(Percentage::from_str("0").is_err());
    assert!(Percentage::from_str("11").is_err());
    assert!(Percentage::from_str("not a number").is_err());
    assert!(Percentage::try_from(0).is_err());
    assert!(Percentage::try_from(11).is_err());
}

#[test]
fn ranged_newtype_exposes_only_checked_values() {
    let value = Percentage::try_from(7).unwrap();
    assert_eq!(value.value(), 7);
    assert_eq!(u32::from(value), 7);
    assert_eq!(value.to_string(), "7");
    assert_eq!(Percentage::default().value(), 4);
}

#[test]
fn clamped_constructor_returns_the_nearest_valid_value() {
    const LOW: Percentage = Percentage::clamped(0);
    const MIDDLE: Percentage = Percentage::clamped(7);
    const HIGH: Percentage = Percentage::clamped(99);

    assert_eq!(Percentage::MIN, 1);
    assert_eq!(Percentage::MAX, 10);
    assert_eq!(LOW.value(), 1);
    assert_eq!(MIDDLE.value(), 7);
    assert_eq!(HIGH.value(), 10);
}

#[test]
fn serde_uses_the_integer_wire_shape_and_revalidates_it() {
    let encoded = serde_json::to_string(&Percentage::try_from(6).unwrap()).unwrap();
    assert_eq!(encoded, "6");
    assert_eq!(
        serde_json::from_str::<Percentage>(&encoded)
            .unwrap()
            .value(),
        6
    );
    assert!(serde_json::from_str::<Percentage>("0").is_err());
    assert!(serde_json::from_str::<Percentage>("11").is_err());
}

#[test]
fn generated_error_is_public_and_descriptive() {
    let error = Percentage::try_from(0).unwrap_err();
    assert_eq!(error.to_string(), "percentage must be between one and ten");
    let error: &dyn std::error::Error = &error;
    assert!(error.source().is_none());
}

#[test]
fn unbounded_and_max_only_forms_keep_their_declared_domains() {
    assert_eq!(AnyInteger::from_str("-12").unwrap().value(), -12);
    assert!(AnyInteger::from_str("not an integer").is_err());
    assert_eq!(AtMostTen::try_from(10).unwrap().value(), 10);
    assert!(AtMostTen::try_from(11).is_err());
    assert_eq!(
        AtMostTen::try_from(11).unwrap_err().to_string(),
        "AtMostTen must be an integer of at most 10"
    );
}

#[test]
fn sorts_and_keys_a_btreeset() {
    let mut v = vec![
        Count::from_str("9").unwrap(),
        Count::from_str("2").unwrap(),
        Count::from_str("5").unwrap(),
    ];
    v.sort();
    assert_eq!(v[0].value(), 2);
    assert_eq!(v[2].value(), 9);

    let set: BTreeSet<Count> = v.into_iter().collect();
    assert_eq!(set.len(), 3);
}

#[test]
fn min_bound_still_rejects() {
    // Exercises the `min` branch this fixture declares, so the option earns its keep.
    assert!(Count::from_str("0").is_err());
}
