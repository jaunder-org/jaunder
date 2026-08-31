//! `#[text_enum]` against the real `strum`, which the crate's own unit tests cannot do —
//! they assert on rendered tokens, so nothing there proves the injected derives actually
//! resolve and produce a working token round-trip.

#[macros::text_enum(
    error = InvalidColour,
    message = "colour must be \"red\" or \"blue\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[strum(serialize_all = "snake_case")]
pub enum Colour {
    Red,
    Blue,
}

#[macros::text_enum(
    no_serde,
    error = InvalidWireColour,
    message = "wire colour must be \"red\" or \"blue\""
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[strum(serialize_all = "snake_case")]
pub enum WireColour {
    Red,
    Blue,
}

#[test]
fn token_round_trips_through_the_injected_derives() {
    assert_eq!(Colour::Red.as_ref(), "red");
    assert_eq!(Colour::Red.to_string(), "red");
    assert_eq!("blue".parse::<Colour>(), Ok(Colour::Blue));
    let s: &'static str = (&Colour::Blue).into();
    assert_eq!(s, "blue");
}

#[test]
fn parse_failure_carries_the_declared_message() {
    let err = "green".parse::<Colour>().unwrap_err();
    assert_eq!(err, InvalidColour);
    assert_eq!(err.to_string(), "colour must be \"red\" or \"blue\"");
}

#[test]
fn serde_round_trips_the_token_and_reports_the_declared_message() {
    assert_eq!(serde_json::to_string(&Colour::Red).unwrap(), "\"red\"");
    assert_eq!(
        serde_json::from_str::<Colour>("\"blue\"").unwrap(),
        Colour::Blue
    );
    let err = serde_json::from_str::<Colour>("\"green\"").unwrap_err();
    assert!(err.to_string().contains("colour must be"));
}

#[test]
fn no_serde_preserves_an_adopters_ordinary_serde_representation() {
    assert_eq!(WireColour::Red.to_string(), "red");
    assert_eq!("blue".parse::<WireColour>(), Ok(WireColour::Blue));
    assert_eq!(serde_json::to_string(&WireColour::Red).unwrap(), "\"Red\"");
    assert_eq!(
        serde_json::from_str::<WireColour>("\"Blue\"").unwrap(),
        WireColour::Blue
    );
}
