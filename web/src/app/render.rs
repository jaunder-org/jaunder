//! The pure shell projector — the app vertical's host-compiled render leaf
//! (ADR-0070, like `crate::posts::render`).
//!
//! Non-reactive, plain-string HTML for the page frame (`<head>` + the `j-root`
//! shell). Shared by the server-side projector (`server::projector`) and the
//! reactive shell (`crate::app::component`'s `App`/`AppShell`): both derive the
//! SAME layout from the SAME data, so the projector's server-painted shell and
//! the client's first paint coincide byte-for-byte (flash-free, #181 / ADR-0044).
//! There is deliberately NO leptos reactivity here — plain string building only,
//! like the discovery-link helpers — so `reactive_graph` never sits on the public request
//! path (the #173 escape). See `docs/adr/0041` and `docs/inbound-data-handling.md`
//! §4.
//!
//! Ungated and host-compiled, so these twins stay host-tested and
//! coverage-measured; the `#[cfg(test)] mod tests` below are the coincidence
//! tests. Leaf primitives it composes (`posts::render::body`, `render_sidebar`) live in
//! their own modules and are called cross-module.

use common::seed::{PageSeed, PublicPresentation};
use maud::{PreEscaped, html};

use crate::html::Markup;

/// The pre-paint auth-detection script (#181, ADR-0044). A tiny inline, blocking
/// `<head>` script: reads the localStorage auth marker (`jaunder_auth`, same key
/// as `auth::marker`) and marks `<html class="authed" data-user=…>` BEFORE first
/// paint, so CSS reserves the authed layout and the SPA boots already knowing.
/// Never external/deferred (a round-trip would guarantee paint-then-swap).
/// The redirect-pref (`jaunder_home_redirect`) read path is present with the
/// safe stay-default — nothing writes the key yet (ADR-0044 D7/D10). Bytes are
/// identical for every visitor → cacheability intact. The bundle producer renders
/// this source template into the static shell after resolving asset identities.
pub const PREPAINT_SCRIPT: &str = concat!(
    "<script>\n",
    "      // prettier-ignore\n",
    "      (function () {try {var m = localStorage.getItem('jaunder_auth'); if (m) {var u = JSON.parse(m).username; if (u) {var e = document.documentElement; e.classList.add('authed'); e.setAttribute('data-user', u); if (localStorage.getItem('jaunder_home_redirect') === 'app' && location.pathname === '/') {location.replace('/app');} } } } catch (_) { } })();\n",
    "    </script>",
);

/// The document-frame mark emitted immediately before `initMeasured()` consumes
/// the early response (or starts the explicit fallback). Shared with both shell
/// surfaces and drift-guarded in tests (#870).
pub const MODULE_BEFORE_INIT_MARK: &str = "jaunder.module.before_init";

/// The document `<head>` inner HTML: the host supplies the generated early
/// wasm fetch script, keeping final runtime asset identity outside `web`.
#[must_use]
pub fn render_head(seed: &PageSeed, early_wasm_fetch_script: Option<&str>) -> Markup {
    let (title, description) = match seed {
        PageSeed::Permalink(authored) => (
            authored.post.title.clone().map_or_else(
                || format!("Post by {}", authored.post.username),
                String::from,
            ),
            authored
                .post
                .summary
                .as_deref()
                .unwrap_or_default()
                .to_owned(),
        ),
        PageSeed::Profile { username, .. } => (format!("Posts by {username}"), String::new()),
        PageSeed::SiteTimeline(_) => ("Jaunder".to_string(), String::new()),
        PageSeed::SiteTag { tag, .. } => (format!("#{tag}"), String::new()),
        PageSeed::UserTag { username, tag, .. } => (format!("#{tag} by {username}"), String::new()),
    };
    Markup::new(html! {
        @if let Some(script) = early_wasm_fetch_script {
            // raw-html-door:allow generated server bundle URL script; no request or viewer data enters this sink
            (PreEscaped(script))
        }
        meta charset="utf-8";
        meta name="viewport" content="width=device-width, initial-scale=1";
        // NO wasm preload here — a measured decision with a fired abort rule, not
        // an oversight (#866; docs/adr/0121-no-wasm-preload.md). Do not re-add
        // without reading that draft; `crossorigin` would be mandatory.
        link rel="stylesheet" href="/style/jaunder.css";
        link rel="stylesheet" href="/style/jaunder-themes.css";
        title { (title) }
        meta name="description" content=(description);
        meta property="og:title" content=(title);
        meta property="og:description" content=(description);
        (render_discovery(seed))
    })
}

/// Marker attribute on each projector-painted autodiscovery `<link>`. The CSR boot
/// (`csr::mount`) removes `link[data-jaunder-discovery]` before mounting so the reactive
/// `FeedDiscovery`/`RsdDiscovery` own the single post-boot set (#198). Shared here so the
/// emitter below and the boot-time remover cannot drift.
pub const DISCOVERY_MARKER_ATTR: &str = "data-jaunder-discovery";

/// Feed + RSD autodiscovery `<link>`s for the seed's surface, the pure mirror of
/// the reactive `FeedDiscovery`/`RsdDiscovery` components (`web::feed_discovery`)
/// so the projector's `<head>` carries the same discovery metadata the reactive
/// CSR components produce (feed readers + `AtomPub` editors follow these). Each page emits
/// exactly what its reactive counterpart does: the RSS/Atom/JSON feed links for
/// its surface, and — only on the user-profile page — the RSD `EditURI` link. The
/// permalink page renders none. Post-boot the reactive components re-add
/// identical links; the duplicates are invisible.
fn render_discovery(seed: &PageSeed) -> Markup {
    use common::feed::{FeedFormat, FeedSurface, canonicalize};

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
        PageSeed::Permalink(_) => None,
    };

    Markup::new(html! {
        @if let Some(surface) = surface {
            @let label = feed_label(&surface);
            @for (format, suffix, mime) in [
                (FeedFormat::Rss, "RSS", "application/rss+xml"),
                (FeedFormat::Atom, "Atom", "application/atom+xml"),
                (FeedFormat::Json, "JSON Feed", "application/feed+json"),
            ] {
                link data-jaunder-discovery rel="alternate" type=(mime)
                    title={ (label) " (" (suffix) ")" }
                    href=(canonicalize(&surface, format));
            }
        }
        @if let PageSeed::Profile { username, .. } = seed {
            link data-jaunder-discovery rel="EditURI" type="application/rsd+xml"
                title="AtomPub (RSD)" href={ "/~" (username) "/rsd.xml" };
        }
    })
}

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
pub fn render_shell(presentation: &PublicPresentation<PageSeed>) -> Markup {
    let seed = &presentation.page;
    let active_key = matches!(seed, PageSeed::SiteTimeline(_))
        .then_some("/")
        .and_then(crate::sidebar::active_key)
        .unwrap_or("");
    Markup::new(html! {
        div class="j-root" data-theme=(presentation.theme.as_ref()) {
            div id="j-trusted-chrome" class="j-trusted-chrome" {}
            div class="j-theme-clip" data-jaunder-theme-clip {
                div class="j-shell" data-jaunder-theme-surface data-jaunder-style-contract="1" {
                    aside class="j-sidebar" { (crate::sidebar::render_sidebar(active_key)) }
                    div class="j-main-region" {
                        main class="j-main" data-jaunder-part="main" { (crate::posts::render::body(seed)) }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // Shared with the reactive suite (`crate::posts::render`) so both assert the
    // projector↔reactive coincidence against the same fixture, not a divergent copy.
    use crate::posts::render::test_fixtures::{one_post_page, sample_post};
    use common::local_storage_key::LocalStorageKey;
    use common::test_support::parse_username;

    #[test]
    fn discovery_links_carry_the_marker_per_surface() {
        // Site: three feed links, all marked, no RSD (#198 — the boot-time remover keys
        // on the marker, so every projector discovery <link> must carry it).
        let site = render_discovery(&PageSeed::SiteTimeline(one_post_page())).into_string();
        assert_eq!(site.matches(DISCOVERY_MARKER_ATTR).count(), 3, "{site}");
        assert_eq!(site.matches("rel=\"alternate\"").count(), 3, "{site}");
        assert!(!site.contains("EditURI"), "{site}");
        // Profile: three feed links + one RSD, all four marked.
        let profile = render_discovery(&PageSeed::Profile {
            username: parse_username("bob"),
            page: one_post_page(),
        })
        .into_string();
        assert_eq!(
            profile.matches(DISCOVERY_MARKER_ATTR).count(),
            4,
            "{profile}"
        );
        assert!(profile.contains("rel=\"EditURI\""), "{profile}");
        // Permalink: none.
        assert_eq!(
            render_discovery(&PageSeed::Permalink(sample_post())).as_str(),
            ""
        );
    }

    /// maud (like any compile-time markup macro) needs a literal attribute *name*,
    /// so `render_discovery` spells `data-jaunder-discovery` out rather than
    /// splicing the const. The const is still the one `csr::mount` uses to build its
    /// removal selector (#198), so pin the two together — otherwise changing the
    /// const would silently stop matching the markup and the boot-time remover would
    /// quietly no-op.
    #[test]
    fn discovery_marker_attr_matches_the_literal_written_in_the_markup() {
        assert_eq!(DISCOVERY_MARKER_ATTR, "data-jaunder-discovery");
    }

    #[test]
    fn prepaint_script_is_inline_blocking_and_reads_the_registered_storage_keys() {
        let s = PREPAINT_SCRIPT;
        assert!(s.starts_with("<script>") && s.ends_with("</script>"), "{s}");
        // No async/defer/src — a network round-trip would defeat pre-paint.
        assert!(
            !s.contains("src=") && !s.contains("defer") && !s.contains("async"),
            "{s}"
        );
        // Reads the same key + field the marker module writes, and the redirect
        // preference key reserved for the pre-WASM home redirect path.
        for key in [
            LocalStorageKey::AuthMarker,
            LocalStorageKey::HomeRedirectPreference,
        ] {
            let read = format!("localStorage.getItem('{}')", key.as_ref());
            assert!(s.contains(&read), "{s}");
        }
        assert!(s.contains(".username"), "{s}");
        assert!(s.contains("classList") && s.contains("authed"), "{s}");
    }

    #[test]
    fn host_supplied_early_fetch_script_precedes_stylesheets() {
        let starter =
            r#"<script>window.__jaunderWasmFetch = fetch("/pkg/wasm-hash.wasm");</script>"#;
        let head =
            render_head(&PageSeed::SiteTimeline(one_post_page()), Some(starter)).into_string();
        let style = head
            .find(r#"<link rel="stylesheet" href="/style/jaunder.css">"#)
            .expect("base stylesheet");
        assert!(head.starts_with(starter) && starter.len() < style, "{head}");
    }

    #[test]
    fn permalink_head_sets_escaped_title_and_og() {
        let head = render_head(&PageSeed::Permalink(sample_post()), None).into_string();
        assert!(
            head.contains("<title>Hello &amp; &lt;World&gt;</title>"),
            "{head}"
        );
        assert!(head.contains("<meta property=\"og:title\""), "{head}");
    }

    // A titleless post still needs a `<title>`: this is the SEO payload the public
    // surface stays server-rendered for, so an empty one is a real defect rather than
    // a cosmetic one. The author's name is the fallback.
    #[test]
    fn permalink_head_falls_back_to_the_author_when_a_post_has_no_title() {
        let mut untitled = sample_post();
        untitled.post.title = None;
        let head = render_head(&PageSeed::Permalink(untitled), None).into_string();
        assert!(head.contains("<title>Post by alice</title>"), "{head}");
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
            let head = render_head(&seed, None).into_string();
            assert!(head.contains(expected_title), "{head}");
        }
    }

    #[test]
    fn shell_has_one_versioned_theme_surface_inside_a_paint_clip() {
        let html = render_shell(&PublicPresentation {
            theme: common::theme::Theme::Studio,
            page: PageSeed::SiteTimeline(one_post_page()),
        })
        .into_string();
        assert_eq!(
            html.matches("data-jaunder-theme-surface").count(),
            1,
            "{html}"
        );
        assert_eq!(
            html.matches("data-jaunder-part=\"primary-navigation\"")
                .count(),
            1,
            "{html}"
        );
        assert_eq!(
            html.matches("data-jaunder-part=\"main\"").count(),
            1,
            "{html}"
        );
        assert!(
            html.starts_with(
                "<div class=\"j-root\" data-theme=\"studio\"><div id=\"j-trusted-chrome\" class=\"j-trusted-chrome\"></div>\
                 <div class=\"j-theme-clip\" data-jaunder-theme-clip><div class=\"j-shell\" \
                 data-jaunder-theme-surface data-jaunder-style-contract=\"1\"><aside class=\"j-sidebar\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("</aside><div class=\"j-main-region\"><main class=\"j-main\" data-jaunder-part=\"main\">"),
            "{html}"
        );
        assert!(html.ends_with("</main></div></div></div></div>"), "{html}");
    }

    #[test]
    fn shell_marks_home_active_only_for_the_site_timeline() {
        let root = render_shell(&PublicPresentation {
            theme: common::theme::Theme::Studio,
            page: PageSeed::SiteTimeline(one_post_page()),
        })
        .into_string();
        assert!(
            root.contains("<a class=\"j-nav-item is-active\" href=\"/\">"),
            "{root}"
        );

        let non_root = render_shell(&PublicPresentation {
            theme: common::theme::Theme::Studio,
            page: PageSeed::Profile {
                username: parse_username("bob"),
                page: one_post_page(),
            },
        })
        .into_string();
        assert!(
            non_root.contains("<a class=\"j-nav-item\" href=\"/\">"),
            "{non_root}"
        );
        assert!(!non_root.contains("j-nav-item is-active"), "{non_root}");
    }

    #[test]
    fn page_seed_round_trips_through_json() {
        let seed = PageSeed::Permalink(sample_post());
        let json = serde_json::to_string(&seed).unwrap();
        let back: PageSeed = serde_json::from_str(&json).unwrap();
        assert_eq!(seed, back);
    }
}
