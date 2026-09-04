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
}
