//! Markdown link scanning — the single home of "what is a link, and where does it
//! point" for the whole of `xtask` (#682).
//!
//! Two callers share it: `adr::strip_one_level` (rewriting a promoted draft's
//! targets) and the `doc-links` gate step (checking the tracked corpus). Keeping
//! one scanner is what makes those two agree about code spans, URL schemes, and
//! fragments — a second implementation would drift.

/// An inline Markdown link found outside code spans and fenced blocks. Carries a
/// byte range rather than a line number so the scanner computes only what its
/// callers read.
pub struct Link {
    /// Byte range of the *target* within the source — the text between `](` and the
    /// closing `)` (or the whitespace that starts a link title).
    pub span: std::ops::Range<usize>,
    /// The target text.
    pub target: String,
}

/// Blank out fenced blocks and inline code spans, replacing every non-newline byte
/// with a space. Length- and line-preserving, so byte offsets computed on the mask
/// are valid in the original.
///
/// UTF-8 safety: the only bytes inspected are backtick, `~`, space, tab and `\n`,
/// none of which can occur inside a multi-byte character (lead and continuation
/// bytes are all >= 0x80). So byte-wise scanning never lands mid-character, and the
/// `from_utf8` below cannot fail.
///
/// An unclosed fence masks to end of file. That yields false negatives (links after
/// it go unchecked), never false positives — the safe direction for a gate.
fn mask_code(body: &str) -> String {
    /// Blank a whole line, keeping its newline.
    fn blank(out: &mut [u8], at: usize, line: &str) {
        for (k, b) in line.bytes().enumerate() {
            if b != b'\n' {
                out[at + k] = b' ';
            }
        }
    }
    /// Blank paired backtick spans within a single line.
    fn blank_spans(out: &mut [u8], at: usize, line: &str) {
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'`' {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'`' {
                    j += 1;
                }
                if j < b.len() {
                    for slot in out.iter_mut().take(at + j + 1).skip(at + i) {
                        *slot = b' ';
                    }
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
    }

    let mut out: Vec<u8> = body.bytes().collect();
    let mut fence: Option<&[u8]> = None;
    let mut at = 0usize;
    for line in body.split_inclusive('\n') {
        // Trim leading whitespace: a fence may be indented inside a list item.
        let trimmed = line.trim_start();
        let marker = [&b"```"[..], &b"~~~"[..]]
            .into_iter()
            .find(|m| trimmed.as_bytes().starts_with(m));
        match (fence, marker) {
            (None, Some(m)) => {
                fence = Some(m);
                blank(&mut out, at, line);
            }
            (Some(open), Some(m)) if open == m => {
                fence = None;
                blank(&mut out, at, line);
            }
            (Some(_), _) => blank(&mut out, at, line),
            (None, None) => blank_spans(&mut out, at, line),
        }
        at += line.len();
    }
    String::from_utf8(out).expect("masking preserves UTF-8 boundaries")
}

/// Every inline `](target)` link in `body`, skipping fenced code blocks and inline
/// code spans.
pub fn links_in(body: &str) -> Vec<Link> {
    let masked = mask_code(body);
    let bytes = masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b']' && bytes[i + 1] == b'(' {
            let start = i + 2;
            // The target ends at the closing paren, or at the whitespace that
            // introduces an optional link title.
            let mut end = start;
            while end < bytes.len() && !matches!(bytes[end], b')' | b'\n' | b' ' | b'\t') {
                end += 1;
            }
            // The link is only well-formed if a closing paren follows on this line.
            let mut close = end;
            while close < bytes.len() && bytes[close] != b')' && bytes[close] != b'\n' {
                close += 1;
            }
            if end > start && close < bytes.len() && bytes[close] == b')' {
                out.push(Link {
                    span: start..end,
                    target: body[start..end].to_string(),
                });
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// True when `target` is a relative path worth resolving — i.e. not a
/// `http:`/`https:`/`mailto:` URL and not a bare `#anchor`.
pub fn is_relative_target(target: &str) -> bool {
    !target.starts_with('#')
        && !target.starts_with("http://")
        && !target.starts_with("https://")
        && !target.starts_with("mailto:")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- links_in: what counts as a link ---

    #[test]
    fn finds_a_plain_inline_link() {
        let ls = links_in("see [x](a.md) here");
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].target, "a.md");
    }

    #[test]
    fn finds_multiple_links_on_one_line() {
        let ls = links_in("[a](x.md) and [b](y.md)");
        assert_eq!(ls.len(), 2);
        assert_eq!(ls[1].target, "y.md");
    }

    #[test]
    fn span_covers_exactly_the_target() {
        let body = "see [x](a.md)";
        let l = &links_in(body)[0];
        assert_eq!(&body[l.span.clone()], "a.md");
    }

    #[test]
    fn skips_links_inside_a_fenced_block() {
        assert!(links_in("before\n```\n[x](a.md)\n```\nafter").is_empty());
    }

    #[test]
    fn skips_links_inside_a_tilde_fenced_block() {
        assert!(links_in("~~~\n[x](a.md)\n~~~\n").is_empty());
    }

    #[test]
    fn skips_links_inside_an_indented_fenced_block() {
        // CONTRIBUTING.md fences code inside list items; a column-0-only check
        // would leave those blocks live.
        assert!(links_in("- item:\n\n  ```\n  [x](a.md)\n  ```\n").is_empty());
    }

    #[test]
    fn skips_links_inside_an_inline_code_span() {
        assert!(links_in("write `[x](a.md)` like so").is_empty());
    }

    #[test]
    fn finds_a_link_after_a_fenced_block_closes() {
        let ls = links_in("```\n[a](x.md)\n```\n[b](y.md)");
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].target, "y.md");
    }

    // --- is_relative_target ---

    #[test]
    fn urls_and_anchors_are_not_relative_targets() {
        for t in ["https://e.com", "http://e.com", "mailto:a@b.c", "#section"] {
            assert!(!is_relative_target(t), "{t}");
        }
    }

    #[test]
    fn paths_are_relative_targets() {
        for t in ["a.md", "../a.md", "adr/", "a.md#frag"] {
            assert!(is_relative_target(t), "{t}");
        }
    }
}
