//! Minimal reader-aware top-level form discovery.
//!
//! We do not need to evaluate Elisp, but a line splitter is unsafe: comments,
//! strings, quoted forms, and nested reader structure can all contain parens.
//! This reader consumes complete forms and rejects unterminated structure so a
//! malformed current source cannot make a producer census appear complete.

use std::{fs, path::Path};

use super::model::{CoverageError, FormCensus};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SourceForm {
    pub start_line: u32,
    pub kind: String,
}

pub(crate) fn read_forms(path: &Path) -> Result<(String, Vec<SourceForm>), CoverageError> {
    let text = fs::read_to_string(path).map_err(|error| CoverageError::Source {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    let mut reader = Reader::new(&text, path);
    let mut forms = Vec::new();
    while reader.skip_space_and_comments()? {
        forms.push(reader.form()?);
    }
    Ok((text, forms))
}

pub(crate) fn assert_forms(path: &Path, census: &[FormCensus]) -> Result<String, CoverageError> {
    let (text, forms) = read_forms(path)?;
    let expected: Vec<_> = forms
        .into_iter()
        .map(|form| (form.start_line, form.kind))
        .collect();
    let actual: Vec<_> = census
        .iter()
        .map(|form| (form.start_line, form.kind.clone()))
        .collect();
    if actual != expected {
        return Err(CoverageError::Census {
            message: format!(
                "{} census forms do not match current source: expected {expected:?}, got {actual:?}",
                path.display()
            ),
        });
    }
    Ok(text)
}

struct Reader<'a> {
    text: &'a str,
    bytes: &'a [u8],
    index: usize,
    line: u32,
    path: &'a Path,
}

impl<'a> Reader<'a> {
    fn new(text: &'a str, path: &'a Path) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            index: 0,
            line: 1,
            path,
        }
    }

    /// Returns false only at EOF; any incomplete reader structure is an error.
    fn skip_space_and_comments(&mut self) -> Result<bool, CoverageError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r') => self.bump(),
                Some(b'\n') => self.bump(),
                // `#;` is a reader comment, not a line comment: consume the
                // complete discarded form before looking for the next real one.
                Some(b'#') if self.bytes.get(self.index + 1) == Some(&b';') => {
                    self.bump();
                    self.bump();
                    self.skip_space_and_comments()?;
                    if self.peek().is_none() {
                        return Err(self.error("reader comment without a form"));
                    }
                    self.form()?;
                }
                Some(b';') => {
                    while !matches!(self.peek(), None | Some(b'\n')) {
                        self.bump();
                    }
                }
                Some(_) => return Ok(true),
                None => return Ok(false),
            }
        }
    }

    fn form(&mut self) -> Result<SourceForm, CoverageError> {
        let start_line = self.line;
        self.consume_prefixes()?;
        let kind = if matches!(self.peek(), Some(b'(')) {
            self.list_kind()?
        } else {
            self.atom()?;
            "atom".to_owned()
        };
        Ok(SourceForm { start_line, kind })
    }

    fn consume_prefixes(&mut self) -> Result<(), CoverageError> {
        loop {
            match (self.peek(), self.bytes.get(self.index + 1).copied()) {
                (Some(b'\'' | b'`'), _) => self.bump(),
                (Some(b','), Some(b'@')) => {
                    self.bump();
                    self.bump();
                }
                (Some(b','), _) => self.bump(),
                (Some(b'#'), Some(b'(')) => self.bump(),
                (Some(b'#'), Some(b'\'' | b'.' | b'_')) => {
                    self.bump();
                    self.bump();
                }
                // Reader notation such as #s(...) leaves the following
                // structure to the normal recursive reader.
                (Some(b'#'), Some(byte)) if byte.is_ascii_alphabetic() => {
                    self.bump();
                    while matches!(self.peek(), Some(byte) if byte.is_ascii_alphanumeric() || byte == b'-')
                    {
                        self.bump();
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn list_kind(&mut self) -> Result<String, CoverageError> {
        self.bump();
        self.skip_space_and_comments()?;
        let kind = match self.peek() {
            Some(b')') => "list".to_owned(),
            Some(_) => self.read_symbol().unwrap_or_else(|| "list".to_owned()),
            None => return Err(self.error("unterminated list")),
        };
        loop {
            self.skip_space_and_comments()?;
            match self.peek() {
                Some(b')') => {
                    self.bump();
                    return Ok(kind);
                }
                Some(b']' | b'}') => return Err(self.error("mismatched closing delimiter")),
                Some(_) => {
                    self.form()?;
                }
                None => return Err(self.error("unterminated list")),
            }
        }
    }

    fn atom(&mut self) -> Result<(), CoverageError> {
        match self.peek() {
            Some(b'"') => self.string(),
            Some(b'?') => self.character(),
            Some(b'(') => {
                self.list_kind()?;
                Ok(())
            }
            Some(b'[') => self.delimited(b'[', b']'),
            Some(b'{') => self.delimited(b'{', b'}'),
            Some(b')' | b']' | b'}') => Err(self.error("unexpected closing delimiter")),
            Some(_) => {
                self.read_symbol();
                Ok(())
            }
            None => Err(self.error("expected form")),
        }
    }

    fn delimited(&mut self, open: u8, close: u8) -> Result<(), CoverageError> {
        debug_assert_eq!(self.peek(), Some(open));
        self.bump();
        loop {
            self.skip_space_and_comments()?;
            match self.peek() {
                Some(byte) if byte == close => {
                    self.bump();
                    return Ok(());
                }
                Some(b')' | b']' | b'}') => return Err(self.error("mismatched closing delimiter")),
                Some(_) => {
                    self.form()?;
                }
                None => return Err(self.error("unterminated reader structure")),
            }
        }
    }

    fn string(&mut self) -> Result<(), CoverageError> {
        self.bump();
        while let Some(byte) = self.peek() {
            self.bump();
            if byte == b'\\' {
                if self.peek().is_none() {
                    return Err(self.error("unterminated escape in string"));
                }
                self.bump();
            } else if byte == b'"' {
                return Ok(());
            }
        }
        Err(self.error("unterminated string"))
    }

    fn character(&mut self) -> Result<(), CoverageError> {
        self.bump();
        if self.peek() == Some(b'\\') {
            self.bump();
        }
        if self.peek().is_none() {
            return Err(self.error("unterminated character literal"));
        }
        self.bump();
        Ok(())
    }

    fn read_symbol(&mut self) -> Option<String> {
        let start = self.index;
        while matches!(self.peek(), Some(byte) if !byte.is_ascii_whitespace() && !matches!(byte, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'\'' | b'`' | b',' | b';' | b'"'))
        {
            self.bump();
        }
        (start != self.index).then(|| self.text[start..self.index].to_owned())
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn bump(&mut self) {
        if self.peek() == Some(b'\n') {
            self.line += 1;
        }
        self.index += 1;
    }

    fn error(&self, message: &str) -> CoverageError {
        CoverageError::Source {
            path: self.path.to_owned(),
            message: format!("line {}: {message}", self.line),
        }
    }
}
