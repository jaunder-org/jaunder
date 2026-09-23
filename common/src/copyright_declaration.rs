//! Projection-only rights metadata for a public Post.
//!
//! The Post's immutable UTC creation year combines with the author's current
//! Display Name and Content License; it is never a stored Post snapshot.

use crate::{
    content_license::ContentLicense, display_name::DisplayName, time::UtcInstant,
    username::Username,
};

/// The current author identity selected for public rights presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyrightAuthor {
    DisplayName(DisplayName),
    Username(Username),
}

impl CopyrightAuthor {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::DisplayName(name) => name.as_ref(),
            Self::Username(username) => username.as_ref(),
        }
    }
}

/// The current public Copyright Declaration of a Post.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyrightDeclaration {
    year: i16,
    author: CopyrightAuthor,
    license: ContentLicense,
}

impl CopyrightDeclaration {
    /// Resolves the author's current presentation at the public projection boundary.
    #[must_use]
    pub fn for_post(
        created_at: UtcInstant,
        display_name: Option<&DisplayName>,
        username: &Username,
        license: ContentLicense,
    ) -> Self {
        let year = jiff::tz::Offset::UTC.to_datetime(created_at.value()).year();
        let author = display_name.map_or_else(
            || CopyrightAuthor::Username(username.clone()),
            |name| CopyrightAuthor::DisplayName(name.clone()),
        );
        Self::from_resolved(year, author, license)
    }

    /// Accepts already resolved values from a Syndication Feed fixture or projection.
    #[must_use]
    pub fn from_resolved(year: i16, author: CopyrightAuthor, license: ContentLicense) -> Self {
        Self {
            year,
            author,
            license,
        }
    }

    #[must_use]
    pub fn year(&self) -> i16 {
        self.year
    }

    #[must_use]
    pub fn author_name(&self) -> &str {
        self.author.as_str()
    }

    #[must_use]
    pub fn license(&self) -> ContentLicense {
        self.license
    }

    /// Atom/RSS plain rights text; the web renderer escapes its parts separately.
    #[must_use]
    pub fn text(&self) -> String {
        format!("{} · {}", self.copyright(), self.license.label())
    }

    /// JSON Feed's copyright field deliberately excludes the rights label.
    #[must_use]
    pub fn copyright(&self) -> String {
        format!("© {} {}", self.year, self.author_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{parse_display_name, parse_username, parse_utc_instant};

    #[test]
    fn declaration_uses_utc_creation_year_and_current_author_rights() {
        let created_at = parse_utc_instant("2024-12-31T23:59:59Z");
        let username = parse_username("ada");
        let display_name = parse_display_name("Ada <&>");
        let declaration = CopyrightDeclaration::for_post(
            created_at,
            Some(&display_name),
            &username,
            ContentLicense::CcBy4_0,
        );
        assert_eq!(declaration.text(), "© 2024 Ada <&> · CC BY 4.0");
        assert_eq!(declaration.copyright(), "© 2024 Ada <&>");
        assert_eq!(declaration.year(), 2024);
        assert_eq!(declaration.author_name(), "Ada <&>");
        assert_eq!(declaration.license(), ContentLicense::CcBy4_0);

        let fallback = CopyrightDeclaration::for_post(
            created_at,
            None,
            &username,
            ContentLicense::AllRightsReserved,
        );
        assert_eq!(fallback.text(), "© 2024 ada · All Rights Reserved");
        assert_eq!(fallback.copyright(), "© 2024 ada");
    }
}
