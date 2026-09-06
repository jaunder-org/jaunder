//! The closed set of public presentation themes.

/// A built-in public presentation theme.
///
/// `serialize_all = "snake_case"` supplies the durable configuration and wire tokens:
/// `"terminal"`, `"studio"`, and `"reader"`. The default keeps existing public
/// presentation unchanged until an operator chooses otherwise.
#[macros::text_enum(
    sqlx,
    error = InvalidTheme,
    message = "theme must be \"terminal\", \"studio\", or \"reader\""
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[strum(serialize_all = "snake_case")]
pub enum Theme {
    Terminal,
    #[default]
    Studio,
    Reader,
}

use std::str::FromStr;

use thiserror::Error;

/// A full SHA-256 digest used to address immutable public Theme content.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeContentDigest(String);

/// The input was not the canonical lowercase hexadecimal representation of a
/// SHA-256 digest.
#[derive(Debug, Error)]
#[error("theme content digest must be 64 lowercase hex characters")]
pub struct InvalidThemeContentDigest;

impl FromStr for ThemeContentDigest {
    type Err = InvalidThemeContentDigest;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if crate::media::is_valid_content_hash(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidThemeContentDigest)
        }
    }
}

/// Canonical digest of validated editable Theme Package source bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeSourceDigest(String);

/// Canonical digest of one immutable published Theme revision.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeRevisionDigest(String);

/// Canonical digest of deterministic transformed Theme CSS bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeStylesheetDigest(String);

/// Canonical digest of an immutable package asset.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeAssetDigest(String);

/// Canonical digest of the sorted entries in an explicit header-image pool.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemePoolRevisionDigest(String);

/// Input was not a canonical theme SHA-256 digest.
#[derive(Debug, Error)]
#[error("theme digest must be 64 lowercase hex characters")]
pub struct InvalidThemeDigest;

macro_rules! theme_digest_from_str {
    ($type:ident) => {
        impl FromStr for $type {
            type Err = InvalidThemeDigest;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                if crate::media::is_valid_content_hash(value) {
                    Ok(Self(value.to_owned()))
                } else {
                    Err(InvalidThemeDigest)
                }
            }
        }
    };
}

theme_digest_from_str!(ThemeSourceDigest);
theme_digest_from_str!(ThemeRevisionDigest);
theme_digest_from_str!(ThemeStylesheetDigest);
theme_digest_from_str!(ThemeAssetDigest);
theme_digest_from_str!(ThemePoolRevisionDigest);

/// A public selection is either one of the closed built-ins or an opaque custom
/// theme identity. The identity is intentionally stable across custom-theme renames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PublicThemeSelection {
    BuiltIn(Theme),
    Custom(crate::ids::ThemeId),
}

/// The two Style Contract image roles a Theme Package may supply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeImageRole {
    Logo,
    Header,
}

impl ThemeImageRole {
    /// Stable relational token shared by both database backends.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Logo => "logo",
            Self::Header => "header",
        }
    }
}

/// The closed source of one presentation-image role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeImageBindingMode {
    PackagedDefault,
    ExplicitAbsent,
    PackageAsset,
    Media,
    HeaderPool,
}

impl ThemeImageBindingMode {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::PackagedDefault => "packaged_default",
            Self::ExplicitAbsent => "explicit_absent",
            Self::PackageAsset => "package_asset",
            Self::Media => "media",
            Self::HeaderPool => "pool",
        }
    }
}

impl Theme {
    /// Stable token used by the public `data-theme` attribute and wire formats.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Studio => "studio",
            Self::Reader => "reader",
        }
    }
}

/// Whether a client route renders viewer-independent public presentation.
///
/// The shell uses this boundary to prevent a previously visited public theme
/// from leaking onto private application routes.
#[must_use]
pub fn is_public_presentation_path(path: &str) -> bool {
    path == "/" || path.starts_with("/tags/") || path.starts_with("/~")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_closed_and_studio_is_the_default() {
        assert_eq!(Theme::default(), Theme::Studio);
        for (token, theme) in [
            ("terminal", Theme::Terminal),
            ("studio", Theme::Studio),
            ("reader", Theme::Reader),
        ] {
            assert_eq!(token.parse::<Theme>().unwrap(), theme);
            assert_eq!(theme.token(), token);
        }
        assert!("solarized".parse::<Theme>().is_err());
    }

    #[test]
    fn public_path_classifier_excludes_private_application_routes() {
        for path in ["/", "/tags/rust", "/~alice", "/~alice/2026/01/02/post"] {
            assert!(is_public_presentation_path(path), "{path}");
        }
        for path in ["/app", "/profile", "/admin/site", "/login"] {
            assert!(!is_public_presentation_path(path), "{path}");
        }
    }

    #[test]
    fn content_digest_and_role_tokens_are_closed() {
        let digest = "a".repeat(64).parse::<ThemeContentDigest>().unwrap();
        assert_eq!(digest.as_ref(), "a".repeat(64));
        assert!("A".repeat(64).parse::<ThemeContentDigest>().is_err());
        assert_eq!(ThemeImageRole::Logo.token(), "logo");
        assert_eq!(ThemeImageRole::Header.token(), "header");
        assert_eq!(
            ThemeImageBindingMode::PackagedDefault.token(),
            "packaged_default"
        );
        assert_eq!(
            ThemeImageBindingMode::ExplicitAbsent.token(),
            "explicit_absent"
        );
        assert_eq!(ThemeImageBindingMode::PackageAsset.token(), "package_asset");
        assert_eq!(ThemeImageBindingMode::Media.token(), "media");
        assert_eq!(ThemeImageBindingMode::HeaderPool.token(), "pool");
    }
}
