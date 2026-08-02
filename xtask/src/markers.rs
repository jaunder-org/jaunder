//! Reading an in-source **exemption marker** off a line of Rust.
//!
//! Two gates express exemptions this way, and they share exactly this primitive —
//! "is there a `<token>` marker on this source line, and what reason does it give" —
//! and nothing above it. The coverage gate ([`crate::coverage`]) hands in a line from
//! an `llvm-cov` text report and honors `cov:ignore`; the ident-keyed XSS gates
//! ([`crate::steps::ident_gate`]) hand in a line from the file itself, located by a
//! `syn` span, and honor `<gate-step>:allow`. Their vocabularies and their strictness
//! differ deliberately — a bare `cov:ignore` is legal, a bare `html-sink:allow` is
//! not — but *where a comment legally begins* must have one answer, tested once.
//!
//! That question is not trivial, which is why it is here rather than open-coded twice:
//! a `//` inside a string, a char literal, a raw string, or a doc comment is **not** a
//! comment, and reading one as a marker would exempt a site nobody exempted.

/// True iff `marker` is the first whitespace-delimited token of `comment` (the text
/// after `//`, as returned by [`line_comment`]). Anchoring marker recognition to the
/// first token keeps an incidental mention in prose (`// unlike the cov:ignore path`)
/// inert, while still honoring `// cov:ignore` and `// cov:ignore <trailing note>`
/// (#246).
pub fn comment_marker_is(comment: &str, marker: &str) -> bool {
    comment.split_whitespace().next() == Some(marker)
}

/// Return the text of the first real trailing line comment in `src` — the slice
/// after the first `//` that begins OUTSIDE a `"`-string, a `'`-char literal, or a
/// raw string. Returns `None` when there is no such comment (so a `//` inside a
/// literal never counts). A **doc comment** — outer `///` or inner `//!` — is
/// deliberately NOT treated as a marker-bearing comment: it documents behavior and
/// can never carry an exemption, so a marker mentioned in prose is inert.
///
/// Escapes (`\"`, `\'`) are honored; a `'` that does not open a well-formed char
/// literal (a lifetime tick) is treated as ordinary text. Raw strings (`r"…"`,
/// `r#"…"#`, `br#"…"#`) are tracked by hash count, and an **unterminated** one
/// consumes the rest of the line rather than exposing its contents — fail-closed,
/// because this decides whether a security gate's exemption is real (#778).
pub fn line_comment(src: &str) -> Option<&str> {
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            match b {
                b'\\' => i += 1, // skip the escaped character
                b'"' => in_str = false,
                _ => {}
            }
            i += 1;
            continue;
        }
        if let Some(hashes) = raw_string_hashes(bytes, i) {
            // Jump the whole `r#"…"#`. An unterminated literal yields `None`: we
            // cannot see where it ends, so we must not report a marker from inside it.
            i = raw_string_end(bytes, i, hashes)?;
            continue;
        }
        match b {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                // `///` (outer doc) or `//!` (inner doc) is a doc comment, not a
                // marker-bearing comment — the rest of the line is documentation
                // prose and can never suppress coverage or exempt a site.
                if matches!(bytes.get(i + 2), Some(&b'/') | Some(&b'!')) {
                    return None;
                }
                return Some(&src[i + 2..]);
            }
            b'"' => in_str = true,
            b'\'' => {
                if let Some(len) = char_literal_len(&bytes[i..]) {
                    i += len; // jump past the whole char literal
                    continue;
                }
                // otherwise a lifetime tick — fall through, treat as ordinary text
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The reason text of a `token` marker on `line`, or `None` when the line carries
/// no such marker. `Some("")` means the marker is present but **bare** — a
/// distinction each gate prices for itself: coverage accepts a bare `cov:ignore`,
/// the XSS gates fail one.
pub fn marker_on_line<'a>(line: &'a str, token: &str) -> Option<&'a str> {
    let comment = line_comment(line)?;
    if !comment_marker_is(comment, token) {
        return None;
    }
    // `comment_marker_is` just proved the first token matches, so this slice is in
    // bounds and lands exactly past the token.
    Some(comment.trim_start()[token.len()..].trim())
}

/// Whether a raw-string literal opens at `i`, and with how many `#`s. `None` when
/// this `r` is an ordinary identifier byte (`var"`) rather than a literal prefix.
fn raw_string_hashes(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes[i] != b'r' {
        return None;
    }
    // The `r` must begin a token: at line start, after a non-identifier byte, or as
    // the `r` of a `br"…"` byte string whose `b` is itself token-initial.
    let token_start = |k: usize| k == 0 || !is_ident_byte(bytes[k - 1]);
    if !(token_start(i) || (bytes[i - 1] == b'b' && token_start(i - 1))) {
        return None;
    }
    let mut j = i + 1;
    while bytes.get(j) == Some(&b'#') {
        j += 1;
    }
    (bytes.get(j) == Some(&b'"')).then_some(j - i - 1)
}

/// Index just past the closing delimiter of the raw string opening at `i` with
/// `hashes` hashes, or `None` if it never closes on this line.
fn raw_string_end(bytes: &[u8], i: usize, hashes: usize) -> Option<usize> {
    let mut j = i + 1 + hashes + 1; // past `r`, the hashes, and the opening quote
    while j < bytes.len() {
        if bytes[j] == b'"' && (0..hashes).all(|k| bytes.get(j + 1 + k) == Some(&b'#')) {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    None
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// If `bytes` (which starts at a `'`) opens a well-formed char literal, return
/// its length in bytes including both quotes; otherwise `None` (e.g. a lifetime
/// tick like `'a`). Best-effort — handles simple and escaped char literals.
fn char_literal_len(bytes: &[u8]) -> Option<usize> {
    debug_assert_eq!(bytes.first(), Some(&b'\''));
    if bytes.len() < 3 {
        return None;
    }
    if bytes[1] == b'\\' {
        // Escaped: '\n', '\'', '\\', '\0', '\u{1F600}', '\x41' … the byte right
        // after the backslash is literal, so start scanning for the closer past it.
        let mut j = 3;
        while j < bytes.len() {
            if bytes[j] == b'\'' {
                return Some(j + 1);
            }
            j += 1;
        }
        None
    } else {
        // Unescaped: one UTF-8 scalar (1..=4 bytes) then a closing quote. A
        // closing quote within the next few bytes marks a real char literal; its
        // absence (e.g. `'a` in `'a, 'b>`) means a lifetime.
        let end = bytes.len().min(6);
        let mut j = 2;
        while j < end {
            if bytes[j] == b'\'' {
                return Some(j + 1);
            }
            j += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{comment_marker_is, line_comment, marker_on_line};

    #[test]
    fn line_comment_ignores_markers_inside_strings_and_finds_real_comments() {
        assert_eq!(
            line_comment("    boom() // cov:ignore"),
            Some(" cov:ignore")
        );
        assert_eq!(line_comment("    let s = \"// cov:ignore\";"), None);
        assert_eq!(line_comment("    let c = '/';"), None);
        // A lifetime tick must not swallow a following real comment.
        assert_eq!(
            line_comment("    fn f<'a>() {} // cov:ignore"),
            Some(" cov:ignore")
        );
        // Doc comments (`///` outer, `//!` inner) are not marker-bearing
        // comments — a real `//` still is.
        assert_eq!(line_comment("/// cov:ignore-start"), None);
        assert_eq!(line_comment("//! cov:ignore"), None);
        assert_eq!(line_comment("    boom() /// cov:ignore"), None);
    }

    #[test]
    fn comment_marker_is_matches_first_token_only() {
        assert!(comment_marker_is(" cov:ignore", "cov:ignore"));
        assert!(comment_marker_is(" cov:ignore trailing", "cov:ignore"));
        assert!(comment_marker_is("cov:ignore", "cov:ignore")); // no leading space
        assert!(!comment_marker_is(
            " unlike the cov:ignore path",
            "cov:ignore"
        ));
        assert!(comment_marker_is(" cov:ignore-start", "cov:ignore-start"));
        assert!(!comment_marker_is(" cov:ignore-start", "cov:ignore")); // distinct token
        assert!(comment_marker_is(" cov:ignore-stop", "cov:ignore-stop"));
        assert!(!comment_marker_is(
            " closes the cov:ignore-stop block",
            "cov:ignore-stop"
        ));
    }

    #[test]
    fn a_marker_inside_a_raw_string_is_not_a_comment() {
        assert_eq!(
            line_comment(r##"let s = r#"// html-sink:allow x"#;"##),
            None
        );
    }

    #[test]
    fn a_marker_after_a_raw_string_is_still_found() {
        assert_eq!(
            line_comment(r##"let s = r#"a // b"#; // html-sink:allow real"##),
            Some(" html-sink:allow real")
        );
    }

    #[test]
    fn a_hash_less_raw_string_is_honored() {
        assert_eq!(line_comment(r#"let s = r"// x"; // real"#), Some(" real"));
    }

    #[test]
    fn a_byte_raw_string_is_honored() {
        assert_eq!(
            line_comment(r##"let s = br#"// x"#; // real"##),
            Some(" real")
        );
    }

    #[test]
    fn an_identifier_ending_in_r_does_not_open_a_raw_string() {
        // `var"…"` is an identifier then an ordinary string, not a raw literal.
        assert_eq!(line_comment(r#"let x = var"// y"; // real"#), Some(" real"));
    }

    #[test]
    fn a_multi_hash_raw_string_needs_its_full_closer() {
        // The inner `"#` must not close a `r##"…"##`.
        assert_eq!(
            line_comment(r###"let s = r##"a "# b // c"##; // real"###),
            Some(" real")
        );
    }

    #[test]
    fn an_unterminated_raw_string_swallows_the_rest_of_the_line() {
        // Fail-closed: if we cannot tell where the literal ends, we do not invent a
        // marker inside it.
        assert_eq!(line_comment(r##"let s = r#"// html-sink:allow x"##), None);
    }

    #[test]
    fn marker_on_line_returns_the_reason() {
        assert_eq!(
            marker_on_line(
                "code() // html-sink:allow because reasons",
                "html-sink:allow"
            ),
            Some("because reasons")
        );
    }

    #[test]
    fn marker_on_line_returns_empty_for_a_bare_marker() {
        assert_eq!(
            marker_on_line("code() // html-sink:allow", "html-sink:allow"),
            Some("")
        );
    }

    #[test]
    fn marker_on_line_ignores_another_gates_token() {
        assert_eq!(
            marker_on_line("code() // raw-html-door:allow r", "html-sink:allow"),
            None
        );
    }

    #[test]
    fn marker_on_line_ignores_a_prose_mention() {
        assert_eq!(
            marker_on_line("code() // see the html-sink:allow docs", "html-sink:allow"),
            None
        );
    }

    #[test]
    fn marker_on_line_ignores_a_doc_comment() {
        assert_eq!(
            marker_on_line("/// html-sink:allow x", "html-sink:allow"),
            None
        );
        assert_eq!(
            marker_on_line("//! html-sink:allow x", "html-sink:allow"),
            None
        );
    }

    #[test]
    fn marker_on_line_finds_nothing_on_an_uncommented_line() {
        assert_eq!(marker_on_line("code()", "html-sink:allow"), None);
    }
}
