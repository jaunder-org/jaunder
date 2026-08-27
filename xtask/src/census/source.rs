//! Centralized source-language policy for census collectors and LSP documents.
//!
//! This module classifies approved source paths, supplies stable report labels, and
//! selects LSP and parser language modes. Callers receive no language match
//! fallbacks: paths outside a language's declared surface are excluded, while
//! parse failures remain collector failures.

use oxc_span::SourceType;

use super::Language;

pub(crate) fn language_for_path(path: &str) -> Option<Language> {
    [Language::Rust, Language::TypeScript, Language::Elisp]
        .into_iter()
        .find(|language| language.matches_path(path))
}

impl Language {
    pub(crate) fn matches_path(self, path: &str) -> bool {
        match self {
            Self::Rust => path.ends_with(".rs"),
            Self::TypeScript => [".ts", ".tsx", ".js", ".jsx"]
                .iter()
                .any(|suffix| path.ends_with(suffix)),
            Self::Elisp => path.ends_with(".el"),
            Self::Repository => false,
        }
    }

    pub(crate) fn slug(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Elisp => "elisp",
            Self::Repository => "repository",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::TypeScript => "TypeScript",
            Self::Elisp => "Elisp",
            Self::Repository => "repository-wide",
        }
    }

    pub(crate) fn lsp_language_id(self, path: &str) -> Option<&'static str> {
        match (self, path.rsplit('.').next()) {
            (Self::Rust, Some("rs")) => Some("rust"),
            (Self::TypeScript, Some("ts")) => Some("typescript"),
            (Self::TypeScript, Some("tsx")) => Some("typescriptreact"),
            (Self::TypeScript, Some("js")) => Some("javascript"),
            (Self::TypeScript, Some("jsx")) => Some("javascriptreact"),
            _ => None,
        }
    }

    pub(crate) fn typescript_source_type(self, path: &str) -> Option<SourceType> {
        (self == Self::TypeScript)
            .then(|| SourceType::from_path(path).unwrap_or_else(|_| SourceType::ts()))
    }
}
