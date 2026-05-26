use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FeedFormat {
    Rss,
    Atom,
    Json,
}

impl FeedFormat {
    #[must_use]
    pub fn ext(self) -> &'static str {
        match self {
            FeedFormat::Rss => "rss",
            FeedFormat::Atom => "atom",
            FeedFormat::Json => "json",
        }
    }
    #[must_use]
    pub fn content_type(self) -> &'static str {
        match self {
            FeedFormat::Rss => "application/rss+xml; charset=utf-8",
            FeedFormat::Atom => "application/atom+xml; charset=utf-8",
            FeedFormat::Json => "application/feed+json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FeedSurface {
    Site,
    SiteTag { tag: String },
    User { username: String },
    UserTag { username: String, tag: String },
}

#[must_use]
pub fn canonicalize(surface: &FeedSurface, format: FeedFormat) -> String {
    let ext = format.ext();
    match surface {
        FeedSurface::Site => format!("/feed.{ext}"),
        FeedSurface::SiteTag { tag } => format!("/tags/{tag}/feed.{ext}"),
        FeedSurface::User { username } => format!("/~{username}/feed.{ext}"),
        FeedSurface::UserTag { username, tag } => format!("/~{username}/tags/{tag}/feed.{ext}"),
    }
}

#[must_use]
pub fn parse(path: &str) -> Option<(FeedSurface, FeedFormat)> {
    // Strip leading '/'
    let rest = path.strip_prefix('/')?;
    // Find ".<ext>" suffix
    let (head, ext) = rest.rsplit_once('.')?;
    let format = match ext {
        "rss" => FeedFormat::Rss,
        "atom" => FeedFormat::Atom,
        "json" => FeedFormat::Json,
        _ => return None,
    };
    // Head must be "feed" (site) or end in "/feed"
    let surface_part = if head == "feed" {
        return Some((FeedSurface::Site, format));
    } else {
        head.strip_suffix("/feed")?
    };
    if surface_part.is_empty() {
        return None;
    }
    // tags/:tag
    if let Some(tag) = surface_part.strip_prefix("tags/") {
        if tag.is_empty() || tag.contains('/') {
            return None;
        }
        return Some((
            FeedSurface::SiteTag {
                tag: tag.to_string(),
            },
            format,
        ));
    }
    // ~:username[/tags/:tag]
    if let Some(after_tilde) = surface_part.strip_prefix('~') {
        if let Some((username, tag_part)) = after_tilde.split_once("/tags/") {
            if username.is_empty() || tag_part.is_empty() || tag_part.contains('/') {
                return None;
            }
            return Some((
                FeedSurface::UserTag {
                    username: username.to_string(),
                    tag: tag_part.to_string(),
                },
                format,
            ));
        }
        if after_tilde.contains('/') || after_tilde.is_empty() {
            return None;
        }
        return Some((
            FeedSurface::User {
                username: after_tilde.to_string(),
            },
            format,
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(surface: FeedSurface, format: FeedFormat) {
        let path = canonicalize(&surface, format);
        let parsed = parse(&path).expect("parses");
        assert_eq!(parsed, (surface, format));
    }

    #[test]
    fn round_trips_all_surfaces_and_formats() {
        for format in [FeedFormat::Rss, FeedFormat::Atom, FeedFormat::Json] {
            rt(FeedSurface::Site, format);
            rt(FeedSurface::SiteTag { tag: "rust".into() }, format);
            rt(
                FeedSurface::User {
                    username: "alice".into(),
                },
                format,
            );
            rt(
                FeedSurface::UserTag {
                    username: "alice".into(),
                    tag: "rust".into(),
                },
                format,
            );
        }
    }

    #[test]
    fn round_trips_tag_with_plus() {
        rt(FeedSurface::SiteTag { tag: "c++".into() }, FeedFormat::Rss);
    }

    #[test]
    fn round_trips_non_ascii_tag() {
        rt(
            FeedSurface::SiteTag {
                tag: "日本語".into(),
            },
            FeedFormat::Atom,
        );
        rt(
            FeedSurface::UserTag {
                username: "bob".into(),
                tag: "日本語".into(),
            },
            FeedFormat::Json,
        );
    }

    #[test]
    fn format_content_types() {
        assert_eq!(
            FeedFormat::Rss.content_type(),
            "application/rss+xml; charset=utf-8"
        );
        assert_eq!(
            FeedFormat::Atom.content_type(),
            "application/atom+xml; charset=utf-8"
        );
        assert_eq!(FeedFormat::Json.content_type(), "application/feed+json");
    }

    #[test]
    fn rejects_path_without_feed_suffix_in_subpath() {
        assert!(parse("/something/else.rss").is_none());
    }

    #[test]
    fn rejects_completely_unrecognized_path() {
        assert!(parse("/random/stuff/here.rss").is_none());
    }

    #[test]
    fn rejects_double_leading_slash() {
        // "//feed.rss" → head = "/feed", strip_suffix yields empty surface_part.
        assert!(parse("//feed.rss").is_none());
    }

    #[test]
    fn rejects_non_tag_non_user_prefix() {
        // surface_part = "something" — not tags/, not ~, no match.
        assert!(parse("/something/feed.rss").is_none());
    }

    #[test]
    fn rejects_unknown_extension() {
        assert!(parse("/feed.xml").is_none());
    }

    #[test]
    fn rejects_missing_feed_suffix() {
        assert!(parse("/~alice/profile.rss").is_none());
    }

    #[test]
    fn rejects_empty_tag() {
        assert!(parse("/tags//feed.rss").is_none());
        assert!(parse("/~alice/tags//feed.rss").is_none());
    }

    #[test]
    fn rejects_empty_username() {
        assert!(parse("/~/feed.rss").is_none());
    }
}
