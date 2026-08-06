//! Per-section byte accounting for a wasm binary (#836).
//!
//! The unit is the section's **on-disk span** — its id byte, its LEB128 length
//! prefix, and its payload — not the payload length alone. That choice is what
//! lets the spans plus the 8-byte magic+version header account for every byte of
//! the file, which [`assert_spans_cover`] enforces. A breakdown whose parts do
//! not sum to the whole invites reading its percentages as shares of the file
//! when they are not, so the invariant is checked rather than assumed.

use anyhow::{bail, Result};
use serde::Serialize;

/// The 8-byte `\0asm` magic plus version that precedes the first section.
const HEADER_BYTES: u64 = 8;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct SectionSize {
    /// The section's wasm name (`type`, `code`, `data`, …), or `custom:<name>`
    /// for a custom section.
    pub name: String,
    /// The section's full on-disk span, id byte and length prefix included.
    pub bytes: u64,
}

/// The coverage invariant, separated from parsing so it can be tested without a
/// wasm file: spans plus the header must equal the file length exactly. Errs in
/// both directions — under-coverage means a section was missed, over-coverage
/// means one was counted twice.
pub fn assert_spans_cover(file_len: u64, spans: &[SectionSize]) -> Result<()> {
    let total: u64 = spans.iter().map(|s| s.bytes).sum();
    let accounted = total + HEADER_BYTES;
    if accounted != file_len {
        bail!(
            "section spans do not account for the file: {accounted} bytes \
             ({total} in {} section(s) + {HEADER_BYTES} header) vs {file_len} on disk",
            spans.len(),
        );
    }
    Ok(())
}

/// Every section's on-disk span, in file order.
///
/// Errs if the module does not parse, or if the spans do not cover the file.
pub fn section_sizes(wasm: &[u8]) -> Result<Vec<SectionSize>> {
    use wasmparser::Payload;

    let mut sizes = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let payload = payload?;
        // `Payload::as_section` gives (id, payload-range) for exactly the
        // section-carrying payloads, skipping the header and the synthetic
        // end-of-module marker. The span reaches back from the payload start to
        // the id byte: the length prefix is a LEB128 encoding of the payload
        // length, whose width we can compute rather than guess.
        let Some((id, range)) = payload.as_section() else {
            continue;
        };
        let payload_len = (range.end - range.start) as u64;
        let span = 1 + leb128_len(payload_len) + payload_len;
        let name = match payload {
            Payload::CustomSection(ref c) => format!("custom:{}", c.name()),
            _ => section_name(id).to_string(),
        };
        sizes.push(SectionSize { name, bytes: span });
    }

    assert_spans_cover(wasm.len() as u64, &sizes)?;
    Ok(sizes)
}

/// Byte width of `value` as an unsigned LEB128 — the section length prefix's size.
fn leb128_len(value: u64) -> u64 {
    let mut n = 1;
    let mut v = value >> 7;
    while v != 0 {
        n += 1;
        v >>= 7;
    }
    n
}

/// The spec name for a section id. Unknown ids are reported by number rather
/// than folded into a catch-all, so a future section type stays visible.
fn section_name(id: u8) -> String {
    match id {
        0 => "custom".to_string(),
        1 => "type".to_string(),
        2 => "import".to_string(),
        3 => "function".to_string(),
        4 => "table".to_string(),
        5 => "memory".to_string(),
        6 => "global".to_string(),
        7 => "export".to_string(),
        8 => "start".to_string(),
        9 => "element".to_string(),
        10 => "code".to_string(),
        11 => "data".to_string(),
        12 => "data-count".to_string(),
        13 => "tag".to_string(),
        other => format!("section-{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_encoder::{
        CodeSection, CustomSection, Function, FunctionSection, Instruction, Module, TypeSection,
        ValType,
    };

    /// A module with one function returning a constant, plus a custom section —
    /// enough to exercise type/function/code/custom spans.
    fn fixture() -> Vec<u8> {
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I32]);
        m.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        m.section(&funcs);
        let mut code = CodeSection::new();
        let mut f = Function::new([]);
        f.instruction(&Instruction::I32Const(7));
        f.instruction(&Instruction::End);
        code.function(&f);
        m.section(&code);
        m.section(&CustomSection {
            name: "producers".into(),
            data: std::borrow::Cow::Borrowed(b"xtask-test"),
        });
        m.finish()
    }

    #[test]
    fn spans_sum_to_the_file_size() {
        let wasm = fixture();
        let sizes = section_sizes(&wasm).unwrap();
        let total: u64 = sizes.iter().map(|s| s.bytes).sum();
        assert_eq!(
            total + 8,
            wasm.len() as u64,
            "sections plus the 8-byte header must account for the whole file"
        );
    }

    #[test]
    fn names_every_section_it_finds() {
        let sizes = section_sizes(&fixture()).unwrap();
        let names: Vec<&str> = sizes.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"type"), "{names:?}");
        assert!(names.contains(&"function"), "{names:?}");
        assert!(names.contains(&"code"), "{names:?}");
        assert!(
            names.contains(&"custom:producers"),
            "custom sections are named by their own name: {names:?}"
        );
    }

    #[test]
    fn every_span_is_non_zero() {
        for s in section_sizes(&fixture()).unwrap() {
            assert!(s.bytes > 0, "section {} has a zero span", s.name);
        }
    }

    #[test]
    fn spans_that_do_not_cover_the_file_are_rejected() {
        // A5's named invariant, pinned directly: `wasmparser` catches malformed
        // input, but the *coverage* check is ours, so it needs its own test
        // rather than riding on a parse failure.
        let spans = vec![SectionSize {
            name: "code".into(),
            bytes: 10,
        }];
        assert!(
            assert_spans_cover(100, &spans).is_err(),
            "under-coverage must Err"
        );
        assert!(
            assert_spans_cover(1000, &spans).is_err(),
            "over-coverage must Err"
        );
        assert!(
            assert_spans_cover(18, &spans).is_ok(),
            "10 + 8 header == 18"
        );
    }

    #[test]
    fn rejects_a_truncated_module() {
        let mut wasm = fixture();
        wasm.truncate(wasm.len() - 3);
        assert!(
            section_sizes(&wasm).is_err(),
            "a truncated module must Err, not silently under-report"
        );
    }

    #[test]
    fn rejects_a_non_wasm_input() {
        assert!(section_sizes(b"not a wasm file at all").is_err());
    }
}
