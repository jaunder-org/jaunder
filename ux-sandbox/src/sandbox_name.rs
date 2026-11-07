//! A validated workspace component, retained across the filesystem boundary.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{borrow::Borrow, fmt, ops::Deref, str::FromStr};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SandboxName(String);

impl FromStr for SandboxName {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut bytes = value.bytes();
        if !bytes
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            || !bytes
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        {
            return Err(
                "NAME must be one lowercase ASCII/digit component with optional '-' or '_'".into(),
            );
        }
        Ok(Self(value.into()))
    }
}

impl Serialize for SandboxName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SandboxName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl TryFrom<String> for SandboxName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<SandboxName> for String {
    fn from(value: SandboxName) -> Self {
        value.0
    }
}

impl Borrow<str> for SandboxName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Deref for SandboxName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq<str> for SandboxName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for SandboxName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl AsRef<str> for SandboxName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SandboxName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_string_traits_preserve_value_semantics() {
        use std::collections::{BTreeSet, HashSet};

        let name = SandboxName::try_from(String::from("writing-flow_2")).unwrap();
        assert_eq!(name, name.clone());
        assert_eq!(name, "writing-flow_2");
        assert!(<SandboxName as PartialEq<str>>::eq(&name, "writing-flow_2"));
        let borrowed: &str = &name;
        assert_eq!(borrowed, "writing-flow_2");
        assert_eq!(name.len(), 14);
        let ordered = BTreeSet::from([name.clone(), "a".parse().unwrap()]);
        assert_eq!(ordered.first().unwrap(), "a");
        assert!(ordered.contains("writing-flow_2"));
        assert!("a".parse::<SandboxName>().unwrap() < name);
        let hashed = HashSet::from([name.clone()]);
        assert!(hashed.contains("writing-flow_2"));
        assert_eq!(String::from(name), "writing-flow_2");
        assert!(SandboxName::try_from(String::from("../writing")).is_err());
    }

    #[test]
    fn serde_round_trip_preserves_spelling_and_rejects_invalid_names() {
        let name: SandboxName = "writing-flow_2".parse().unwrap();
        let json = serde_json::to_string(&name).unwrap();
        assert_eq!(json, r#""writing-flow_2""#);
        assert_eq!(serde_json::from_str::<SandboxName>(&json).unwrap(), name);
        for value in ["", "../writing", "Writing", "writing/next", "a\0b"] {
            let error = serde_json::from_str::<SandboxName>(&serde_json::to_string(value).unwrap())
                .unwrap_err();
            assert!(error.to_string().contains(
                "NAME must be one lowercase ASCII/digit component with optional '-' or '_'"
            ));
        }
        for json in ["null", "42", "[]", "{}"] {
            assert!(serde_json::from_str::<SandboxName>(json).is_err());
        }
    }

    #[test]
    fn valid_components_preserve_exact_spelling() {
        for value in ["a", "0", "writing", "writing-flow_2", "9_-", "a_", "a-"] {
            let name: SandboxName = value.parse().unwrap();
            assert_eq!(name.as_ref(), value);
            assert_eq!(name.to_string(), value);
        }
    }

    #[test]
    fn unsafe_or_noncanonical_components_cannot_be_constructed() {
        for value in [
            "",
            ".",
            "..",
            "/tmp/writing",
            "../writing",
            "writing/next",
            "writing\\next",
            "Writing",
            "_writing",
            "-writing",
            "writing next",
            " writing",
            "writing ",
            "writíng",
            "a\0b",
            "a\nb",
            "C:\\writing",
        ] {
            assert!(value.parse::<SandboxName>().is_err(), "{value:?}");
        }
    }
}
