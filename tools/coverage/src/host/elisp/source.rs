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
    expression: Expression,
}

impl SourceForm {
    pub(crate) fn automatically_structural(&self) -> bool {
        match self.kind.as_str() {
            "require" | "provide" | "declare-function" | "defgroup" | "cl-defstruct" => true,
            "defvar" | "defconst" | "defcustom" => match self.expression.list_elements() {
                Some([_, Expression::Symbol(_)]) => true,
                Some([_, Expression::Symbol(_), initializer, ..]) => {
                    initializer.is_inert_initializer()
                }
                _ => false,
            },
            _ => false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Expression {
    Symbol(String),
    String,
    Character,
    List(Vec<Expression>),
    Vector(Vec<Expression>),
    Quote(Box<Expression>),
    FunctionQuote(Box<Expression>),
    Other,
}

impl Expression {
    fn list_elements(&self) -> Option<&[Self]> {
        let Self::List(elements) = self else {
            return None;
        };
        Some(elements)
    }

    fn is_inert_initializer(&self) -> bool {
        match self {
            Self::String | Self::Character => true,
            Self::Vector(elements) => elements.iter().all(Self::is_literal_data),
            Self::Quote(expression) | Self::FunctionQuote(expression) => {
                expression.is_literal_data()
            }
            Self::Symbol(symbol) => {
                matches!(symbol.as_str(), "nil" | "t")
                    || symbol.starts_with(':')
                    || numeric_literal(symbol)
            }
            Self::List(_) | Self::Other => false,
        }
    }

    /// Vector and quoted contents are reader data, except forms that invoke
    /// evaluation or syntax the reader cannot classify.
    fn is_literal_data(&self) -> bool {
        match self {
            Self::List(elements) | Self::Vector(elements) => {
                elements.iter().all(Self::is_literal_data)
            }
            Self::Quote(expression) | Self::FunctionQuote(expression) => {
                expression.is_literal_data()
            }
            Self::Symbol(_) | Self::String | Self::Character => true,
            Self::Other => false,
        }
    }
}

fn numeric_literal(symbol: &str) -> bool {
    radix_integer_literal(symbol) || decimal_or_float_literal(symbol)
}

fn radix_integer_literal(symbol: &str) -> bool {
    let Some(prefix) = symbol.strip_prefix('#') else {
        return false;
    };
    let (radix, digits) = match prefix.as_bytes() {
        [b'b', rest @ ..] => (2, rest),
        [b'o', rest @ ..] => (8, rest),
        [b'x', rest @ ..] => (16, rest),
        _ => {
            let radix_end = prefix
                .bytes()
                .take_while(|digit| digit.is_ascii_digit())
                .count();
            let Some((radix, digits)) = prefix
                .get(..radix_end)
                .and_then(|radix| radix.parse::<u8>().ok())
                .zip(
                    prefix
                        .get(radix_end..)
                        .and_then(|suffix| suffix.strip_prefix('r')),
                )
            else {
                return false;
            };
            (radix, digits.as_bytes())
        }
    };
    let digits = match digits {
        [b'+' | b'-', digits @ ..] => digits,
        digits => digits,
    };
    (2..=36).contains(&radix)
        && !digits.is_empty()
        && digits
            .iter()
            .all(|digit| digit_value(*digit).is_some_and(|value| value < radix))
}

fn digit_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'z' => Some(digit - b'a' + 10),
        b'A'..=b'Z' => Some(digit - b'A' + 10),
        _ => None,
    }
}

fn decimal_or_float_literal(symbol: &str) -> bool {
    let digits = symbol
        .strip_prefix('+')
        .or_else(|| symbol.strip_prefix('-'))
        .unwrap_or(symbol);
    (!digits.is_empty() && digits.bytes().all(|digit| digit.is_ascii_digit()))
        || (symbol
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'))
            && symbol.parse::<f64>().is_ok())
}

#[derive(Clone, Copy)]
enum Prefix {
    Plain,
    Quote,
    FunctionQuote,
    Other,
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

pub(crate) fn assert_forms(
    path: &Path,
    census: &[FormCensus],
) -> Result<(String, Vec<SourceForm>), CoverageError> {
    let (text, forms) = read_forms(path)?;
    let expected: Vec<_> = forms
        .iter()
        .map(|form| (form.start_line, form.kind.as_str()))
        .collect();
    let actual: Vec<_> = census
        .iter()
        .map(|form| (form.start_line, form.kind.as_str()))
        .collect();
    if actual != expected {
        return Err(CoverageError::Census {
            message: format!(
                "{} census forms do not match current source: expected {expected:?}, got {actual:?}",
                path.display()
            ),
        });
    }
    Ok((text, forms))
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
        let expression = self.expression()?;
        let kind = expression
            .list_elements()
            .and_then(|elements| elements.first())
            .and_then(|head| match head {
                Expression::Symbol(symbol) => Some(symbol.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "atom".to_owned());
        Ok(SourceForm {
            start_line,
            kind,
            expression,
        })
    }

    fn expression(&mut self) -> Result<Expression, CoverageError> {
        let prefix = self.consume_prefixes()?;
        let expression = match self.peek() {
            Some(b'(') => self.list()?,
            Some(b'[') => self.vector()?,
            Some(b'{') => {
                self.delimited(b'{', b'}')?;
                Expression::Other
            }
            Some(b'"') => {
                self.string()?;
                Expression::String
            }
            Some(b'?') => {
                self.character()?;
                Expression::Character
            }
            Some(b')' | b']' | b'}') => return Err(self.error("unexpected closing delimiter")),
            Some(_) => Expression::Symbol(
                self.read_symbol()
                    .ok_or_else(|| self.error("expected form"))?,
            ),
            None => return Err(self.error("expected form")),
        };
        Ok(match prefix {
            Prefix::Plain => expression,
            Prefix::Quote => Expression::Quote(Box::new(expression)),
            Prefix::FunctionQuote => Expression::FunctionQuote(Box::new(expression)),
            Prefix::Other => Expression::Other,
        })
    }

    fn consume_prefixes(&mut self) -> Result<Prefix, CoverageError> {
        let mut prefix = Prefix::Plain;
        loop {
            match (self.peek(), self.bytes.get(self.index + 1).copied()) {
                (Some(b'\''), _) => {
                    self.bump();
                    prefix = match prefix {
                        Prefix::Plain => Prefix::Quote,
                        _ => Prefix::Other,
                    };
                }
                (Some(b'`' | b','), _) => {
                    self.bump();
                    if self.peek() == Some(b'@') {
                        self.bump();
                    }
                    prefix = Prefix::Other;
                }
                (Some(b'#'), Some(b'(')) => {
                    self.bump();
                    prefix = Prefix::Other;
                }
                (Some(b'#'), Some(b'\'')) => {
                    self.bump();
                    self.bump();
                    prefix = match prefix {
                        Prefix::Plain => Prefix::FunctionQuote,
                        _ => Prefix::Other,
                    };
                }
                (Some(b'#'), Some(b'.' | b'_')) => {
                    self.bump();
                    self.bump();
                    prefix = Prefix::Other;
                }
                // `#s(...)` prefixes a record literal; numeric forms such as
                // `#x21` are ordinary atoms and must stay intact.
                (Some(b'#'), Some(b's')) if self.bytes.get(self.index + 2) == Some(&b'(') => {
                    self.bump();
                    self.bump();
                    prefix = Prefix::Other;
                }
                _ => return Ok(prefix),
            }
        }
    }

    fn list(&mut self) -> Result<Expression, CoverageError> {
        let opening_line = self.line;
        self.bump();
        let mut elements = Vec::new();
        loop {
            self.skip_space_and_comments()?;
            match self.peek() {
                Some(b')') => {
                    self.bump();
                    return Ok(Expression::List(elements));
                }
                Some(b']' | b'}') => return Err(self.error("mismatched closing delimiter")),
                Some(_) => elements.push(self.expression()?),
                None => {
                    return Err(
                        self.error(&format!("unterminated list opened at line {opening_line}"))
                    );
                }
            }
        }
    }

    fn vector(&mut self) -> Result<Expression, CoverageError> {
        debug_assert_eq!(self.peek(), Some(b'['));
        self.bump();
        let mut elements = Vec::new();
        loop {
            self.skip_space_and_comments()?;
            match self.peek() {
                Some(b']') => {
                    self.bump();
                    return Ok(Expression::Vector(elements));
                }
                Some(b')' | b'}') => return Err(self.error("mismatched closing delimiter")),
                Some(_) => elements.push(self.expression()?),
                None => return Err(self.error("unterminated reader structure")),
            }
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
                    self.expression()?;
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
            if self.peek() == Some(b'N') && self.bytes.get(self.index + 1) == Some(&b'{') {
                self.bump();
                self.bump();
                while !matches!(self.peek(), None | Some(b'}')) {
                    self.bump();
                }
                if self.peek().is_none() {
                    return Err(self.error("unterminated named character literal"));
                }
                self.bump();
                return Ok(());
            }
        }
        self.bump_utf8_character()
    }

    fn bump_utf8_character(&mut self) -> Result<(), CoverageError> {
        let remaining = &self.text[self.index..];
        let character = remaining
            .chars()
            .next()
            .ok_or_else(|| self.error("unterminated character literal"))?;
        for _ in 0..character.len_utf8() {
            self.bump();
        }
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
