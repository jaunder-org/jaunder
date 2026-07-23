//! A permalink-route segment that matches a username **only** when it is `~`-prefixed,
//! so a non-`~` same-segment-count URL (e.g. `/media/2026/01/01/x`) no longer matches the
//! SPA permalink route and falls to `<Routes fallback>` instead of mounting `PostPage`
//! (#592). The server owns `~`-prefixed permalinks by a literal `~` route
//! (`server/src/projector/mod.rs`); this mirrors that ownership on the client. Capture
//! keeps the `~` so `crate::posts::parse_permalink_params` strips it exactly as before.

use leptos_router::{ParamSegment, PartialPathMatch, PathSegment, PossibleRouteMatch};

/// A `ParamSegment` that only matches a `~`-prefixed first segment. The field is the
/// captured param's key (e.g. `"username"`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TildeUsername(pub &'static str);

impl PossibleRouteMatch for TildeUsername {
    fn optional(&self) -> bool {
        false
    }

    fn test<'a>(&self, path: &'a str) -> Option<PartialPathMatch<'a>> {
        let matched = ParamSegment(self.0).test(path)?;
        // Inspect via `matched()` (borrows `&self`, returns a `&'a str` tied to `path`) —
        // NOT `params()`, which takes `self` by value and would move `matched` before the
        // `then_some(matched)` below. The matched segment retains its leading `/`.
        matched
            .matched()
            .trim_start_matches('/')
            .starts_with('~')
            .then_some(matched)
    }

    fn generate_path(&self, path: &mut Vec<PathSegment>) {
        path.push(PathSegment::Param(self.0.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::TildeUsername;
    use leptos_router::PossibleRouteMatch;

    #[test]
    fn matches_tilde_username_and_captures_with_tilde() {
        let seg = TildeUsername("username");
        let m = seg
            .test("/~alice")
            .expect("should match a ~-prefixed segment");
        assert_eq!(m.matched(), "/~alice");
        assert_eq!(m.remaining(), "");
        let params = m.params();
        assert_eq!(params[0], ("username".into(), "~alice".into()));
    }

    #[test]
    fn matches_tilde_username_with_trailing_path() {
        // The leading segment of a full permalink; the rest stays in `remaining`.
        let m = TildeUsername("username")
            .test("/~alice/2026/01/01/hello")
            .expect("should match the first segment");
        assert_eq!(m.matched(), "/~alice");
        assert_eq!(m.remaining(), "/2026/01/01/hello");
    }

    #[test]
    fn rejects_non_tilde_first_segment() {
        assert!(TildeUsername("username").test("/media").is_none());
        assert!(TildeUsername("username")
            .test("/media/2026/01/01/x")
            .is_none());
        assert!(TildeUsername("username").test("/app").is_none());
    }

    #[test]
    fn rejects_empty_and_root() {
        assert!(TildeUsername("username").test("").is_none());
        assert!(TildeUsername("username").test("/").is_none());
    }

    #[test]
    fn is_never_optional() {
        // Mirrors `ParamSegment`: a permalink's username is a required segment.
        assert!(!TildeUsername("username").optional());
    }

    #[test]
    fn generate_path_emits_a_param_segment() {
        // Path generation treats it exactly like a `ParamSegment` (the `~`-guard is a
        // match-time concern only), so SSR route-list / link generation is unaffected.
        use leptos_router::PathSegment;
        let mut segments = Vec::new();
        TildeUsername("username").generate_path(&mut segments);
        assert_eq!(segments, vec![PathSegment::Param("username".into())]);
    }
}
