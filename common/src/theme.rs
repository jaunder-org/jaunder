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
            assert_eq!(theme.as_ref(), token);
        }
        assert!("solarized".parse::<Theme>().is_err());
    }
}
