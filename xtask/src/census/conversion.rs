//! Parsed conversion and error-mapping collectors.
//!
//! Candidates require both a conversion operation and its error-flow relationship;
//! comments, formatting, and unrelated expressions are not evidence.

use oxc_allocator::Allocator;
use oxc_ast::ast::{TSAsExpression, ThrowStatement, TryStatement};
use oxc_ast_visit::{Visit as OxcVisit, walk as oxc_walk};
use oxc_parser::{Parser, ParserReturn};
use syn::visit::Visit;

use super::common::{failed, files, structural};
use super::model::Candidate;
use super::{CellReport, CollectorContext, Language, SignalFamily};

pub(crate) fn rust_conversions(context: &CollectorContext) -> CellReport {
    conversion_sequences(context, Language::Rust)
}

pub(crate) fn typescript_conversions(context: &CollectorContext) -> CellReport {
    conversion_sequences(context, Language::TypeScript)
}

fn conversion_sequences(context: &CollectorContext, language: Language) -> CellReport {
    let mut candidates = Vec::new();
    for (path, source) in files(context, language) {
        let found = match language {
            Language::Rust => rust_conversion_sequence(source),
            Language::TypeScript => typescript_conversion_sequence(path, source),
            Language::Elisp | Language::Repository => Ok(false),
        };
        let found = match found {
            Ok(found) => found,
            Err(error) => {
                return failed(
                    SignalFamily::ConversionAndErrorMapping,
                    language,
                    &format!("census-{}-structural", language.slug()),
                    format!("{path}: {error}"),
                );
            }
        };
        if found {
            candidates.push(Candidate {
                identity: format!("{}-conversion-error:{path}", language.slug()),
                summary: format!(
                    "parsed {} conversion and error-mapping sequence",
                    language.slug()
                ),
                paths: vec![path.into()],
                total_paths: 1,
            });
        }
    }
    structural(
        SignalFamily::ConversionAndErrorMapping,
        language,
        "parsed conversion and error-flow relationships are structural review candidates; runtime error equivalence is not inferred",
        candidates,
    )
}

fn rust_conversion_sequence(source: &str) -> Result<bool, String> {
    struct ConversionArgument {
        found: bool,
    }

    impl<'ast> Visit<'ast> for ConversionArgument {
        fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
            self.found |= matches!(
                expression.method.to_string().as_str(),
                "into" | "parse" | "to_string"
            );
            syn::visit::visit_expr_method_call(self, expression);
        }

        fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
            self.found |= expression
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "from");
            syn::visit::visit_expr_path(self, expression);
        }
    }

    struct Sequences {
        found: bool,
        in_try: bool,
    }

    impl<'ast> Visit<'ast> for Sequences {
        fn visit_expr_try(&mut self, expression: &'ast syn::ExprTry) {
            let previous = self.in_try;
            self.in_try = true;
            syn::visit::visit_expr_try(self, expression);
            self.in_try = previous;
        }

        fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
            if self.in_try && expression.method == "map_err" {
                let mut conversion = ConversionArgument { found: false };
                for argument in &expression.args {
                    conversion.visit_expr(argument);
                }
                self.found |= conversion.found;
            }
            syn::visit::visit_expr_method_call(self, expression);
        }
    }

    let file = syn::parse_file(source).map_err(|error| error.to_string())?;
    let mut sequences = Sequences {
        found: false,
        in_try: false,
    };
    sequences.visit_file(&file);
    Ok(sequences.found)
}

fn typescript_conversion_sequence(path: &str, source: &str) -> Result<bool, String> {
    struct AssertionVisitor {
        found: bool,
    }

    impl<'ast> OxcVisit<'ast> for AssertionVisitor {
        fn visit_ts_as_expression(&mut self, expression: &TSAsExpression<'ast>) {
            self.found = true;
            oxc_walk::walk_ts_as_expression(self, expression);
        }
    }

    struct ThrowVisitor {
        found: bool,
    }

    impl<'ast> OxcVisit<'ast> for ThrowVisitor {
        fn visit_throw_statement(&mut self, statement: &ThrowStatement<'ast>) {
            self.found = true;
            oxc_walk::walk_throw_statement(self, statement);
        }
    }

    struct Sequences {
        found: bool,
    }

    impl<'ast> OxcVisit<'ast> for Sequences {
        fn visit_try_statement(&mut self, statement: &TryStatement<'ast>) {
            let mut assertions = AssertionVisitor { found: false };
            assertions.visit_block_statement(&statement.block);
            let mut throws = ThrowVisitor { found: false };
            if let Some(handler) = &statement.handler {
                throws.visit_block_statement(&handler.body);
            }
            self.found |= assertions.found && throws.found;
            oxc_walk::walk_try_statement(self, statement);
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
            .join("; "));
    }
    let mut sequences = Sequences { found: false };
    sequences.visit_program(&program);
    Ok(sequences.found)
}

#[cfg(test)]
mod tests {
    use super::super::CellState;
    use super::super::common::context;
    use super::*;

    #[test]
    fn rust_conversion_sequence_requires_a_parsed_conversion_in_mapped_try_flow() {
        assert!(matches!(
            rust_conversions(&context(&[(
                "a.rs",
                "fn a() -> Result<(), E> { value.map_err(E::from)?; Ok(()) }"
            )]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            rust_conversions(&context(&[(
                "a.rs",
                "// .map_err(E::from)?\nfn a() { let label = \".map_err(E::from)?\"; }"
            )]))
            .state,
            CellState::Clean
        ));
    }

    #[test]
    fn typescript_conversion_sequence_requires_assertion_and_catch_throw_relationship() {
        assert!(matches!(
            typescript_conversions(&context(&[(
                "a.ts",
                "function a() { try { const value = input as string; return value; } catch (error) { throw error; } }"
            )]))
            .state,
            CellState::Candidates { .. }
        ));
        assert!(matches!(
            typescript_conversions(&context(&[(
                "a.ts",
                "const note = 'catch (error) { throw error; } as string'; function a() { return input as string; }"
            )]))
            .state,
            CellState::Clean
        ));
    }
}
