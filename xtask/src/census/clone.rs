//! Parsed structural clone and repeated-shape collectors.
//!
//! This module selects parsed Rust and TypeScript functions and Emacs-read Elisp
//! definitions/tests, then compares normalized syntax shapes across tracked
//! files. Its candidates are structural prompts rather than proof of duplicated
//! behavior. Parser and reader failures remain failed cells; a missing Emacs
//! reader is unavailable. The owned Emacs process drains stderr at spawn and is
//! always terminated and reaped on its error path, with cleanup warnings never
//! replacing reader evidence.

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrowFunctionExpression, BigIntLiteral, BindingIdentifier, BooleanLiteral, Function,
    IdentifierName, IdentifierReference, LabelIdentifier, NullLiteral, NumericLiteral,
    PrivateIdentifier, RegExpLiteral, StringLiteral, TemplateElement,
};
use oxc_ast_visit::{Visit as OxcVisit, walk as oxc_walk};
use oxc_parser::{Parser, ParserReturn};
use oxc_span::Span;
use oxc_syntax::scope::ScopeFlags;
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::visit::Visit;

use super::common::{STRUCTURAL_VERSION, failed, files, structural};
use super::model::{Candidate, CollectorMetadata};
use super::process::{StderrDrain, StdoutDrain, terminate_and_reap};
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
                .push(normalize_typescript_function(self.source, function, flags));
            oxc_walk::walk_function(self, function, flags);
        }

        fn visit_arrow_function_expression(&mut self, function: &ArrowFunctionExpression<'a>) {
            self.shapes
                .push(normalize_typescript_arrow_function(self.source, function));
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

fn normalize_typescript_function(
    source: &str,
    function: &Function<'_>,
    flags: ScopeFlags,
) -> String {
    let mut roles = TypeScriptRoles::default();
    roles.visit_function(function, flags);
    normalize_typescript_span(source, function.span, roles.spans)
}

fn normalize_typescript_arrow_function(
    source: &str,
    function: &ArrowFunctionExpression<'_>,
) -> String {
    let mut roles = TypeScriptRoles::default();
    roles.visit_arrow_function_expression(function);
    normalize_typescript_span(source, function.span, roles.spans)
}

#[derive(Default)]
struct TypeScriptRoles {
    spans: Vec<(Span, &'static str)>,
}

impl TypeScriptRoles {
    fn identifier(&mut self, span: Span) {
        self.spans.push((span, "id"));
    }

    fn literal(&mut self, span: Span) {
        self.spans.push((span, "literal"));
    }
}

impl<'a> OxcVisit<'a> for TypeScriptRoles {
    fn visit_identifier_name(&mut self, identifier: &IdentifierName<'a>) {
        self.identifier(identifier.span);
        oxc_walk::walk_identifier_name(self, identifier);
    }

    fn visit_identifier_reference(&mut self, identifier: &IdentifierReference<'a>) {
        self.identifier(identifier.span);
        oxc_walk::walk_identifier_reference(self, identifier);
    }

    fn visit_binding_identifier(&mut self, identifier: &BindingIdentifier<'a>) {
        self.identifier(identifier.span);
        oxc_walk::walk_binding_identifier(self, identifier);
    }

    fn visit_label_identifier(&mut self, identifier: &LabelIdentifier<'a>) {
        self.identifier(identifier.span);
        oxc_walk::walk_label_identifier(self, identifier);
    }

    fn visit_null_literal(&mut self, literal: &NullLiteral) {
        self.literal(literal.span);
        oxc_walk::walk_null_literal(self, literal);
    }
    fn visit_private_identifier(&mut self, identifier: &PrivateIdentifier<'a>) {
        self.identifier(identifier.span);
        oxc_walk::walk_private_identifier(self, identifier);
    }

    fn visit_boolean_literal(&mut self, literal: &BooleanLiteral) {
        self.literal(literal.span);
        oxc_walk::walk_boolean_literal(self, literal);
    }

    fn visit_numeric_literal(&mut self, literal: &NumericLiteral<'a>) {
        self.literal(literal.span);
        oxc_walk::walk_numeric_literal(self, literal);
    }

    fn visit_string_literal(&mut self, literal: &StringLiteral<'a>) {
        self.literal(literal.span);
        oxc_walk::walk_string_literal(self, literal);
    }

    fn visit_big_int_literal(&mut self, literal: &BigIntLiteral<'a>) {
        self.literal(literal.span);
        oxc_walk::walk_big_int_literal(self, literal);
    }

    fn visit_reg_exp_literal(&mut self, literal: &RegExpLiteral<'a>) {
        self.literal(literal.span);
        oxc_walk::walk_reg_exp_literal(self, literal);
    }

    fn visit_template_element(&mut self, literal: &TemplateElement<'a>) {
        self.literal(literal.span);
        oxc_walk::walk_template_element(self, literal);
    }
}

fn normalize_typescript_span(
    source: &str,
    span: Span,
    mut roles: Vec<(Span, &'static str)>,
) -> String {
    let start = span.start as usize;
    let end = span.end as usize;
    roles.sort_unstable_by_key(|(span, _)| span.start);
    let mut cursor = start;
    let mut normalized = String::new();
    for (role, replacement) in roles {
        let role_start = role.start as usize;
        let role_end = role.end as usize;
        if role_start < cursor || role_end > end {
            continue;
        }
        normalized.push_str(&normalize_typescript_syntax(&source[cursor..role_start]));
        normalized.push_str(replacement);
        cursor = role_end;
    }
    normalized.push_str(&normalize_typescript_syntax(&source[cursor..end]));
    normalized
}

trait ElispReader {
    fn function_shapes(&mut self, source: &str) -> Result<Vec<String>, CloneShapeError>;
}

struct EmacsElispReader;

impl ElispReader for EmacsElispReader {
    fn function_shapes(&mut self, source: &str) -> Result<Vec<String>, CloneShapeError> {
        const READER: &str = r#"(progn
(defun census-normalize-tail (value)
  (cond ((consp value) (cons (census-normalize (car value))
                              (census-normalize-tail (cdr value))))
        ((null value) nil)
        (t (census-normalize value))))
(defun census-normalize (value &optional head)
  (cond ((consp value) (cons (census-normalize (car value) t)
                              (census-normalize-tail (cdr value))))
        ((symbolp value) (if head value 'id))
        ((numberp value) 'number)
        ((stringp value) 'string)
        (t 'literal)))
(defun census-normalize-definition (form)
  (append (list (car form)
                'id
                (mapcar (lambda (_argument) 'id) (nth 2 form)))
          (mapcar #'census-normalize (cdddr form))))
(with-temp-buffer
  (insert-file-contents "/dev/stdin")
  (emacs-lisp-mode)
  (check-parens)
  (goto-char (point-min))
  (condition-case nil
      (while t
        (let ((form (read (current-buffer))))
          (when (memq (car-safe form) '(defun ert-deftest))
            (princ (prin1-to-string (census-normalize-definition form)))
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
        let stderr = match reader.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_and_reap(&mut reader, "Emacs reader");
                return Err(CloneShapeError::Failed(
                    "Emacs reader stderr was not piped".into(),
                ));
            }
        };
        let mut stderr = StderrDrain::start(stderr);
        let stdout = match reader.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_and_reap(&mut reader, "Emacs reader");
                let diagnostics = stderr.finish("Emacs reader");
                return Err(CloneShapeError::Failed(with_diagnostics(
                    "Emacs reader stdout was not piped".into(),
                    diagnostics,
                )));
            }
        };
        let mut stdout = StdoutDrain::start(stdout);
        let write_result = match reader.stdin.take() {
            Some(mut stdin) => stdin.write_all(source.as_bytes()),
            None => Err(std::io::Error::other("Emacs reader stdin was not piped")),
        };
        if let Err(error) = write_result {
            terminate_and_reap(&mut reader, "Emacs reader");
            finish_stdout_after_failure(&mut stdout);
            let diagnostics = stderr.finish("Emacs reader");
            return Err(CloneShapeError::Failed(with_diagnostics(
                error.to_string(),
                diagnostics,
            )));
        }
        let status = match reader.wait() {
            Ok(status) => status,
            Err(error) => {
                terminate_and_reap(&mut reader, "Emacs reader");
                finish_stdout_after_failure(&mut stdout);
                let diagnostics = stderr.finish("Emacs reader");
                return Err(CloneShapeError::Failed(with_diagnostics(
                    error.to_string(),
                    diagnostics,
                )));
            }
        };
        let output = match stdout.finish() {
            Ok(output) => output,
            Err(error) => {
                terminate_and_reap(&mut reader, "Emacs reader");
                let diagnostics = stderr.finish("Emacs reader");
                return Err(CloneShapeError::Failed(with_diagnostics(
                    error,
                    diagnostics,
                )));
            }
        };
        let diagnostics = stderr.finish("Emacs reader");
        if !status.success() {
            return Err(CloneShapeError::Failed(with_diagnostics(
                format!("Emacs reader exited with {status}"),
                diagnostics,
            )));
        }
        Ok(String::from_utf8_lossy(&output)
            .lines()
            .map(str::to_owned)
            .collect())
    }
}

fn finish_stdout_after_failure(stdout: &mut StdoutDrain) {
    if let Err(error) = stdout.finish() {
        eprintln!("warning: draining census Emacs reader stdout failed: {error}");
    }
}

fn with_diagnostics(error: String, diagnostics: String) -> String {
    if diagnostics.is_empty() {
        error
    } else {
        format!("{error}; Emacs stderr: {diagnostics}")
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

fn normalize_typescript_syntax(source: &str) -> String {
    enum State {
        Code,
        LineComment,
        BlockComment,
    }

    let mut state = State::Code;
    let mut normalized = String::new();
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        match state {
            State::Code if character == '/' && characters.peek() == Some(&'/') => {
                characters.next();
                state = State::LineComment;
            }
            State::Code if character == '/' && characters.peek() == Some(&'*') => {
                characters.next();
                state = State::BlockComment;
            }
            State::Code => {
                if !character.is_whitespace() {
                    normalized.push(character);
                }
            }
            State::LineComment if character == '\n' => state = State::Code,
            State::BlockComment if character == '*' && characters.peek() == Some(&'/') => {
                characters.next();
                state = State::Code;
            }
            _ => {}
        }
    }
    normalized
}
fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}
#[cfg(test)]
mod tests {

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
    fn typescript_normalization_uses_identifier_and_literal_roles() {
        let clones = typescript_clones(&context(&[
            (
                "a.ts",
                "function get(from: string) { const of = true; return { set: from, type: null, module: `a ${of}`, undefined: 1n }; }",
            ),
            (
                "b.ts",
                "function setter(value: string) { const count = false; return { field: value, kind: null, namespace: `b ${count}`, absent: 2n }; }",
            ),
        ]));
        assert!(matches!(clones.state, CellState::Candidates { .. }));

        let syntax_distinct = typescript_clones(&context(&[
            ("a.ts", "function alpha(x) { if (x) return 1; }"),
            ("b.ts", "function beta(y) { while (y) throw 2; }"),
        ]));
        assert!(matches!(syntax_distinct.state, CellState::Clean));
    }

    #[test]
    fn elisp_normalization_uses_actual_reader_and_preserves_heads() {
        let mut reader = EmacsElispReader;
        let clones = clones_with_elisp_reader(
            &context(&[
                ("a.el", "(defun alpha (x) (+ x 1))"),
                ("b.el", "(defun beta (y) (+ y 2))"),
            ]),
            &mut reader,
        );
        assert!(matches!(clones.state, CellState::Candidates { .. }));

        let clean = clones_with_elisp_reader(
            &context(&[
                ("a.el", "(defun alpha (x) (+ x 1))"),
                ("b.el", "(defun beta (y) (- y 2))"),
                ("c.el", "(defun gamma (z) (if z 1 2))"),
                ("d.el", "(defun delta (w) (while w (let ((n 3)) n)))"),
                ("e.el", "(defun epsilon (a b) (= a b))"),
                ("f.el", "(defun zeta (c d) (equal c d))"),
                ("g.el", "(defun eta (q) (custom-call q 4))"),
                ("h.el", "(defun theta (r) (other-call r 5))"),
            ]),
            &mut reader,
        );
        assert!(matches!(clean.state, CellState::Clean));
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

        let syntax_distinct = typescript_clones(&context(&[
            ("a.ts", "function alpha(x) { if (x) return 1; }"),
            ("b.ts", "function beta(y) { while (y) throw 2; }"),
        ]));
        assert!(matches!(syntax_distinct.state, CellState::Clean));
    }

    struct UnavailableReader;

    impl ElispReader for UnavailableReader {
        fn function_shapes(&mut self, _: &str) -> Result<Vec<String>, CloneShapeError> {
            Err(CloneShapeError::ReaderUnavailable)
        }
    }

    #[test]
    fn unavailable_elisp_reader_remains_unavailable() {
        let mut reader = UnavailableReader;
        assert!(matches!(
            clones_with_elisp_reader(&context(&[("a.el", "(defun a ())")]), &mut reader).state,
            CellState::Unavailable { .. }
        ));
    }
}
