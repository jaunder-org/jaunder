//! Production-derived quality regressions for host-rendered Post code.

use common::{post_body::PostBody, render::PostFormat};

#[path = "quality/catalog.rs"]
mod catalog;

// Captured as decoded <code> text from the two public Posts on 2026-10-04.
// Keep the malformed Haskell pragma intact: repairing the source would hide
// the real Post's whole-block keyword regression.
const HASKELL: &str = include_str!("fixtures/production-haskell.hs");
const ELISP: &str = include_str!("fixtures/production-elisp.el");

// The host renderer emits only nested spans inside a code element. Walk that
// closed output shape rather than matching serialized tag boundaries: a role
// may cover a token through nested punctuation or split string captures.
pub(super) fn semantic_code(html: &str) -> (String, Vec<Vec<&str>>) {
    let (_, rest) = html.split_once("<code").expect("rendered code block");
    let (_, rest) = rest.split_once('>').expect("open code element");
    let (inner, _) = rest.split_once("</code>").expect("close code element");
    let mut text = String::new();
    let mut roles = Vec::new();
    let mut scopes: Vec<Option<&str>> = Vec::new();
    let mut remaining = inner;
    while !remaining.is_empty() {
        if remaining.starts_with("<span") {
            let (tag, after) = remaining.split_once('>').expect("span start tag");
            scopes.push(
                tag.strip_prefix("<span class=\"")
                    .and_then(|class| class.strip_suffix('"')),
            );
            remaining = after;
        } else if let Some(after) = remaining.strip_prefix("</span>") {
            scopes.pop().expect("balanced spans");
            remaining = after;
        } else {
            let length = remaining.find('<').unwrap_or(remaining.len());
            assert!(length > 0, "unexpected markup inside code: {remaining}");
            let decoded = html_escape::decode_html_entities(&remaining[..length]);
            let active = scopes.iter().filter_map(|scope| *scope).collect::<Vec<_>>();
            roles.extend(std::iter::repeat_n(active, decoded.len()));
            text.push_str(&decoded);
            remaining = &remaining[length..];
        }
    }
    assert!(scopes.is_empty(), "unbalanced spans in rendered code");
    assert_eq!(roles.len(), text.len());
    (text, roles)
}

fn render_code(label: &str, code: &str, format: PostFormat) -> String {
    let source = match format {
        PostFormat::Markdown => format!("```{label}\n{code}```"),
        PostFormat::Org => format!("#+begin_src {label}\n{code}#+end_src"),
        PostFormat::Html => unreachable!("HTML Posts do not syntax-highlight authored code"),
    };
    let body: PostBody = source.parse().expect("the fixture is a Post body");
    crate::render::render(&body, &format)
        .expect("a pinned highlighter is available")
        .to_string()
}

fn captured_bytes(language: tree_sitter::Language, query: &str, code: &str, role: &str) -> usize {
    use tree_sitter_highlight::{HighlightEvent, Highlighter};

    let configuration = super::config(language, "comparison", query, "", "").unwrap();
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(&configuration, code.as_bytes(), None, None, |_| None)
        .unwrap();
    let mut scopes = Vec::new();
    let mut covered = 0;
    for event in events {
        match event.unwrap() {
            HighlightEvent::HighlightStart(capture) => {
                scopes.push(super::CAPTURES.get(capture.0).copied().unwrap_or(""));
            }
            HighlightEvent::HighlightEnd => {
                scopes.pop().expect("balanced capture events");
            }
            HighlightEvent::Source { start, end } => {
                if scopes.contains(&role) {
                    covered += end - start;
                }
            }
        }
    }
    covered
}

#[test]
fn upstream_and_bundled_queries_expose_the_same_malformed_haskell_cascade() {
    let language: tree_sitter::Language = tree_sitter_haskell::LANGUAGE.into();
    for query in [
        tree_sitter_haskell::HIGHLIGHTS_QUERY,
        syntastica_queries::HASKELL_HIGHLIGHTS_CRATES_IO,
    ] {
        let directive_bytes = captured_bytes(language.clone(), query, HASKELL, "keyword.directive");
        assert!(
            directive_bytes > HASKELL.len() * 9 / 10,
            "the reference query did not reproduce the whole-document pragma capture"
        );
    }
    let html = render_code("haskell", HASKELL, PostFormat::Markdown);
    assert!(
        !html.contains("j-syn-keyword"),
        "integrated fallback must reject that capture"
    );
}

#[test]
fn upstream_elisp_query_omits_call_heads_but_the_reviewed_query_captures_them() {
    let language: tree_sitter::Language = tree_sitter_elisp::LANGUAGE.into();
    let original = captured_bytes(
        language.clone(),
        tree_sitter_elisp::HIGHLIGHTS_QUERY,
        ELISP,
        "function.call",
    );
    let revised = format!(
        "{}\n{}",
        tree_sitter_elisp::HIGHLIGHTS_QUERY,
        super::ELISP_CALL_QUERY
    );
    let candidate = captured_bytes(language, &revised, ELISP, "function.call");
    assert_eq!(
        original, 0,
        "upstream already distinguishes calls; reconsider the extension"
    );
    assert!(
        candidate >= "add-to-list".len(),
        "candidate loses call heads"
    );
}

#[test]
fn catalog_corpus_names_every_grammar_and_checks_reviewed_token_ranges() {
    let supported: std::collections::BTreeSet<_> = super::GRAMMARS
        .iter()
        .map(|grammar| grammar.labels[0])
        .chain(["elisp", "haskell"])
        .collect();
    let represented: std::collections::BTreeSet<_> =
        catalog::CASES.iter().map(|case| case.label).collect();
    assert_eq!(
        represented.len(),
        catalog::CASES.len(),
        "duplicate corpus labels"
    );
    assert_eq!(
        represented, supported,
        "a grammar is missing a quality oracle"
    );
    let mut failures = Vec::new();
    for case in catalog::CASES {
        assert!(
            !case.roles.is_empty(),
            "{} has no expected roles",
            case.label
        );
        for format in [PostFormat::Markdown, PostFormat::Org] {
            let html = render_code(case.label, case.code, format);
            let (decoded, roles) = semantic_code(&html);
            let plain_html = render_code("unknown-grammar", case.code, format);
            let (plain, _) = semantic_code(&plain_html);
            assert_eq!(
                decoded, plain,
                "{}/{format:?}: changed decoded source; highlighted={html}; plain={plain_html}",
                case.label
            );
            for &(token, role) in case.roles {
                assert!(
                    case.code.contains(token),
                    "{}/{format:?}: missing fixture token {token:?}",
                    case.label
                );
                let expected = format!("j-syn-{role}");
                let covered = decoded.match_indices(token).any(|(start, _)| {
                    roles[start..start + token.len()]
                        .iter()
                        .all(|active| active.contains(&expected.as_str()))
                });
                if !covered {
                    failures.push(format!(
                        "{}/{format:?}: expected {token:?} as {role}; HTML: {html}",
                        case.label
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn unknown_language_remains_escaped_and_uncolored_in_both_formats() {
    let code = "<script>alert('not executable')</script> & safe\n";
    for format in [PostFormat::Markdown, PostFormat::Org] {
        let html = render_code("not-a-grammar", code, format);
        let (decoded, roles) = semantic_code(&html);
        assert_eq!(decoded, code, "{format:?}: authored text changed");
        assert!(
            roles.iter().all(Vec::is_empty),
            "{format:?}: unknown language received a token role: {html}"
        );
        assert!(
            !html.contains("<script>"),
            "{format:?}: code must stay escaped: {html}"
        );
    }
}

#[test]
fn malformed_production_haskell_must_not_mark_entire_source_as_keyword() {
    // https://tendentious.org/~mdorman/2014/06/01/replacing-nothing-values-with-just-values-in-a-nested-structure
    for format in [PostFormat::Markdown, PostFormat::Org] {
        let html = render_code("haskell", HASKELL, format);
        assert!(
            !html.contains("<span class=\"j-syn-keyword\">import Control.Lens"),
            "{format:?}: a missing closing pragma delimiter must not color the import as an entire keyword line: {html}"
        );
        let plain = ammonia::Builder::empty().clean(&html).to_string();
        assert_eq!(
            html_escape::decode_html_entities(&plain).trim_end_matches('\n'),
            HASKELL.trim_end_matches('\n'),
            "{format:?}: fallback must preserve authored code (aside from the format's trailing LF)"
        );
    }
}

#[test]
fn well_formed_haskell_pragma_does_not_disable_highlighting() {
    let code = "{-# LANGUAGE OverloadedStrings #-}\nmodule Main where\nmain = putStrLn \"hello\"\n";
    for format in [PostFormat::Markdown, PostFormat::Org] {
        let html = render_code("haskell", code, format);
        assert!(html.contains("j-syn-keyword"), "{format:?}: {html}");
        assert!(html.contains("j-syn-string"), "{format:?}: {html}");
    }
}

#[test]
fn quoted_elisp_list_head_is_not_a_function_call() {
    for format in [PostFormat::Markdown, PostFormat::Org] {
        let html = render_code(
            "elisp",
            "(list '(not-a-call (still-not-call x)) (quote (also-not-a-call z)) (actual-call y))\n",
            format,
        );
        for symbol in ["not-a-call", "still-not-call", "also-not-a-call"] {
            assert!(
                !html.contains(&format!("class=\"j-syn-function-call\">{symbol}</span>")),
                "{format:?}: quoted data is not a call ({symbol}): {html}"
            );
        }
        assert!(
            html.contains("class=\"j-syn-function-call\">actual-call</span>"),
            "{format:?}: an unquoted list head is a call: {html}"
        );
    }
}

#[test]
fn elisp_quasiquoted_data_and_unquoted_expressions_keep_distinct_roles() {
    let code = "(list `(quoted-call ,(real-call 1)) (actual-call 2))\n";
    for format in [PostFormat::Markdown, PostFormat::Org] {
        let html = render_code("elisp", code, format);
        assert!(
            !html.contains("class=\"j-syn-function-call\">quoted-call</span>"),
            "{format:?}: quasiquoted data cannot be a call: {html}"
        );
        for call in ["real-call", "actual-call"] {
            assert!(
                html.contains(&format!("class=\"j-syn-function-call\">{call}</span>")),
                "{format:?}: unquoted expression must be a call ({call}): {html}"
            );
        }
    }
}

#[test]
fn elisp_nested_quasiquotes_only_activate_fully_unquoted_calls() {
    let code = "(list ``(data ,(still-quoted 1) ,,(live-call 2)) '(literal `(ignored ,(also-ignored 3))))\n";
    for format in [PostFormat::Markdown, PostFormat::Org] {
        let html = render_code("elisp", code, format);
        for symbol in ["still-quoted", "ignored", "also-ignored"] {
            assert!(
                !html.contains(&format!("class=\"j-syn-function-call\">{symbol}</span>")),
                "{format:?}: quoted data is not a call ({symbol}): {html}"
            );
        }
        assert!(
            html.contains("class=\"j-syn-function-call\">live-call</span>"),
            "{format:?}: a doubly unquoted head must be a call: {html}"
        );
    }
}

#[test]
fn production_elisp_marks_unquoted_call_heads_as_functions() {
    // https://tendentious.org/~mdorman/2014/12/28/how-i-would-start-out-with-emacs-now
    for format in [PostFormat::Markdown, PostFormat::Org] {
        let html = render_code("elisp", ELISP, format);
        for call in ["add-to-list", "package-list-packages"] {
            assert!(
                html.contains(&format!("class=\"j-syn-function-call\">{call}</span>")),
                "{format:?}: expected a function-call token for {call}: {html}"
            );
        }
    }
}
