//! Host-run coverage evidence consumers behind `coverage-check` (ADR-0246):
//! the source-aware coverage/CRAP policy, its local benchmark and source-filter
//! probe, Emacs Lisp coverage reconciliation, Playwright/WASM coverage evidence
//! and mutation-evidence verification. None of this runs inside Nix.

mod digest;
pub mod elisp;
pub mod mutants;
pub mod nix;
pub mod policy;
pub mod wasm_evidence;

pub(crate) use digest::lowercase_hex;
