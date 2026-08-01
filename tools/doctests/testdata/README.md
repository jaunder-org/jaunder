# Fixture crates

Each file here is the `src/lib.rs` (or `src/main.rs`) of a **synthesized**
one-file crate that `harness.rs` materializes into a tempdir and actually
compiles. They are evidence about **what rustdoc and cargo do**, not about our
scanner — which is why they cannot be inline `r#"…"#` strings like the rest of
the crate's tests.

They are deliberately dependency-free, so the harness needs no registry access
and runs the same on a cold machine.

| fixture               | pins                                                                                         |
| --------------------- | -------------------------------------------------------------------------------------------- |
| `ordering_control.rs` | `PartialEq + Eq` alone does not give you `<` — the compiler fact the ordering proofs rest on |
| `cfg_feature.rs`      | shrink vector 1: a fence behind an unenabled `#[cfg(feature)]` never runs                    |
| `cfg_test_module.rs`  | shrink vector 2: rustdoc sets `cfg(doctest)`, not `cfg(test)`                                |
| `unknown_tag.rs`      | shrink vector 3: a wholly unrecognized info string is dropped silently                       |
| `bin_only.rs`         | shrink vector 5: cargo collects doctests from lib targets only                               |
| `failing.rs`          | a failing doctest is reported as FAILED, not as absent                                       |

**Shrink vector 4** — a crate outside every scan root — is structural and cannot
be shown in a fixture crate: it is the absence of a root, not a property of one.
It is covered instead by the scan-root coverage assertion over `roots::ALL`.

**The macro half of the ordering claim** — that `#[str_newtype(no_ord)]` and
`secret` actually suppress ordering, while an un-suppressed newtype orders — is
pinned by a control fence in `macros/src/lib.rs`'s own doc comment, where it
runs in the real gate. This directory pins only the compiler fact underneath it.
