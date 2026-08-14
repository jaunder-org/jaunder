#[path = "../src/build_staging.rs"]
mod build_impl;

use std::cell::Cell;
use std::error::Error as _;
use std::io;
use std::path::Path;

#[test]
fn build_script_staging_cleanup_failure_aborts_before_create_or_copy() {
    let created = Cell::new(false);
    let copied = Cell::new(false);
    let site = Path::new("/injected/out/site");

    let error = build_impl::prepare_staging_with(
        site,
        |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected remove failure",
            ))
        },
        |_| {
            created.set(true);
            Ok(())
        },
        || copied.set(true),
    )
    .unwrap_err();

    assert_eq!(error.operation, "removing");
    assert_eq!(error.path, site);
    assert_eq!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<io::Error>())
            .map(io::Error::kind),
        Some(io::ErrorKind::PermissionDenied)
    );
    assert!(!created.get(), "staging directory must not be recreated");
    assert!(!copied.get(), "assets must not be copied");
}
