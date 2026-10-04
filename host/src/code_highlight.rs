//! Host-only, bounded Tree-sitter highlighting for authored Post code blocks.

use std::sync::{LazyLock, OnceLock};
use tree_sitter_highlight::{
    Highlight, HighlightConfiguration, HighlightEvent, Highlighter, HtmlRenderer,
};

const CAPTURES: &[&str] = &[
    "comment",
    "keyword",
    "string",
    "number",
    "function",
    "function.builtin",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "constant",
    "operator",
    "punctuation",
    "constructor",
    "module",
    "attribute",
    "attribute.builtin",
    "boolean",
    "character",
    "character.special",
    "constant.builtin",
    "constant.macro",
    "diff.minus",
    "diff.plus",
    "function.call",
    "function.macro",
    "function.method",
    "function.method.call",
    "interface",
    "keyword.conditional",
    "keyword.conditional.ternary",
    "keyword.coroutine",
    "keyword.debug",
    "keyword.directive",
    "keyword.directive.define",
    "keyword.exception",
    "keyword.function",
    "keyword.import",
    "keyword.modifier",
    "keyword.operator",
    "keyword.repeat",
    "keyword.return",
    "keyword.type",
    "label",
    "markup.heading",
    "markup.heading.1",
    "markup.heading.2",
    "markup.heading.3",
    "markup.heading.4",
    "markup.heading.5",
    "markup.heading.6",
    "markup.italic",
    "markup.link.label",
    "markup.link.url",
    "markup.list",
    "markup.list.checked",
    "markup.list.unchecked",
    "markup.quote",
    "markup.raw",
    "markup.raw.block",
    "markup.strikethrough",
    "markup.strong",
    "markup.underline",
    "module.builtin",
    "namespace",
    "none",
    "number.float",
    "operator",
    "property",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string.documentation",
    "string.escape",
    "string.regexp",
    "string.special",
    "string.special.path",
    "string.special.symbol",
    "string.special.url",
    "tag",
    "tag.attribute",
    "tag.builtin",
    "tag.delimiter",
    "type.definition",
    "variable.member",
    "variable.parameter",
    "variable.parameter.builtin",
];

const BLOCK_LIMIT: usize = 64 * 1024;
const POST_BYTES_LIMIT: usize = 128 * 1024;
const POST_ATTEMPTS_LIMIT: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum HighlightError {
    #[error("failed to initialize {language} highlighting: {detail}")]
    Initialization {
        language: &'static str,
        detail: String,
    },
    #[error("Tree-sitter highlighting failed: {0}")]
    Engine(#[from] tree_sitter_highlight::Error),
}

fn config(
    language: tree_sitter::Language,
    name: &'static str,
    highlights: &str,
    injections: &str,
    locals: &str,
) -> Result<HighlightConfiguration, HighlightError> {
    let mut configuration = HighlightConfiguration::new(
        language, name, highlights, injections, locals,
    )
    .map_err(|error| HighlightError::Initialization {
        language: name,
        detail: error.to_string(),
    })?;
    configuration.configure(CAPTURES);
    Ok(configuration)
}

#[cfg(any(test, feature = "test-utils"))]
tokio::task_local! {
    static INVALID_QUERY_FOR_TEST: ();
}

#[cfg(any(test, feature = "test-utils"))]
pub(crate) async fn with_invalid_query_for_test<F: std::future::Future>(future: F) -> F::Output {
    INVALID_QUERY_FOR_TEST.scope((), future).await
}

// Upstream Elisp colors definitions but omits unquoted call heads. Quote
// context needs separate structural filtering: query patterns cannot exclude
// arbitrarily nested quoted lists from this call-head capture.
const ELISP_CALL_QUERY: &str = "(list . (symbol) @function.call)";

static ELISP: LazyLock<Result<HighlightConfiguration, HighlightError>> = LazyLock::new(|| {
    let query = format!(
        "{}\n{ELISP_CALL_QUERY}",
        tree_sitter_elisp::HIGHLIGHTS_QUERY
    );
    config(tree_sitter_elisp::LANGUAGE.into(), "elisp", &query, "", "")
});

static HASKELL: LazyLock<Result<HighlightConfiguration, HighlightError>> = LazyLock::new(|| {
    // The upstream blanket `(variable) @type` masks ordinary variable/function
    // captures; retain the narrower actual `(name) @type` captures instead.
    let query = tree_sitter_haskell::HIGHLIGHTS_QUERY.replace("\n(variable) @type\n", "\n");
    config(
        tree_sitter_haskell::LANGUAGE.into(),
        "haskell",
        &query,
        tree_sitter_haskell::INJECTIONS_QUERY,
        tree_sitter_haskell::LOCALS_QUERY,
    )
});

// Grammar data and queries are statically linked. No runtime grammar acquisition,
// authored query, or browser-side highlighting is possible through this registry.
struct Grammar {
    name: &'static str,
    labels: &'static [&'static str],
    language: fn() -> tree_sitter::Language,
    highlights: &'static str,
    configuration: OnceLock<Result<HighlightConfiguration, HighlightError>>,
}

macro_rules! grammar {
    ($name:literal, $language:expr, $query:ident, [$($label:literal),+ $(,)?]) => {
        Grammar {
            name: $name,
            labels: &[$($label),+],
            language: || $language,
            highlights: syntastica_queries::$query,
            configuration: OnceLock::new(),
        }
    };
}

static GRAMMARS: [Grammar; 40] = [
    grammar!(
        "asm",
        tree_sitter_asm::LANGUAGE.into(),
        ASM_HIGHLIGHTS_CRATES_IO,
        ["asm", "s", "assembly"]
    ),
    grammar!(
        "bash",
        tree_sitter_bash::LANGUAGE.into(),
        BASH_HIGHLIGHTS_CRATES_IO,
        ["bash", "sh", "shell", "zsh"]
    ),
    grammar!(
        "c",
        tree_sitter_c::LANGUAGE.into(),
        C_HIGHLIGHTS_CRATES_IO,
        ["c", "h"]
    ),
    grammar!(
        "c-sharp",
        tree_sitter_c_sharp::LANGUAGE.into(),
        C_SHARP_HIGHLIGHTS_CRATES_IO,
        ["c-sharp", "csharp", "cs"]
    ),
    grammar!(
        "cmake",
        tree_sitter_cmake::LANGUAGE.into(),
        CMAKE_HIGHLIGHTS_CRATES_IO,
        ["cmake"]
    ),
    Grammar {
        name: "containerfile",
        labels: &["dockerfile", "containerfile", "docker"],
        language: || tree_sitter_containerfile::LANGUAGE.into(),
        highlights: tree_sitter_containerfile::HIGHLIGHTS_QUERY,
        configuration: OnceLock::new(),
    },
    grammar!(
        "cpp",
        tree_sitter_cpp::LANGUAGE.into(),
        CPP_HIGHLIGHTS_CRATES_IO,
        ["cpp", "c++", "cc", "cxx", "hpp", "h++"]
    ),
    grammar!(
        "css",
        tree_sitter_css::LANGUAGE.into(),
        CSS_HIGHLIGHTS_CRATES_IO,
        ["css"]
    ),
    Grammar {
        name: "dart",
        labels: &["dart"],
        language: || tree_sitter_dart::LANGUAGE.into(),
        highlights: tree_sitter_dart::HIGHLIGHTS_QUERY,
        configuration: OnceLock::new(),
    },
    grammar!(
        "diff",
        tree_sitter_diff::LANGUAGE.into(),
        DIFF_HIGHLIGHTS_CRATES_IO,
        ["diff", "patch"]
    ),
    grammar!(
        "elixir",
        tree_sitter_elixir::LANGUAGE.into(),
        ELIXIR_HIGHLIGHTS_CRATES_IO,
        ["elixir", "ex", "exs"]
    ),
    grammar!(
        "fish",
        tree_sitter_fish::language(),
        FISH_HIGHLIGHTS_CRATES_IO,
        ["fish"]
    ),
    grammar!(
        "gleam",
        tree_sitter_gleam::LANGUAGE.into(),
        GLEAM_HIGHLIGHTS_CRATES_IO,
        ["gleam"]
    ),
    grammar!(
        "go",
        tree_sitter_go::LANGUAGE.into(),
        GO_HIGHLIGHTS_CRATES_IO,
        ["go", "golang"]
    ),
    grammar!(
        "html",
        tree_sitter_html::LANGUAGE.into(),
        HTML_HIGHLIGHTS_CRATES_IO,
        ["html", "htm"]
    ),
    grammar!(
        "java",
        tree_sitter_java::LANGUAGE.into(),
        JAVA_HIGHLIGHTS_CRATES_IO,
        ["java"]
    ),
    grammar!(
        "javascript",
        tree_sitter_javascript::LANGUAGE.into(),
        JAVASCRIPT_HIGHLIGHTS_CRATES_IO,
        ["javascript", "js", "jsx", "mjs", "cjs"]
    ),
    grammar!(
        "json",
        tree_sitter_json::LANGUAGE.into(),
        JSON_HIGHLIGHTS_CRATES_IO,
        ["json"]
    ),
    grammar!(
        "julia",
        tree_sitter_julia::LANGUAGE.into(),
        JULIA_HIGHLIGHTS_CRATES_IO,
        ["julia", "jl"]
    ),
    Grammar {
        name: "kotlin",
        labels: &["kotlin", "kt", "kts"],
        language: || tree_sitter_kotlin_sg::LANGUAGE.into(),
        highlights: tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY,
        configuration: OnceLock::new(),
    },
    grammar!(
        "lua",
        tree_sitter_lua::LANGUAGE.into(),
        LUA_HIGHLIGHTS_CRATES_IO,
        ["lua"]
    ),
    grammar!(
        "make",
        tree_sitter_make::LANGUAGE.into(),
        MAKE_HIGHLIGHTS_CRATES_IO,
        ["make", "makefile", "mk"]
    ),
    grammar!(
        "markdown",
        tree_sitter_md::LANGUAGE.into(),
        MARKDOWN_HIGHLIGHTS_CRATES_IO,
        ["markdown", "md"]
    ),
    grammar!(
        "nix",
        tree_sitter_nix::LANGUAGE.into(),
        NIX_HIGHLIGHTS_CRATES_IO,
        ["nix"]
    ),
    grammar!(
        "ocaml",
        tree_sitter_ocaml::LANGUAGE_OCAML.into(),
        OCAML_HIGHLIGHTS_CRATES_IO,
        ["ocaml", "ml"]
    ),
    grammar!(
        "php",
        tree_sitter_php::LANGUAGE_PHP_ONLY.into(),
        PHP_HIGHLIGHTS_CRATES_IO,
        ["php"]
    ),
    grammar!(
        "python",
        tree_sitter_python::LANGUAGE.into(),
        PYTHON_HIGHLIGHTS_CRATES_IO,
        ["python", "py", "py3"]
    ),
    Grammar {
        name: "r",
        labels: &["r", "rscript"],
        language: || tree_sitter_r::LANGUAGE.into(),
        highlights: tree_sitter_r::HIGHLIGHTS_QUERY,
        configuration: OnceLock::new(),
    },
    grammar!(
        "ql",
        tree_sitter_ql::LANGUAGE.into(),
        QL_HIGHLIGHTS_CRATES_IO,
        ["ql", "codeql"]
    ),
    grammar!(
        "ruby",
        tree_sitter_ruby::LANGUAGE.into(),
        RUBY_HIGHLIGHTS_CRATES_IO,
        ["ruby", "rb"]
    ),
    grammar!(
        "rust",
        tree_sitter_rust::LANGUAGE.into(),
        RUST_HIGHLIGHTS_CRATES_IO,
        ["rust", "rs"]
    ),
    grammar!(
        "scala",
        tree_sitter_scala::LANGUAGE.into(),
        SCALA_HIGHLIGHTS_CRATES_IO,
        ["scala", "sc"]
    ),
    grammar!(
        "sql",
        tree_sitter_sequel::LANGUAGE.into(),
        SQL_HIGHLIGHTS_CRATES_IO,
        ["sql"]
    ),
    grammar!(
        "swift",
        tree_sitter_swift::LANGUAGE.into(),
        SWIFT_HIGHLIGHTS_CRATES_IO,
        ["swift"]
    ),
    grammar!(
        "toml",
        tree_sitter_toml_ng::LANGUAGE.into(),
        TOML_HIGHLIGHTS_CRATES_IO,
        ["toml"]
    ),
    grammar!(
        "typescript",
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        TYPESCRIPT_HIGHLIGHTS_CRATES_IO,
        ["typescript", "ts"]
    ),
    grammar!(
        "tsx",
        tree_sitter_typescript::LANGUAGE_TSX.into(),
        TSX_HIGHLIGHTS_CRATES_IO,
        ["tsx"]
    ),
    Grammar {
        name: "xml",
        labels: &["xml", "xhtml", "svg"],
        language: || tree_sitter_xml::LANGUAGE_XML.into(),
        highlights: tree_sitter_xml::XML_HIGHLIGHT_QUERY,
        configuration: OnceLock::new(),
    },
    grammar!(
        "yaml",
        tree_sitter_yaml::LANGUAGE.into(),
        YAML_HIGHLIGHTS_CRATES_IO,
        ["yaml", "yml"]
    ),
    grammar!(
        "zig",
        tree_sitter_zig::LANGUAGE.into(),
        ZIG_HIGHLIGHTS_CRATES_IO,
        ["zig"]
    ),
];

fn catalog(label: &str) -> Option<&'static Grammar> {
    (label.len() <= 128 && label.is_ascii()).then_some(())?;
    GRAMMARS.iter().find(|grammar| {
        grammar
            .labels
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(label))
    })
}

fn supported(
    label: &str,
) -> Option<(
    &'static str,
    &'static LazyLock<Result<HighlightConfiguration, HighlightError>>,
)> {
    if label.eq_ignore_ascii_case("elisp") || label.eq_ignore_ascii_case("emacs-lisp") {
        Some(("elisp", &ELISP))
    } else if label.eq_ignore_ascii_case("haskell") || label.eq_ignore_ascii_case("hs") {
        Some(("haskell", &HASKELL))
    } else {
        None
    }
}

fn configured<'a>(
    language: &'static str,
    configuration: Result<&'a HighlightConfiguration, &'a HighlightError>,
) -> Result<&'a HighlightConfiguration, HighlightError> {
    configuration.map_err(|error| HighlightError::Initialization {
        language,
        detail: error.to_string(),
    })
}

fn parse_elisp(code: &str) -> Result<tree_sitter::Tree, HighlightError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_elisp::LANGUAGE.into())
        .map_err(tree_sitter_highlight::Error::from)?;
    parser.parse(code, None).ok_or(HighlightError::Engine(
        tree_sitter_highlight::Error::Cancelled,
    ))
}

fn elisp_quoted_symbols(tree: &tree_sitter::Tree, code: &str) -> std::collections::HashSet<usize> {
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Mode {
        Evaluated,
        Quoted,
        Quasiquoted(u32),
    }

    // One bounded AST walk, rather than walking every call's ancestors in a
    // potentially deeply nested list. Unquotes reduce only the innermost
    // quasiquote depth; nothing inside a literal quote becomes a live call.
    let mut symbols = std::collections::HashSet::new();
    let mut pending = vec![(tree.root_node(), Mode::Evaluated)];
    while let Some((node, parent_mode)) = pending.pop() {
        let mode = if node.kind() == "special_form"
            && node.child(1).is_some_and(|child| child.kind() == "quote")
        {
            Mode::Quoted
        } else if node.kind() == "quote" {
            match (parent_mode, code.as_bytes().get(node.start_byte())) {
                (Mode::Quoted, _) => Mode::Quoted,
                (Mode::Quasiquoted(depth), Some(&b'`')) => Mode::Quasiquoted(depth + 1),
                (_, Some(&b'`')) => Mode::Quasiquoted(1),
                _ => Mode::Quoted,
            }
        } else if node.kind() == "unquote" {
            match parent_mode {
                Mode::Quasiquoted(1) => Mode::Evaluated,
                Mode::Quasiquoted(depth) => Mode::Quasiquoted(depth - 1),
                other => other,
            }
        } else {
            parent_mode
        };
        if node.kind() == "symbol" && mode != Mode::Evaluated {
            symbols.insert(node.start_byte());
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            pending.push((child, mode));
        }
    }
    symbols
}

fn preserve_renderer_line_ending(code: &str, mut output: String) -> String {
    // HtmlRenderer invents an LF on unterminated lines and normalizes CRLF
    // boundaries to LF. Restore the exporter's original line separator.
    if !code.ends_with('\n') && output.ends_with('\n') {
        output.pop();
    } else if code.ends_with("\r\n") && output.ends_with('\n') && !output.ends_with("\r\n") {
        output.insert(output.len() - 1, '\r');
    }
    output
}

fn has_unpaired_carriage_return(code: &str) -> bool {
    code.as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'\r' && pair[1] != b'\n')
        || code.ends_with('\r')
}

fn semantic_category(capture_name: &str) -> Option<&str> {
    match capture_name.split('.').next() {
        Some("function") if capture_name.rsplit('.').next() == Some("call") => {
            Some("function-call")
        }
        Some("diff") if capture_name == "diff.plus" => Some("diff-plus"),
        Some("diff") if capture_name == "diff.minus" => Some("diff-minus"),
        Some("markup") if capture_name.starts_with("markup.heading") => Some("heading"),
        Some("markup") if capture_name == "markup.quote" => Some("quote"),
        Some("module" | "constructor" | "namespace" | "interface") => Some("type"),
        Some("tag" | "import") => Some("keyword"),
        Some("attribute" | "property" | "label") => Some("variable"),
        Some("boolean") => Some("constant"),
        Some("character" | "markup" | "diff") => Some("string"),
        Some(
            category @ ("comment" | "keyword" | "string" | "number" | "function" | "type"
            | "variable" | "constant" | "operator" | "punctuation"),
        ) => Some(category),
        _ => None,
    }
}

fn authored_highlighted_lines(
    renderer: &HtmlRenderer,
    code: &str,
) -> Result<String, HighlightError> {
    let mut lines = renderer.lines();
    let mut output = String::new();
    for source_line in code.split_inclusive('\n') {
        let line = lines.next().ok_or(HighlightError::Engine(
            tree_sitter_highlight::Error::Unknown,
        ))?;
        output.push_str(&preserve_renderer_line_ending(source_line, line.to_owned()));
    }
    // Some queries emit a zero-width capture at EOF, creating a phantom
    // renderer line. Never append it to the authored source.
    Ok(output)
}

/// One budget shared by every eligible code block in document order.
#[derive(Default)]
pub(crate) struct HighlightBudget {
    attempted_bytes: usize,
    attempts: usize,
}

impl HighlightBudget {
    pub(crate) fn render(
        &mut self,
        label: &str,
        code: &str,
    ) -> Result<Option<String>, HighlightError> {
        let direct = supported(label);
        let bundled = if direct.is_none() {
            catalog(label)
        } else {
            None
        };
        if direct.is_none() && bundled.is_none() {
            return Ok(None);
        }
        if code.len() > BLOCK_LIMIT
            || self.attempts >= POST_ATTEMPTS_LIMIT
            || code.len() > POST_BYTES_LIMIT - self.attempted_bytes
        {
            return Ok(None);
        }
        self.attempts += 1;
        self.attempted_bytes += code.len();
        // The pinned Haskell grammar accepts an unterminated opening pragma as
        // one `pragma` node spanning the whole block. Its query then colors
        // every subsequent line as a keyword rather than recovering. A plain
        // block is less misleading; never modify the authored source text.
        if matches!(direct, Some(("haskell", _)))
            && code.trim_start().starts_with("{-#")
            && !code.contains("#-}")
        {
            return Ok(None);
        }
        // Test-only, task-scoped query corruption proves callers propagate an
        // initialization failure without changing the pinned production registry.
        #[cfg(any(test, feature = "test-utils"))]
        if INVALID_QUERY_FOR_TEST.try_with(|()| ()).is_ok() {
            config(
                tree_sitter_elisp::LANGUAGE.into(),
                "injected-invalid-query",
                "(this_node_cannot_exist) @keyword",
                "",
                "",
            )?;
        }
        let (language, configuration) = if let Some((language, configuration)) = direct {
            (language, configuration.as_ref())
        } else {
            let Some(grammar) = bundled else {
                unreachable!("unknown labels return before the resource budget");
            };
            (
                grammar.name,
                grammar
                    .configuration
                    .get_or_init(|| {
                        config(
                            (grammar.language)(),
                            grammar.name,
                            grammar.highlights,
                            "",
                            "",
                        )
                    })
                    .as_ref(),
            )
        };
        let configuration = configured(language, configuration)?;
        // A standalone carriage return is not a line separator the HTML
        // renderer can round-trip. Leave the whole block escaped and intact.
        if has_unpaired_carriage_return(code) {
            return Ok(None);
        }
        let quoted_elisp_symbols = if language == "elisp" {
            elisp_quoted_symbols(&parse_elisp(code)?, code)
        } else {
            std::collections::HashSet::new()
        };
        let mut highlighter = Highlighter::new();
        let events = highlighter.highlight(configuration, code.as_bytes(), None, None, |_| None)?;
        let mut source_offset = 0;
        let no_highlight = CAPTURES
            .iter()
            .position(|capture| *capture == "none")
            // cov:ignore-start: the fixed capture registry always includes `none`; no source or query can remove it at runtime.
            .ok_or_else(|| HighlightError::Initialization {
                language,
                detail: "closed capture registry is missing `none`".into(),
            })?;
        // cov:ignore-stop
        let events = events.map(|event| {
            event.map(|event| match event {
                HighlightEvent::Source { end, .. } => {
                    source_offset = end;
                    event
                }
                HighlightEvent::HighlightStart(Highlight(index))
                    if CAPTURES.get(index) == Some(&"function.call")
                        && quoted_elisp_symbols.contains(&source_offset) =>
                {
                    HighlightEvent::HighlightStart(Highlight(no_highlight))
                }
                _ => event,
            })
        });
        let mut renderer = HtmlRenderer::new();
        renderer.render(events, code.as_bytes(), &|capture, attrs| {
            let capture_name = CAPTURES.get(capture.0).copied().unwrap_or("");
            let Some(category) = semantic_category(capture_name) else {
                return;
            };
            attrs.extend_from_slice(format!("class=\"j-syn-{category}\"").as_bytes());
        })?;
        Ok(Some(authored_highlighted_lines(&renderer, code)?))
    }
}

#[cfg(test)]
#[path = "code_highlight/quality.rs"]
mod quality;

#[cfg(test)]
mod tests {
    use super::*;
    use common::{post_body::PostBody, render::PostFormat};

    #[test]
    fn every_bundled_grammar_has_a_working_query_and_preserves_source() {
        let code = "value = \"<tag> & é\"\n";
        for grammar in &GRAMMARS {
            let highlighted = HighlightBudget::default()
                .render(grammar.labels[0], code)
                .unwrap_or_else(|error| panic!("{}: {error}", grammar.name))
                .unwrap_or_else(|| panic!("{} has no alias", grammar.name));
            let plain = ammonia::Builder::empty().clean(&highlighted).to_string();
            assert_eq!(
                html_escape::decode_html_entities(&plain),
                code,
                "{} changed decoded code",
                grammar.name
            );
            for format in [PostFormat::Org, PostFormat::Markdown] {
                let source = match format {
                    PostFormat::Org => {
                        format!("#+begin_src {}\n{code}#+end_src", grammar.labels[0])
                    }
                    PostFormat::Markdown => format!("```{}\n{code}```", grammar.labels[0]),
                    PostFormat::Html => {
                        unreachable!("catalog smoke tests cover Org and Markdown only")
                    }
                };
                let body: PostBody = source.parse().unwrap();
                let html = crate::render::render(&body, &format)
                    .unwrap_or_else(|error| panic!("{}/{format:?}: {error}", grammar.name))
                    .to_string();
                assert!(
                    html.contains("<code"),
                    "{}/{format:?}: {html}",
                    grammar.name
                );
                assert!(
                    html.contains("value"),
                    "{}/{format:?}: {html}",
                    grammar.name
                );
            }
        }
    }

    #[test]
    fn bounded_renderer_keeps_exact_source_across_eof_and_line_ending_shapes() {
        for label in ["elisp", "haskell", "markdown", "python"] {
            for code in ["x", "x\n", "x\n\n", "\n", "x\r\n", "x\r\n\r\n"] {
                let highlighted = HighlightBudget::default()
                    .render(label, code)
                    .unwrap_or_else(|error| panic!("{label}: {error}"))
                    .expect("known language stays eligible");
                let (plain, _) =
                    quality::semantic_code(&format!("<pre><code>{highlighted}</code></pre>"));
                assert_eq!(plain, code, "{label}: changed the source shape {code:?}");
            }
        }
    }

    #[test]
    fn every_alias_renders_the_same_code_as_its_canonical_label_in_both_formats() {
        let code = "value = \"<tag> & é\"\n";
        for grammar in &GRAMMARS {
            let expected = HighlightBudget::default()
                .render(grammar.labels[0], code)
                .unwrap_or_else(|error| panic!("{}: {error}", grammar.name))
                .expect("each canonical label is supported");
            for &alias in grammar.labels {
                let actual = HighlightBudget::default()
                    .render(alias, code)
                    .unwrap_or_else(|error| panic!("{} alias {alias}: {error}", grammar.name))
                    .unwrap_or_else(|| panic!("{} alias {alias} was not recognized", grammar.name));
                assert_eq!(actual, expected, "{} alias {alias}", grammar.name);
                for format in [PostFormat::Org, PostFormat::Markdown] {
                    let source = match format {
                        PostFormat::Org => format!("#+begin_src {alias}\n{code}#+end_src"),
                        PostFormat::Markdown => format!("```{alias}\n{code}```"),
                        PostFormat::Html => unreachable!("catalog tests exclude HTML"),
                    };
                    let body: PostBody = source.parse().unwrap();
                    let html = crate::render::render(&body, &format)
                        .unwrap_or_else(|error| {
                            panic!("{} alias {alias}/{format:?}: {error}", grammar.name) // cov:ignore: pinned catalog aliases render successfully; this is diagnostic-only on a regression.
                        })
                        .to_string();
                    assert!(
                        html.contains("<code"),
                        "{} alias {alias}/{format:?}: {html}",
                        grammar.name
                    );
                    assert!(
                        html.contains("value"),
                        "{} alias {alias}/{format:?}: {html}",
                        grammar.name
                    );
                    let canonical_source = match format {
                        PostFormat::Org => {
                            format!("#+begin_src {}\n{code}#+end_src", grammar.labels[0])
                        }
                        PostFormat::Markdown => {
                            format!("```{}\n{code}```", grammar.labels[0])
                        }
                        PostFormat::Html => unreachable!("catalog tests exclude HTML"),
                    };
                    let canonical_body: PostBody = canonical_source.parse().unwrap();
                    let canonical_html = crate::render::render(&canonical_body, &format)
                        .unwrap_or_else(|error| panic!("{} canonical: {error}", grammar.name))
                        .to_string();
                    assert_eq!(
                        quality::semantic_code(&html),
                        quality::semantic_code(&canonical_html),
                        "{} alias {alias}/{format:?} changed exporter text or token roles",
                        grammar.name
                    );
                }
            }
        }
    }

    #[test]
    fn mainstream_languages_emit_semantic_tokens_in_both_formats() {
        for (label, code) in [
            ("rust", "fn greet() { println!(\"hello\"); }"),
            ("py", "def greet(name): return \"hello\" + name"),
            ("typescript", "const greeting: string = \"hello\";"),
            ("javascript", "const greeting = \"hello\";"),
            ("json", "{\"message\": \"hello\"}"),
            ("yaml", "message: \"hello\""),
            ("bash", "echo \"hello\""),
            ("cpp", "int main() { return 0; }"),
            ("nix", "{ greeting = \"hello\"; }"),
            ("swift", "let greeting = \"hello\""),
            ("kotlin", "val greeting = \"hello\""),
            ("elixir", "def hello, do: \"hello\""),
            ("zig", "const greeting = \"hello\";"),
            ("haskell", "greet name = \"hello\" ++ name"),
            ("elisp", "(message \"hello\")"),
        ] {
            for format in [PostFormat::Org, PostFormat::Markdown] {
                let source = match format {
                    PostFormat::Org => format!("#+begin_src {label}\n{code}\n#+end_src"),
                    PostFormat::Markdown => format!("```{label}\n{code}\n```"),
                    PostFormat::Html => {
                        unreachable!("catalog smoke tests cover Org and Markdown only")
                    }
                };
                let body: PostBody = source.parse().unwrap();
                let html = crate::render::render(&body, &format)
                    .unwrap_or_else(|error| panic!("{label}/{format:?}: {error}"))
                    .to_string();
                assert!(
                    html.contains("class=\"j-syn-"),
                    "{label}/{format:?}: {html}"
                );
            }
        }
    }

    #[test]
    fn configured_query_failure_retains_language_and_source_detail() {
        let failure = Err(HighlightError::Initialization {
            language: "fixture",
            detail: "unknown capture".to_owned(),
        });
        let mapped = configured("pinned-grammar", failure.as_ref())
            .err()
            .expect("invalid query is rejected");
        assert!(matches!(
            mapped,
            HighlightError::Initialization {
                language: "pinned-grammar",
                detail,
            } if detail.contains("unknown capture")
        ));
    }

    #[test]
    fn renderer_terminal_lf_is_removed_only_when_not_authored() {
        assert_eq!(preserve_renderer_line_ending("x", "x\n".into()), "x");
        assert_eq!(preserve_renderer_line_ending("x\n", "x\n".into()), "x\n");
        assert_eq!(preserve_renderer_line_ending("x", "x".into()), "x");
        assert_eq!(
            preserve_renderer_line_ending("x\r\n", "x\n".into()),
            "x\r\n"
        );
        assert_eq!(
            preserve_renderer_line_ending("x\r\n", "x\r\n".into()),
            "x\r\n"
        );
    }

    #[test]
    fn lone_carriage_return_falls_back_without_mutating_source() {
        let html = HighlightBudget::default()
            .render("python", "first\rsecond")
            .expect("the fallback is not an engine failure");
        assert!(html.is_none());
    }

    #[test]
    fn bundled_aliases_are_unambiguous_and_unknown_labels_fall_back() {
        let mut labels = std::collections::HashSet::new();
        for grammar in &GRAMMARS {
            for &label in grammar.labels {
                assert!(labels.insert(label), "duplicate alias: {label}");
                assert_eq!(catalog(label).unwrap().name, grammar.name);
                assert_eq!(
                    catalog(&label.to_ascii_uppercase()).unwrap().name,
                    grammar.name
                );
            }
        }
        assert!(
            HighlightBudget::default()
                .render("unlisted-grammar", "<tag>")
                .unwrap()
                .is_none()
        );
    }
}
