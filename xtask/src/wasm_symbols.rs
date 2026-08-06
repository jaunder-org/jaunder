//! Attribute wasm function bytes to the crates they came from (#836).
//!
//! Parsing and rolling up are separate on purpose: [`rollup`] is a pure function
//! over `(name, bytes)` pairs, so the attribution rules — which mangling schemes
//! are understood, what happens to an unmangled name — are testable without a
//! wasm file anywhere in sight. [`function_sizes`] does the wasm-shaped half.
//!
//! Bytes are **conserved**, never filtered: a function whose name teaches us
//! nothing still contributes its size, to [`UNATTRIBUTED`]. A rollup that
//! quietly dropped such functions would report a code section smaller than it is.

use anyhow::Result;
use serde::Serialize;

/// The bucket for functions whose originating crate cannot be determined —
/// unnamed functions, and named ones that are not Rust-mangled (wasm-bindgen's
/// JS-glue shims, for instance).
pub const UNATTRIBUTED: &str = "<unattributed>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSize {
    /// The name-section name, if the module carries one for this function.
    pub name: Option<String>,
    /// The function body's byte span in the code section.
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CrateBytes {
    #[serde(rename = "crate")]
    pub krate: String,
    pub bytes: u64,
}

/// Function body spans in the code section, paired with their name-section name.
///
/// Function indices span imports first, so a code-section entry `i` is function
/// `imported_function_count + i` — getting that offset wrong would misattribute
/// every function in a module that imports any.
pub fn function_sizes(wasm: &[u8]) -> Result<Vec<FunctionSize>> {
    use wasmparser::{Name, Payload};

    let mut imported_functions = 0u32;
    let mut bodies: Vec<u64> = Vec::new();
    let mut names: std::collections::HashMap<u32, String> = std::collections::HashMap::new();

    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        match payload? {
            Payload::ImportSection(reader) => {
                // `into_imports` flattens the compact encodings, so each item is
                // one import; iterating the reader directly yields groups.
                for import in reader.into_imports() {
                    if matches!(import?.ty, wasmparser::TypeRef::Func(_)) {
                        imported_functions += 1;
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                let range = body.range();
                bodies.push((range.end - range.start) as u64);
            }
            Payload::CustomSection(c) if c.name() == "name" => {
                let reader = wasmparser::NameSectionReader::new(wasmparser::BinaryReader::new(
                    c.data(),
                    c.data_offset(),
                ));
                for subsection in reader {
                    if let Name::Function(map) = subsection? {
                        for naming in map {
                            let naming = naming?;
                            names.insert(naming.index, naming.name.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(bodies
        .into_iter()
        .enumerate()
        .map(|(i, bytes)| {
            let index = imported_functions + i as u32;
            FunctionSize {
                name: names.get(&index).cloned(),
                bytes,
            }
        })
        .collect())
}

/// Bucket function bytes by originating crate, sorted by bytes descending.
///
/// Conserves every byte it is given — see the module docs.
pub fn rollup(functions: &[FunctionSize]) -> Vec<CrateBytes> {
    let mut totals: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for f in functions {
        let krate = f
            .name
            .as_deref()
            .and_then(crate_of)
            .unwrap_or_else(|| UNATTRIBUTED.to_string());
        *totals.entry(krate).or_default() += f.bytes;
    }
    let mut out: Vec<CrateBytes> = totals
        .into_iter()
        .map(|(krate, bytes)| CrateBytes { krate, bytes })
        .collect();
    // Name is the tie-break so the report is stable run to run; a HashMap's
    // iteration order is not, and an unstable report reads as churn in a diff.
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.krate.cmp(&b.krate)));
    out
}

/// The crate a Rust-mangled symbol came from: the first path segment of its
/// demangled form. `None` for anything that is not Rust-mangled — inventing a
/// crate from an unmangled name would put wasm-bindgen's shims in a bucket named
/// after whatever their first identifier happened to be.
fn crate_of(symbol: &str) -> Option<String> {
    // `try_demangle` rejects non-Rust symbols, which is the discrimination we
    // want; `demangle` alone passes them through unchanged and looks successful.
    let demangled = rustc_demangle::try_demangle(symbol).ok()?.to_string();
    crate_of_demangled(&demangled)
}

/// The crate a *demangled* Rust path belongs to.
///
/// Split from [`crate_of`] so the attribution rule is testable on readable paths
/// rather than on hand-built mangled symbols.
///
/// Trait-impl methods demangle to `<Self as Trait>::method`, which has no crate
/// in leading position. Treating those as unattributable would be a large and
/// entirely self-inflicted blind spot — they are among the most common symbols
/// in any Rust binary — so the self type's crate is used, falling back to the
/// trait's when the self type is a primitive (`<u32 as Display>::fmt` is code
/// that `core` emitted).
fn crate_of_demangled(path: &str) -> Option<String> {
    let path = path.trim();
    if let Some(inner) = path.strip_prefix('<') {
        let inner = inner.split_once(">::").map_or(inner, |(i, _)| i);
        let (self_ty, trait_ty) = match inner.split_once(" as ") {
            Some((s, t)) => (s, Some(t)),
            None => (inner, None),
        };
        return first_segment(self_ty).or_else(|| trait_ty.and_then(first_segment));
    }
    first_segment(path)
}

/// The crate name leading a type or path expression, or `None` if it does not
/// start with one (a primitive, a reference to one, a bare generic parameter).
fn first_segment(text: &str) -> Option<String> {
    // Peel the type syntax that can precede a path.
    let mut t = text.trim();
    loop {
        let stripped = t
            .strip_prefix('&')
            .or_else(|| t.strip_prefix("*const "))
            .or_else(|| t.strip_prefix("*mut "))
            .or_else(|| t.strip_prefix("mut "))
            .or_else(|| t.strip_prefix("dyn "))
            .or_else(|| t.strip_prefix("impl "))
            .or_else(|| t.strip_prefix('('))
            .or_else(|| t.strip_prefix('['));
        match stripped {
            Some(s) => t = s.trim_start(),
            None => break,
        }
    }
    // The crate is the first `::` segment, cut before any generic arguments.
    let seg = t
        .split("::")
        .next()?
        .split('<')
        .next()?
        // v0 mangling renders the crate with its disambiguator, `croner[3c1c0]`.
        // Left in, one crate would split across as many buckets as it has
        // disambiguators, understating every one of them.
        .split('[')
        .next()?
        .trim();
    // A path that never had a `::` is a primitive or a generic parameter, not a
    // crate: `u32`, `T`. Requiring the separator is what keeps `<u32 as Display>`
    // from inventing a crate named `u32`.
    if seg.is_empty() || !t.contains("::") {
        return None;
    }
    Some(seg.to_string())
}

#[cfg(test)]
pub mod tests_support {
    use wasm_encoder::{
        CodeSection, Function, FunctionSection, Instruction, Module, NameMap, NameSection,
        TypeSection, ValType,
    };

    /// The mangled symbol both fixtures name their single function with.
    pub const FIXTURE_FN: &str = "_ZN6orgize5parse17h0123456789abcdefE";

    fn base() -> Module {
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I32]);
        m.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        m.section(&funcs);
        let mut code = CodeSection::new();
        let mut fun = Function::new([]);
        fun.instruction(&Instruction::I32Const(7));
        fun.instruction(&Instruction::End);
        code.function(&fun);
        m.section(&code);
        m
    }

    pub fn named_module() -> Vec<u8> {
        let mut m = base();
        let mut names = NameSection::new();
        let mut map = NameMap::new();
        map.append(0, FIXTURE_FN);
        names.functions(&map);
        m.section(&names);
        m.finish()
    }

    pub fn unnamed_module() -> Vec<u8> {
        base().finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: Option<&str>, bytes: u64) -> FunctionSize {
        FunctionSize {
            name: name.map(str::to_string),
            bytes,
        }
    }

    #[test]
    fn buckets_legacy_mangled_names_by_crate() {
        let fns = vec![
            f(Some("_ZN6orgize5parse17h0123456789abcdefE"), 100),
            f(Some("_ZN6orgize6render17hfedcba9876543210E"), 50),
            f(Some("_ZN4core3fmt5write17h1111111111111111E"), 30),
        ];
        let r = rollup(&fns);
        assert_eq!(r[0].krate, "orgize");
        assert_eq!(r[0].bytes, 150);
        assert_eq!(r[1].krate, "core");
        assert_eq!(r[1].bytes, 30);
    }

    #[test]
    fn buckets_v0_mangled_names_by_crate() {
        let fns = vec![f(Some("_RNvCs1234_6croner5parse"), 64)];
        let r = rollup(&fns);
        assert_eq!(r[0].krate, "croner", "v0 names must attribute too: {r:?}");
    }

    #[test]
    fn unnamed_functions_land_in_the_unattributed_bucket() {
        let fns = vec![
            f(None, 200),
            f(Some("_ZN6orgize5parse17h0123456789abcdefE"), 100),
        ];
        let r = rollup(&fns);
        assert_eq!(r[0].krate, UNATTRIBUTED);
        assert_eq!(r[0].bytes, 200);
    }

    #[test]
    fn unmangled_names_are_unattributed_not_treated_as_crates() {
        // wasm-bindgen emits plain JS-glue shims with unmangled names; they are
        // real bytes but belong to no crate, and must not invent one.
        let fns = vec![f(Some("__wbindgen_malloc"), 40)];
        let r = rollup(&fns);
        assert_eq!(r[0].krate, UNATTRIBUTED);
        assert_eq!(r[0].bytes, 40);
    }

    #[test]
    fn rollup_is_sorted_by_bytes_descending_and_conserves_total() {
        let fns = vec![
            f(Some("_ZN4core3fmt5write17h1111111111111111E"), 10),
            f(Some("_ZN6orgize5parse17h0123456789abcdefE"), 500),
            f(None, 90),
        ];
        let r = rollup(&fns);
        assert!(r.windows(2).all(|w| w[0].bytes >= w[1].bytes), "{r:?}");
        assert_eq!(
            r.iter().map(|c| c.bytes).sum::<u64>(),
            600,
            "rollup must conserve every byte it was given"
        );
    }

    #[test]
    fn rollup_of_nothing_is_empty() {
        assert!(rollup(&[]).is_empty());
    }

    #[test]
    fn trait_impl_methods_attribute_to_the_self_type_crate() {
        // The commonest symbol shape in any Rust binary. Charging these to
        // `<unattributed>` would hide a large share of the code section behind a
        // gap in this function rather than a fact about the bundle.
        assert_eq!(
            crate_of_demangled("<reactive_graph::Signal as core::fmt::Debug>::fmt"),
            Some("reactive_graph".to_string())
        );
        assert_eq!(
            crate_of_demangled("<&orgize::Ast as core::clone::Clone>::clone"),
            Some("orgize".to_string())
        );
        assert_eq!(
            crate_of_demangled("<alloc::vec::Vec<T> as core::ops::Drop>::drop"),
            Some("alloc".to_string())
        );
    }

    #[test]
    fn primitive_self_types_fall_back_to_the_trait_crate() {
        // `<u32 as Display>::fmt` is code `core` emitted; `u32` is not a crate.
        assert_eq!(
            crate_of_demangled("<u32 as core::fmt::Display>::fmt"),
            Some("core".to_string())
        );
    }

    #[test]
    fn inherent_impls_without_a_trait_still_attribute() {
        assert_eq!(
            crate_of_demangled("<tachys::View>::build"),
            Some("tachys".to_string())
        );
    }

    #[test]
    fn bare_generics_and_primitives_attribute_to_nothing() {
        assert_eq!(crate_of_demangled("T"), None);
        assert_eq!(crate_of_demangled("u32"), None);
        assert_eq!(crate_of_demangled(""), None);
    }

    #[test]
    fn plain_paths_still_take_their_first_segment() {
        assert_eq!(
            crate_of_demangled("orgize::parse::inner"),
            Some("orgize".to_string())
        );
    }

    #[test]
    fn function_sizes_reads_names_from_the_name_section() {
        let got = function_sizes(&tests_support::named_module()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name.as_deref(), Some(tests_support::FIXTURE_FN));
        assert!(got[0].bytes > 0);
    }

    #[test]
    fn function_sizes_yields_unnamed_entries_without_a_name_section() {
        let got = function_sizes(&tests_support::unnamed_module()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].name, None,
            "no name section => no name, but still a body"
        );
        assert!(got[0].bytes > 0);
    }
}
