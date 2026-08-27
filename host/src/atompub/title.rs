use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

use common::username::Username;

/// Human-readable title of an `AtomPub` Workspace.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct WorkspaceTitle(String);

/// Error returned for an empty [`WorkspaceTitle`].
#[derive(Debug, Error)]
#[error("workspace title cannot be empty")]
pub struct InvalidWorkspaceTitle;

impl FromStr for WorkspaceTitle {
    type Err = InvalidWorkspaceTitle;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err(InvalidWorkspaceTitle);
        }
        Ok(Self(value.to_owned()))
    }
}

impl WorkspaceTitle {
    /// Applies Jaunder's current Workspace-title policy for a user.
    #[must_use]
    pub fn for_user(username: &Username) -> Self {
        Self(username.to_string())
    }
}

/// Human-readable title of a Collection in an `AtomPub` Service Document.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct CollectionTitle(String);

/// Error returned for an empty [`CollectionTitle`].
#[derive(Debug, Error)]
#[error("collection title cannot be empty")]
pub struct InvalidCollectionTitle;

impl FromStr for CollectionTitle {
    type Err = InvalidCollectionTitle;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err(InvalidCollectionTitle);
        }
        Ok(Self(value.to_owned()))
    }
}

impl CollectionTitle {
    /// Title of the Posts Collection advertised by the Service Document.
    #[must_use]
    pub fn posts() -> Self {
        Self("Posts".to_owned())
    }

    /// Title of the Media Collection advertised by the Service Document.
    #[must_use]
    pub fn media() -> Self {
        Self("Media".to_owned())
    }
}

/// Human-readable title of an `AtomPub` Collection feed.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct CollectionFeedTitle(String);

/// Error returned for an empty [`CollectionFeedTitle`].
#[derive(Debug, Error)]
#[error("collection feed title cannot be empty")]
pub struct InvalidCollectionFeedTitle;

impl FromStr for CollectionFeedTitle {
    type Err = InvalidCollectionFeedTitle;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err(InvalidCollectionFeedTitle);
        }
        Ok(Self(value.to_owned()))
    }
}

impl CollectionFeedTitle {
    /// Title of a user's Posts Collection feed.
    #[must_use]
    pub fn posts(username: &Username) -> Self {
        Self(format!("{username}'s posts"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atompub_titles_parse_trim_and_reject_blank() {
        assert_eq!(
            "  Workspace  ".parse::<WorkspaceTitle>().unwrap(),
            "Workspace"
        );
        assert_eq!("  Posts  ".parse::<CollectionTitle>().unwrap(), "Posts");
        assert_eq!(
            "  Alice's posts  ".parse::<CollectionFeedTitle>().unwrap(),
            "Alice's posts"
        );
        assert!("".parse::<WorkspaceTitle>().is_err());
        assert!(" ".parse::<WorkspaceTitle>().is_err());
        assert!("".parse::<CollectionTitle>().is_err());
        assert!("\n".parse::<CollectionTitle>().is_err());
        assert!("".parse::<CollectionFeedTitle>().is_err());
        assert!(" \t ".parse::<CollectionFeedTitle>().is_err());
    }

    #[test]
    fn atompub_title_constructors_preserve_current_policy() {
        let username = "alice".parse::<Username>().unwrap();
        assert_eq!(WorkspaceTitle::for_user(&username), "alice");
        assert_eq!(CollectionTitle::posts(), "Posts");
        assert_eq!(CollectionTitle::media(), "Media");
        assert_eq!(CollectionFeedTitle::posts(&username), "alice's posts");
    }
}
