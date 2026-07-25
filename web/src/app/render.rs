//! The pure shell projector — the app vertical's host-compiled render leaf
//! (ADR-0070, like `crate::posts::render`).
//!
//! Non-reactive, plain-string HTML for the page frame (`<head>` + the `j-root`
//! shell). Shared by the server-side projector (`server::projector`) and the
//! reactive shell (`crate::app::component`'s `App`/`AppShell`): both derive the
//! SAME layout from the SAME data, so the projector's server-painted shell and
//! the client's first paint coincide byte-for-byte (flash-free, #181 / ADR-0044).
//! There is deliberately NO leptos reactivity here — plain string building only,
//! like `common::feed` — so `reactive_graph` never sits on the public request
//! path (the #173 escape). See `docs/adr/0041` and `docs/inbound-data-handling.md`
//! §4.
//!
//! Ungated and host-compiled, so these twins stay host-tested and
//! coverage-measured; the `#[cfg(test)] mod tests` below are the coincidence
//! tests. Leaf primitives it composes (`escape_html`, `render_body`,
//! `render_sidebar`) live in their own modules and are called cross-module.

use common::seed::PageSeed;
use std::fmt::Write as _;

use crate::render::escape_html;

/// The default theme applied to `<div class="j-root" data-theme=…>`. Lives here
/// (the shell-rendering layer) so the projector's server-painted shell and the
/// reactive `AppShell` share one value; re-exported from `app` (via `mod.rs`) for
/// the client.
pub const DEFAULT_THEME: &str = "studio";

/// The pre-paint auth-detection script (#181, ADR-0044). A tiny inline, blocking
/// `<head>` script: reads the localStorage auth marker (`jaunder_auth`, same key
/// as `auth::marker`) and marks `<html class="authed" data-user=…>` BEFORE first
/// paint, so CSS reserves the authed layout and the SPA boots already knowing.
/// Never external/deferred (a round-trip would guarantee paint-then-swap). The
/// redirect-pref (`jaunder_home_redirect`) read path is present with the safe
/// stay-default — nothing writes the key yet (ADR-0044 D7/D10). Bytes are
/// identical for every visitor → cacheability intact. Kept byte-identical in
/// `csr/index.html` (a `<!-- prettier-ignore -->`-pinned copy, drift-guarded by a
/// unit test) — deliberately minified so the two copies can match verbatim.
pub const PREPAINT_SCRIPT: &str = "<script>(function(){try{\
var m=localStorage.getItem('jaunder_auth');\
if(m){var u=JSON.parse(m).username;\
if(u){var e=document.documentElement;e.classList.add('authed');e.setAttribute('data-user',u);\
if(localStorage.getItem('jaunder_home_redirect')==='app'&&location.pathname==='/'){location.replace('/app');}}}\
}catch(_){}})();</script>";

/// The CSR SPA shell, embedded at compile time. The `cargo xtask build-csr` build
/// never writes `index.html` to `site_root` (#239); the server owns it and serves it — the
/// same way the projector renders its routes from constants. Single source of the
/// shell; copied to no build output.
pub const SPA_SHELL: &str = include_str!("../../../csr/index.html");

/// The document `<head>` inner HTML: per-page title + description + Open Graph.
/// This is the SEO/discoverability payload — the whole reason the public
/// surface stays server-rendered.
#[must_use]
pub fn render_head(seed: &PageSeed) -> String {
    let (title, description) = match seed {
        PageSeed::Permalink(post) => (
            post.title
                .clone()
                .map_or_else(|| format!("Post by {}", post.username), String::from),
            post.summary.as_deref().unwrap_or_default().to_owned(),
        ),
        PageSeed::Profile { username, .. } => (format!("Posts by {username}"), String::new()),
        PageSeed::SiteTimeline(_) => ("Jaunder".to_string(), String::new()),
        PageSeed::SiteTag { tag, .. } => (format!("#{tag}"), String::new()),
        PageSeed::UserTag { username, tag, .. } => (format!("#{tag} by {username}"), String::new()),
    };
    let title = escape_html(&title);
    let description = escape_html(&description);
    let mut head = format!(
        concat!(
            "<meta charset=\"utf-8\" />",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />",
            "<link rel=\"stylesheet\" href=\"/style/jaunder.css\" />",
            "<link rel=\"stylesheet\" href=\"/style/jaunder-themes.css\" />",
            "<title>{title}</title>",
            "<meta name=\"description\" content=\"{description}\" />",
            "<meta property=\"og:title\" content=\"{title}\" />",
            "<meta property=\"og:description\" content=\"{description}\" />",
        ),
        title = title,
        description = description,
    );
    head.push_str(&render_discovery(seed));
    head
}

/// Marker attribute on each projector-painted autodiscovery `<link>`. The CSR boot
/// (`csr::mount`) removes `link[data-jaunder-discovery]` before mounting so the reactive
/// `FeedDiscovery`/`RsdDiscovery` own the single post-boot set (#198). Shared here so the
/// emitter below and the boot-time remover cannot drift.
pub const DISCOVERY_MARKER_ATTR: &str = "data-jaunder-discovery";

/// Feed + RSD autodiscovery `<link>`s for the seed's surface, the pure mirror of
/// the reactive `FeedDiscovery`/`RsdDiscovery` components (`web::feed_discovery`)
/// so the projector's `<head>` carries the same discovery metadata the reactive
/// SSR render did (feed readers + `AtomPub` editors follow these). Each page emits
/// exactly what its reactive counterpart does: the RSS/Atom/JSON feed links for
/// its surface, and — only on the user-profile page — the RSD `EditURI` link. The
/// permalink page renders none. Post-boot the reactive components re-add
/// identical links; the duplicates are invisible.
fn render_discovery(seed: &PageSeed) -> String {
    use common::feed::{canonicalize, FeedFormat, FeedSurface};

    let mut out = String::new();

    let surface = match seed {
        PageSeed::SiteTimeline(_) => Some(FeedSurface::Site),
        PageSeed::SiteTag { tag, .. } => Some(FeedSurface::SiteTag { tag: tag.clone() }),
        PageSeed::Profile { username, .. } => Some(FeedSurface::User {
            username: username.clone(),
        }),
        PageSeed::UserTag { username, tag, .. } => Some(FeedSurface::UserTag {
            username: username.clone(),
            tag: tag.clone(),
        }),
        // The reactive permalink page renders no discovery links.
        PageSeed::Permalink(_) => None,
    };

    if let Some(surface) = surface {
        let label = feed_label(&surface);
        for (format, suffix, mime) in [
            (FeedFormat::Rss, "RSS", "application/rss+xml"),
            (FeedFormat::Atom, "Atom", "application/atom+xml"),
            (FeedFormat::Json, "JSON Feed", "application/feed+json"),
        ] {
            let _ = write!(
                out,
                "<link {marker} rel=\"alternate\" type=\"{mime}\" title=\"{title}\" href=\"{href}\" />",
                marker = DISCOVERY_MARKER_ATTR,
                title = escape_html(format!("{label} ({suffix})")),
                href = escape_html(canonicalize(&surface, format)),
            );
        }
    }

    // Only the reactive user-profile page hoists the RSD link (the user-tag page
    // does not), so mirror that exactly.
    if let PageSeed::Profile { username, .. } = seed {
        let _ = write!(
            out,
            "<link {marker} rel=\"EditURI\" type=\"application/rsd+xml\" title=\"AtomPub (RSD)\" href=\"{href}\" />",
            marker = DISCOVERY_MARKER_ATTR,
            href = escape_html(format!("/~{username}/rsd.xml")),
        );
    }

    out
}

/// Human-readable feed title per surface — the pure mirror of the reactive
/// `web::feed_discovery::labels::surface_label`.
fn feed_label(surface: &common::feed::FeedSurface) -> String {
    use common::feed::FeedSurface;
    match surface {
        FeedSurface::Site => "Site feed".to_string(),
        FeedSurface::SiteTag { tag } => format!("#{tag} feed"),
        FeedSurface::User { username } => format!("@{username} feed"),
        FeedSurface::UserTag { username, tag } => format!("@{username} #{tag} feed"),
    }
}

/// The full anonymous `#app` shell the projector serves: the exact `j-root`
/// layout the reactive `App`/`AppShell` produces for an anonymous viewer (the
/// sidebar, the main region, and the per-route `<main>` content), so removing
/// `#app` and mounting the CSR app on boot causes no reflow. The authed extras
/// (footer avatar, authed nav, action columns) layer on top reactively once
/// `current_user` resolves (that is #181, and needs no coincidence).
/// `BackupBanner` renders nothing for an anonymous viewer, so it is omitted here.
#[must_use]
pub fn render_shell(seed: &PageSeed) -> String {
    format!(
        concat!(
            "<div class=\"j-root\" data-theme=\"{theme}\"><div class=\"j-shell\">",
            "<aside class=\"j-sidebar\">{sidebar}</aside>",
            "<div class=\"j-main-region\"><main class=\"j-main\">{body}</main></div></div></div>",
        ),
        theme = DEFAULT_THEME,
        sidebar = crate::sidebar::render_sidebar(""),
        body = crate::posts::render::render_body(seed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    // Shared with the reactive suite (`crate::posts::render`) so both assert the
    // projector↔reactive coincidence against the same fixture, not a divergent copy.
    use crate::posts::render::test_fixtures::{one_post_page, sample_post};
    use common::test_support::parse_username;

    #[test]
    fn discovery_links_carry_the_marker_per_surface() {
        // Site: three feed links, all marked, no RSD (#198 — the boot-time remover keys
        // on the marker, so every projector discovery <link> must carry it).
        let site = render_discovery(&PageSeed::SiteTimeline(one_post_page()));
        assert_eq!(site.matches(DISCOVERY_MARKER_ATTR).count(), 3, "{site}");
        assert_eq!(site.matches("rel=\"alternate\"").count(), 3, "{site}");
        assert!(!site.contains("EditURI"), "{site}");
        // Profile: three feed links + one RSD, all four marked.
        let profile = render_discovery(&PageSeed::Profile {
            username: parse_username("bob"),
            page: one_post_page(),
        });
        assert_eq!(
            profile.matches(DISCOVERY_MARKER_ATTR).count(),
            4,
            "{profile}"
        );
        assert!(profile.contains("rel=\"EditURI\""), "{profile}");
        // Permalink: none.
        assert_eq!(render_discovery(&PageSeed::Permalink(sample_post())), "");
    }

    #[test]
    fn default_theme_is_nonempty() {
        assert!(!DEFAULT_THEME.is_empty());
    }

    #[test]
    fn prepaint_script_is_inline_blocking_and_reads_the_marker() {
        let s = PREPAINT_SCRIPT;
        assert!(s.starts_with("<script>") && s.ends_with("</script>"), "{s}");
        // No async/defer/src — a network round-trip would defeat pre-paint.
        assert!(
            !s.contains("src=") && !s.contains("defer") && !s.contains("async"),
            "{s}"
        );
        // Reads the same key + field the marker module writes.
        assert!(s.contains("jaunder_auth"), "{s}");
        assert!(s.contains(".username"), "{s}");
        assert!(s.contains("classList") && s.contains("authed"), "{s}");
    }

    #[test]
    fn index_html_shell_contains_the_prepaint_script() {
        // The projector's SPA-shell fallback IS csr/index.html; it must carry the
        // identical pre-paint script (a prettier-ignored, minified copy) so
        // authed-only / shell-fallback pages pre-paint too.
        let index = include_str!("../../../csr/index.html");
        assert!(
            index.contains(PREPAINT_SCRIPT),
            "csr/index.html must embed app::PREPAINT_SCRIPT verbatim (drift guard)"
        );
    }

    #[test]
    fn csr_index_html_boots_wasm_with_an_explicit_url() {
        // Fast unit smoke (#234): the SPA shell must pass an explicit wasm URL to
        // init(), not the arg-less init() that falls back to wasm-bindgen's
        // `jaunder_bg.wasm` default. This runs in `check`; `cargo xtask audit-wasm`
        // is what ties this URL to the file the build actually emits.
        let index = include_str!("../../../csr/index.html");
        assert!(
            index.contains(r#"init("/pkg/jaunder.wasm")"#),
            "csr/index.html must boot via an explicit init(\"/pkg/jaunder.wasm\") (drift guard #234)"
        );
    }

    #[test]
    fn permalink_head_sets_escaped_title_and_og() {
        let head = render_head(&PageSeed::Permalink(sample_post()));
        assert!(
            head.contains("<title>Hello &amp; &lt;World&gt;</title>"),
            "{head}"
        );
        assert!(head.contains("<meta property=\"og:title\""), "{head}");
    }

    #[test]
    fn head_titles_cover_every_page_kind() {
        let cases = [
            (
                PageSeed::SiteTimeline(one_post_page()),
                "<title>Jaunder</title>",
            ),
            (
                PageSeed::Profile {
                    username: parse_username("bob"),
                    page: one_post_page(),
                },
                "<title>Posts by bob</title>",
            ),
            (
                PageSeed::SiteTag {
                    tag: "rust".parse().unwrap(),
                    page: one_post_page(),
                },
                "<title>#rust</title>",
            ),
            (
                PageSeed::UserTag {
                    username: parse_username("bob"),
                    tag: "rust".parse().unwrap(),
                    page: one_post_page(),
                },
                "<title>#rust by bob</title>",
            ),
        ];
        for (seed, expected_title) in cases {
            let head = render_head(&seed);
            assert!(head.contains(expected_title), "{head}");
        }
    }

    #[test]
    fn shell_wraps_body_in_j_root_with_sidebar_and_main() {
        let html = render_shell(&PageSeed::SiteTimeline(one_post_page()));
        assert!(
            html.starts_with(
                "<div class=\"j-root\" data-theme=\"studio\"><div class=\"j-shell\">\
                 <aside class=\"j-sidebar\">"
            ),
            "{html}"
        );
        // Sidebar inner is present, then the main region.
        assert!(html.contains("j-brand-text"), "{html}");
        assert!(
            html.contains("</aside><div class=\"j-main-region\"><main class=\"j-main\">"),
            "{html}"
        );
        assert!(html.ends_with("</main></div></div></div>"), "{html}");
    }

    #[test]
    fn page_seed_round_trips_through_json() {
        let seed = PageSeed::Permalink(sample_post());
        let json = serde_json::to_string(&seed).unwrap();
        let back: PageSeed = serde_json::from_str(&json).unwrap();
        assert_eq!(seed, back);
    }
}
