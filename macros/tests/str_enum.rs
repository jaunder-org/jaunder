//! Exercises the full positive surface `#[derive(StrEnum)]` generates against real derived
//! enums: `as_str`/`Display`/`FromStr`/`TryFrom`, the serde bridge under `#[str_enum(serde)]`,
//! the generated `Invalid<Name>` error (auto and `error =`-overridden message), per-variant
//! `rename`, and std `Default` coexistence. (Codegen *branch* coverage lives in the
//! in-crate `macros/src/lib.rs` unit tests — integration enums expand at compile time.)

use macros::StrEnum;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, StrEnum)]
#[str_enum(serde)]
enum Fmt {
    #[default]
    Markdown,
    Org,
    Html,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, StrEnum)] // no serde, no Default
enum Kind {
    Public,
    Subscribers,
    Named,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, StrEnum)]
#[str_enum(error = "post format must be \"markdown\", \"org\", or \"html\"")]
enum WithMsg {
    Alpha,
    #[str_enum(rename = "ZED")]
    Zed,
}

// A multi-word variant: the default token is the `snake_case` of the identifier, not the
// concatenated lowercase (`InviteOnly` -> `invite_only`, not `inviteonly`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, StrEnum)]
enum Policy {
    Open,
    InviteOnly,
    Closed,
}

#[test]
fn as_str_and_display_use_lowercase_tokens() {
    assert_eq!(Fmt::Markdown.as_str(), "markdown");
    assert_eq!(Fmt::Org.as_str(), "org");
    assert_eq!(Fmt::Html.as_str(), "html");
    assert_eq!(Kind::Subscribers.as_str(), "subscribers");
    assert_eq!(Fmt::Org.to_string(), "org"); // Display == as_str
}

#[test]
fn from_str_and_try_from_round_trip_every_variant() {
    for f in [Fmt::Markdown, Fmt::Org, Fmt::Html] {
        assert_eq!(f.as_str().parse::<Fmt>(), Ok(f));
        assert_eq!(Fmt::try_from(f.as_str()), Ok(f));
    }
    for k in [Kind::Public, Kind::Subscribers, Kind::Named] {
        assert_eq!(k.as_str().parse::<Kind>(), Ok(k));
    }
}

#[test]
fn unknown_token_is_rejected_with_auto_message() {
    let err = "xml".parse::<Fmt>().unwrap_err();
    assert_eq!(err.to_string(), "must be one of: markdown, org, html");
    // The generated error is a named, usable type (`Invalid{Name}`), PartialEq + Debug.
    assert_eq!(err, InvalidFmt);
    assert!(Fmt::try_from("nope").is_err());
}

#[test]
fn error_message_override_and_rename() {
    assert_eq!("ZED".parse::<WithMsg>(), Ok(WithMsg::Zed)); // renamed token
    assert_eq!("alpha".parse::<WithMsg>(), Ok(WithMsg::Alpha)); // lowercase default
    assert!("zed".parse::<WithMsg>().is_err()); // the identifier is NOT the token
    assert_eq!(
        "zed".parse::<WithMsg>().unwrap_err().to_string(),
        "post format must be \"markdown\", \"org\", or \"html\"",
    );
}

#[test]
fn serde_round_trips_the_token_and_rejects_unknown() {
    for f in [Fmt::Markdown, Fmt::Org, Fmt::Html] {
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, format!("\"{}\"", f.as_str()));
        assert_eq!(serde_json::from_str::<Fmt>(&json).unwrap(), f);
    }
    assert!(serde_json::from_str::<Fmt>("\"nope\"").is_err());
}

#[test]
fn std_default_is_the_marked_variant() {
    assert_eq!(Fmt::default(), Fmt::Markdown);
}

#[test]
fn multiword_variants_snake_case_their_tokens() {
    assert_eq!(Policy::Open.as_str(), "open");
    assert_eq!(Policy::InviteOnly.as_str(), "invite_only");
    assert_eq!("invite_only".parse::<Policy>(), Ok(Policy::InviteOnly));
    assert!("inviteonly".parse::<Policy>().is_err()); // not the concatenated form
}
