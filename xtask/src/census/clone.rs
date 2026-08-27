//! Parsed clone and repeated-shape collectors.
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

use oxc_allocator::Allocator;
use oxc_ast::ast::{ArrowFunctionExpression, Function};
use oxc_ast_visit::{Visit as OxcVisit, walk as oxc_walk};
use oxc_parser::{Parser, ParserReturn};
use oxc_span::Span;
use oxc_syntax::scope::ScopeFlags;
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::visit::Visit;

use super::common::{STRUCTURAL_VERSION, failed, files, structural};
use super::model::{Candidate, CollectorMetadata};
use super::{CellReport, CollectorContext, EvidenceMethod, Language, SignalFamily};

pub(crate) fn rust_clones(context: &CollectorContext) -> CellReport {
    clones(context, Language::Rust)
}

pub(crate) fn typescript_clones(context: &CollectorContext) -> CellReport {
    clones(context, Language::TypeScript)
}
pub(crate) fn elisp_clones(context: &CollectorContext) -> CellReport {
    clones_with_elisp_reader(context, &mut EmacsElispReader)
}

fn clones(context: &CollectorContext, language: Language) -> CellReport {
    clone_shapes(context, language, |path, source| match language {
        Language::Rust => rust_function_shapes(source),
        Language::TypeScript => typescript_function_shapes(path, source),
        Language::Elisp | Language::Repository => Ok(Vec::new()),
    })
}

fn clones_with_elisp_reader(
    context: &CollectorContext,
    reader: &mut impl ElispReader,
) -> CellReport {
    clone_shapes(context, Language::Elisp, |_, source| {
        reader.function_shapes(source)
    })
}

fn clone_shapes(
    context: &CollectorContext,
    language: Language,
    mut shapes_for: impl FnMut(&str, &str) -> Result<Vec<String>, CloneShapeError>,
) -> CellReport {
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for (path, source) in files(context, language) {
        let shapes = match shapes_for(path, source) {
            Ok(shapes) => shapes,
            Err(CloneShapeError::ReaderUnavailable) => {
                return CellReport::unavailable_with_collector(
                    SignalFamily::ClonesAndRepeatedTestShapes,
                    language,
                    CollectorMetadata {
                        identity: "census-elisp-structural".into(),
                        version: Some(STRUCTURAL_VERSION.into()),
                        evidence_method: EvidenceMethod::Structural,
                        limitation: "the declared Emacs reader is not available".into(),
                    },
                    "Emacs reader structural shape collector is not available",
                );
            }
            Err(CloneShapeError::Failed(error)) => {
                return failed(
                    SignalFamily::ClonesAndRepeatedTestShapes,
                    language,
                    &format!("census-{}-structural", language.slug()),
                    format!("{path}: {error}"),
                );
            }
        };
        for shape in shapes {
            groups.entry(shape).or_default().push(path.into());
        }
    }
    let candidates = groups
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(shape, paths)| Candidate {
            identity: format!("{}-shape:{:x}", language.slug(), stable_hash(&shape)),
            summary: format!(
                "repeated parsed {} function or test shape across {} files",
                language.slug(),
                paths.len()
            ),
            total_paths: paths.len(),
            paths,
        })
        .collect();
    structural(
        SignalFamily::ClonesAndRepeatedTestShapes,
        language,
        "normalized parsed function and test shapes are structural review candidates",
        candidates,
    )
}

enum CloneShapeError {
    ReaderUnavailable,
    Failed(String),
}

impl From<String> for CloneShapeError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

fn rust_function_shapes(source: &str) -> Result<Vec<String>, CloneShapeError> {
    struct Functions {
        shapes: Vec<String>,
    }
    impl<'ast> Visit<'ast> for Functions {
        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            self.shapes.push(normalize_rust_shape(quote::quote!(#item)));
            syn::visit::visit_item_fn(self, item);
        }
    }
    let file =
        syn::parse_file(source).map_err(|error| CloneShapeError::Failed(error.to_string()))?;
    let mut functions = Functions { shapes: Vec::new() };
    functions.visit_file(&file);
    Ok(functions.shapes)
}

fn typescript_function_shapes(path: &str, source: &str) -> Result<Vec<String>, CloneShapeError> {
    struct Functions<'s> {
        source: &'s str,
        shapes: Vec<String>,
    }
    impl<'a> OxcVisit<'a> for Functions<'_> {
        fn visit_function(&mut self, function: &Function<'a>, flags: ScopeFlags) {
            self.shapes
                .push(normalize_typescript_span(self.source, function.span));
            oxc_walk::walk_function(self, function, flags);
        }

        fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
            self.shapes
                .push(normalize_typescript_span(self.source, function.span));
            oxc_walk::walk_arrow_function_expression(self, function);
        }
    }

    let allocator = Allocator::default();
    let ParserReturn {
        program,
        diagnostics,
        panicked,
        ..
    } = Parser::new(
        &allocator,
        source,
        Language::TypeScript
            .typescript_source_type(path)
            .expect("TypeScript language has a parser mode"),
    )
    .parse();
    if panicked || !diagnostics.is_empty() {
        return Err(diagnostics
            .iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("; ")
            .into());
    }
    let mut functions = Functions {
        source,
        shapes: Vec::new(),
    };
    functions.visit_program(&program);
    Ok(functions.shapes)
}

fn normalize_typescript_span(source: &str, span: Span) -> String {
    normalize_shape(&source[span.start as usize..span.end as usize])
}

trait ElispReader {
    fn function_shapes(&mut self, source: &str) -> Result<Vec<String>, CloneShapeError>;
}

struct EmacsElispReader;

impl ElispReader for EmacsElispReader {
    fn function_shapes(&mut self, source: &str) -> Result<Vec<String>, CloneShapeError> {
        const READER: &str = r#"(progn
(defun census-normalize (value)
  (cond ((consp value) (cons (census-normalize (car value))
                              (census-normalize (cdr value))))
        ((symbolp value) 'id)
        ((numberp value) 'number)
        ((stringp value) 'string)
        (t 'literal)))
(with-temp-buffer
  (insert-file-contents "/dev/stdin")
  (emacs-lisp-mode)
  (check-parens)
  (goto-char (point-min))
  (condition-case nil
      (while t
        (let ((form (read (current-buffer))))
          (when (memq (car-safe form) '(defun ert-deftest))
            (princ (prin1-to-string (census-normalize form)))
            (princ "\n"))))
    (end-of-file nil))))"#;
        let mut reader = Command::new("emacs")
            .args(["--batch", "--quick", "--eval", READER])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    CloneShapeError::ReaderUnavailable
                } else {
                    CloneShapeError::Failed(error.to_string())
                }
            })?;
        reader
            .stdin
            .take()
            .expect("configured piped stdin")
            .write_all(source.as_bytes())
            .map_err(|error| CloneShapeError::Failed(error.to_string()))?;
        let output = reader
            .wait_with_output()
            .map_err(|error| CloneShapeError::Failed(error.to_string()))?;
        if !output.status.success() {
            return Err(CloneShapeError::Failed(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect())
    }
}

fn normalize_rust_shape(tokens: TokenStream) -> String {
    fn tokens_into(out: &mut String, tokens: TokenStream) {
        for token in tokens {
            match token {
                TokenTree::Group(group) => {
                    let (open, close) = match group.delimiter() {
                        Delimiter::Parenthesis => ('(', ')'),
                        Delimiter::Brace => ('{', '}'),
                        Delimiter::Bracket => ('[', ']'),
                        Delimiter::None => ('<', '>'),
                    };
                    out.push(open);
                    tokens_into(out, group.stream());
                    out.push(close);
                }
                TokenTree::Ident(_) => out.push_str("id"),
                TokenTree::Punct(punctuation) => out.push(punctuation.as_char()),
                TokenTree::Literal(_) => out.push('#'),
            }
        }
    }

    let mut out = String::new();
    tokens_into(&mut out, tokens);
    out
}

fn normalize_shape(source: &str) -> String {
    enum State {
        Code,
        LineComment,
        BlockComment(usize),
        String(char),
    }
    let mut state = State::Code;
    let mut out = String::new();
    let mut token = String::new();
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        match &mut state {
            State::Code if character == '/' && characters.peek() == Some(&'/') => {
                characters.next();
                state = State::LineComment;
            }
            State::Code if character == '/' && characters.peek() == Some(&'*') => {
                characters.next();
                state = State::BlockComment(1);
            }
            State::Code if matches!(character, '"' | '\'' | '`') => {
                normalize_token(&mut out, &mut token);
                out.push_str("str");
                state = State::String(character);
            }
            State::Code if character.is_ascii_alphanumeric() || character == '_' => {
                token.push(character)
            }
            State::Code => {
                normalize_token(&mut out, &mut token);
                if !character.is_whitespace() {
                    out.push(character);
                }
            }
            State::LineComment if character == '\n' => state = State::Code,
            State::BlockComment(depth) if character == '*' && characters.peek() == Some(&'/') => {
                characters.next();
                *depth -= 1;
                if *depth == 0 {
                    state = State::Code;
                }
            }
            State::String(_) if character == '\\' => {
                characters.next();
            }
            State::String(delimiter) if character == *delimiter => state = State::Code,
            _ => {}
        }
    }
    normalize_token(&mut out, &mut token);
    out
}

fn normalize_token(out: &mut String, token: &mut String) {
    if token.is_empty() {
        return;
    }
    out.push_str(
        if token
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            "#"
        } else {
            "id"
        },
    );
    token.clear();
}
fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}
#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::super::common::context;
    use super::super::{CellState, EvidenceMethod};
    use super::*;
    #[test]
    fn clone_positive_and_negative() {
        assert!(matches!(
            rust_clones(&context(&[
                ("a.rs", "fn a(){let x=1; x+1;}"),
                ("b.rs", "fn b(){let y=2; y+2;}")
            ]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            rust_clones(&context(&[
                ("a.rs", "fn a(){}"),
                ("b.rs", "fn b(){let y=2;}")
            ]))
            .state,
            CellState::Clean
        ));
    }
    #[test]
    fn clone_collectors_use_parsed_function_shapes_and_ignore_markers_in_comments_and_strings() {
        let typescript = typescript_clones(&context(&[
            (
                "a.ts",
                "function alpha() { /* CLONE-A */ return \"CLONE-A\"; }",
            ),
            (
                "b.ts",
                "function beta() { /* CLONE-B */ return \"CLONE-B\"; }",
            ),
        ]));
        let CellState::Candidates { candidates, .. } = typescript.state else {
            panic!("parsed TypeScript functions with equivalent structure are candidates");
        };
        assert_eq!(candidates[0].paths, ["a.ts", "b.ts"]);
        assert_eq!(
            typescript.collector.evidence_method,
            EvidenceMethod::Structural
        );

        let clean = typescript_clones(&context(&[
            ("a.ts", "function alpha() { return 1; }"),
            ("b.ts", "function beta() { return 1; return 2; }"),
        ]));
        assert!(matches!(clean.state, CellState::Clean));

        let malformed = typescript_clones(&context(&[("a.ts", "function broken( {")]));
        assert!(matches!(malformed.state, CellState::Failed { .. }));
    }

    struct FixtureReader {
        responses: VecDeque<Result<Vec<String>, CloneShapeError>>,
    }

    impl ElispReader for FixtureReader {
        fn function_shapes(&mut self, _: &str) -> Result<Vec<String>, CloneShapeError> {
            self.responses
                .pop_front()
                .expect("one fixture response per source")
        }
    }

    #[test]
    fn elisp_clone_forms_are_testable_without_emacs() {
        let mut reader = FixtureReader {
            responses: VecDeque::from([
                Ok(vec!["(defun id () (id string))".into()]),
                Ok(vec!["(defun id () (id string))".into()]),
            ]),
        };
        assert!(matches!(
            clones_with_elisp_reader(
                &context(&[("a.el", "(defun a ())"), ("b.el", "(defun b ())")]),
                &mut reader,
            )
            .state,
            CellState::Candidates { .. }
        ));

        let mut clean_reader = FixtureReader {
            responses: VecDeque::from([
                Ok(vec!["(defun id () literal)".into()]),
                Ok(vec!["(ert-deftest id () literal)".into()]),
            ]),
        };
        assert!(matches!(
            clones_with_elisp_reader(
                &context(&[("a.el", "(defun a ())"), ("b.el", "(ert-deftest b ())")]),
                &mut clean_reader,
            )
            .state,
            CellState::Clean
        ));
    }

    #[test]
    fn unavailable_elisp_reader_remains_unavailable() {
        let mut reader = FixtureReader {
            responses: VecDeque::from([Err(CloneShapeError::ReaderUnavailable)]),
        };
        assert!(matches!(
            clones_with_elisp_reader(&context(&[("a.el", "(defun a ())")]), &mut reader).state,
            CellState::Unavailable { .. }
        ));
    }
}
