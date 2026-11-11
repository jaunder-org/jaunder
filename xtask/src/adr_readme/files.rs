//! Shared numbered-ADR parser for the retained parity consumers.
#[cfg(test)]
pub(super) use repository_policy::adr::{ADR_DIR, adr_files_from, format_problems};
pub(super) use repository_policy::adr::{
    AdrEntry, AdrFile, adr_files, adr_link, parse_adr_dir, parse_adr_files,
};
