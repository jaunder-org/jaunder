//! Pure startup-theme resolution for the reactive app shell.

/// The visible startup theme plus any storage failure the browser adapter must
/// report before continuing.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct ThemeResolution<E> {
    /// Theme value the reactive shell should render.
    pub(super) theme: String,
    /// Unexpected storage failure the browser adapter must report.
    pub(super) error: Option<E>,
}

/// Resolve a truthful storage read without conflating absence with failure.
///
/// An absent or empty stored value selects `default` with no error. A storage
/// failure also preserves `default`, but returns the error for caller reporting.
pub(super) fn resolve_theme<E>(
    stored: Result<Option<String>, E>,
    default: &str,
) -> ThemeResolution<E> {
    match stored {
        Ok(Some(theme)) if !theme.is_empty() => ThemeResolution { theme, error: None },
        Ok(_) => ThemeResolution {
            theme: default.to_owned(),
            error: None,
        },
        Err(error) => ThemeResolution {
            theme: default.to_owned(),
            error: Some(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_nonempty_theme_wins() {
        assert_eq!(
            resolve_theme::<()>(Ok(Some("night".to_owned())), "studio"),
            ThemeResolution {
                theme: "night".to_owned(),
                error: None,
            }
        );
    }

    #[test]
    fn absent_and_empty_theme_use_default_without_error() {
        for stored in [None, Some(String::new())] {
            assert_eq!(
                resolve_theme::<()>(Ok(stored), "studio"),
                ThemeResolution {
                    theme: "studio".to_owned(),
                    error: None,
                }
            );
        }
    }

    #[test]
    fn storage_failure_uses_default_and_returns_error() {
        assert_eq!(
            resolve_theme(Err("denied"), "studio"),
            ThemeResolution {
                theme: "studio".to_owned(),
                error: Some("denied"),
            }
        );
    }
}
