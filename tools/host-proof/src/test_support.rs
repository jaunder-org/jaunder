//! Unit-test fixtures.

use std::path::Path;

use crate::traces::run::{TRACE_MEMBER, instance_diag_member};

/// Write a `capture-*.tar.gz` shaped like a VM capture: the given trace beside
/// one server instance's `diag.log`, which every real capture carries (#2029,
/// #2076). `None` omits the log, for the fail-closed case.
pub fn write_capture(path: &Path, trace: &str, diag_log: Option<&str>) {
    let file = std::fs::File::create(path).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    let diag_member = instance_diag_member(0);
    let diag = diag_log.map(|log| (diag_member.as_str(), log));
    for (member, contents) in std::iter::once((TRACE_MEMBER, trace)).chain(diag) {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len().try_into().unwrap());
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, member, contents.as_bytes())
            .unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
}

/// A fn wearing the **retired** leptos `#[server]` spelling (#714). Assembled
/// from pieces so a grep for the retired attribute at the start of a fixture
/// literal stays a clean zero; this deliberate negative fixture lives here, named.
pub fn retired_server_fn(attr_args: &str, item: &str) -> String {
    format!("#[{}{attr_args}]\n{item}\n", "server")
}
