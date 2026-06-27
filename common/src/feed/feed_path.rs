use serde::{Deserialize, Serialize};

use crate::{tag::Tag, username::Username};

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
    SiteTag { tag: Tag },
    User { username: Username },
    UserTag { username: Username, tag: Tag },
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

/// Every feed URL a post by `username` carrying `tags` can appear on: the site
/// feed, the author's feed, and the site-/user-tag feeds for each tag — each in
/// all three [`FeedFormat`]s. Shared by the write-path feed fan-out (`web`) and
/// the feed worker's go-live pass so both enqueue exactly the same surfaces.
#[must_use]
pub fn affected_feed_urls<'a, I>(username: &Username, tags: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a Tag>,
{
    let mut surfaces = vec![
        FeedSurface::Site,
        FeedSurface::User {
            username: username.clone(),
        },
    ];
    for tag in tags {
        surfaces.push(FeedSurface::SiteTag { tag: tag.clone() });
        surfaces.push(FeedSurface::UserTag {
            username: username.clone(),
            tag: tag.clone(),
        });
    }
    let mut urls = Vec::with_capacity(surfaces.len() * 3);
    for surface in &surfaces {
        for format in [FeedFormat::Rss, FeedFormat::Atom, FeedFormat::Json] {
            urls.push(canonicalize(surface, format));
        }
    }
    urls
}

#[must_use]
pub fn parse(path: &str) -> Option<(FeedSurface, FeedFormat)> {
    let rest = path.strip_prefix('/')?;
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
        // `Tag::from_str` rejects empty input and any non-`[a-z0-9-]`
        // character (including `/`), so it subsumes the prior ad-hoc checks
        // while also rejecting inputs the old parser wrongly accepted
        // (uppercase, dots, leading hyphen, non-ASCII).
        let tag = tag.parse::<Tag>().ok()?;
        return Some((FeedSurface::SiteTag { tag }, format));
    }
    // ~:username[/tags/:tag]
    if let Some(after_tilde) = surface_part.strip_prefix('~') {
        if let Some((username, tag_part)) = after_tilde.split_once("/tags/") {
            let username = username.parse::<Username>().ok()?;
            let tag = tag_part.parse::<Tag>().ok()?;
            return Some((FeedSurface::UserTag { username, tag }, format));
        }
        let username = after_tilde.parse::<Username>().ok()?;
        return Some((FeedSurface::User { username }, format));
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

    fn tag(s: &str) -> Tag {
        s.parse().expect("valid tag")
    }

    fn user(s: &str) -> Username {
        s.parse().expect("valid username")
    }

    #[test]
    fn round_trips_all_surfaces_and_formats() {
        for format in [FeedFormat::Rss, FeedFormat::Atom, FeedFormat::Json] {
            rt(FeedSurface::Site, format);
            rt(FeedSurface::SiteTag { tag: tag("rust") }, format);
            rt(
                FeedSurface::User {
                    username: user("alice"),
                },
                format,
            );
            rt(
                FeedSurface::UserTag {
                    username: user("alice"),
                    tag: tag("rust"),
                },
                format,
            );
        }
    }

    #[test]
    fn round_trips_hyphenated_tag() {
        rt(
            FeedSurface::SiteTag {
                tag: tag("hello-world"),
            },
            FeedFormat::Rss,
        );
    }

    // Previously-divergent inputs: the old ad-hoc parser accepted these
    // (anything without '/'), but the canonical `Tag`/`Username` validators
    // reject them, so `parse()` must now refuse them too.
    #[test]
    fn rejects_inputs_the_canonical_validators_reject() {
        // `+` is not in the tag/username grammar (old parser accepted "c++").
        assert!(parse("/tags/c++/feed.rss").is_none());
        // Non-ASCII (old parser accepted "日本語").
        assert!(parse("/tags/日本語/feed.atom").is_none());
        assert!(parse("/~bob/tags/日本語/feed.json").is_none());
        // Leading hyphen is rejected by `Tag::from_str`.
        assert!(parse("/tags/-rust/feed.rss").is_none());
        // Dots in the username segment are rejected by `Username::from_str`.
        assert!(parse("/~al.ice/feed.rss").is_none());
    }

    // The canonical validators lowercase their input, so a mixed-case path
    // parses to the normalized newtype (and no longer round-trips verbatim).
    #[test]
    fn normalizes_case_via_canonical_validators() {
        assert_eq!(
            parse("/~Alice/feed.rss"),
            Some((
                FeedSurface::User {
                    username: user("alice"),
                },
                FeedFormat::Rss,
            ))
        );
        assert_eq!(
            parse("/tags/Rust/feed.atom"),
            Some((FeedSurface::SiteTag { tag: tag("rust") }, FeedFormat::Atom))
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
