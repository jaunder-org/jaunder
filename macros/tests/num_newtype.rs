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

#[test]
fn ordering_agrees_with_the_inner_integer() {
    let a = Count::from_str("3").unwrap();
    let b = Count::from_str("7").unwrap();
    assert!(a < b);
    assert!(b > a);
    assert_eq!(a.cmp(&b), 3u32.cmp(&7));
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
