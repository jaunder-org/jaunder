//! The private same-executable worker invocation, before CLI or telemetry setup.
//!
//! Spool paths stay native OS strings. Neither metadata nor the original upload
//! filename belongs in the invocation or its diagnostics.

use std::ffi::OsString;
use std::path::PathBuf;
use thiserror::Error;

pub const WORKER_FLAG: &str = "--internal-image-metadata-worker";

#[derive(Debug, PartialEq, Eq)]
pub struct WorkerRequest {
    pub path: PathBuf,
    pub claims_raster: bool,
    pub max_bytes: usize,
}

#[derive(Debug, Error)]
pub enum RequestError {
    #[error("invalid image worker invocation")]
    Shape,
    #[error("invalid image worker raster claim")]
    Claim,
    #[error("invalid image worker size limit")]
    LimitEncoding,
    #[error("invalid image worker size limit")]
    Limit(#[from] std::num::ParseIntError),
}

/// Recognizes the internal invocation without interpreting normal CLI arguments.
/// Arguments exclude argv0; exactly one spool, claim and limit follow the flag.
///
/// # Errors
/// Returns typed argument rejection without echoing argument content.
pub fn parse(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Option<WorkerRequest>, RequestError> {
    let mut args = args.into_iter();
    if args.next().as_deref() != Some(std::ffi::OsStr::new(WORKER_FLAG)) {
        return Ok(None);
    }
    let path = args.next().ok_or(RequestError::Shape)?;
    let claim = args.next().ok_or(RequestError::Shape)?;
    let limit = args.next().ok_or(RequestError::Shape)?;
    if args.next().is_some() {
        return Err(RequestError::Shape);
    }
    let claims_raster = if claim == "1" {
        true
    } else if claim == "0" {
        false
    } else {
        return Err(RequestError::Claim);
    };
    let max_bytes = limit.to_str().ok_or(RequestError::LimitEncoding)?.parse()?;
    Ok(Some(WorkerRequest {
        path: path.into(),
        claims_raster,
        max_bytes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    #[test]
    fn native_spool_path_survives_internal_invocation() {
        let path = OsString::from_vec(b"/private/spool-\xff".to_vec());
        let request = parse([
            WORKER_FLAG.into(),
            path.clone(),
            "1".into(),
            "1048576".into(),
        ])
        .expect("internal arguments")
        .expect("worker request");
        assert_eq!(request.path.as_os_str().as_bytes(), path.as_bytes());
        assert!(request.claims_raster);
        assert_eq!(request.max_bytes, 1_048_576);
    }

    #[test]
    fn malformed_worker_arguments_fail_without_echoing_private_content() {
        use std::error::Error;
        let marker = "private-owner@example.test";
        assert!(matches!(
            parse([WORKER_FLAG.into()]),
            Err(RequestError::Shape)
        ));
        assert!(matches!(
            parse([
                WORKER_FLAG.into(),
                marker.into(),
                "0".into(),
                "1".into(),
                "extra".into()
            ]),
            Err(RequestError::Shape)
        ));
        assert!(matches!(
            parse([WORKER_FLAG.into(), marker.into(), marker.into(), "1".into()]),
            Err(RequestError::Claim)
        ));
        assert!(matches!(
            parse([
                WORKER_FLAG.into(),
                marker.into(),
                "0".into(),
                OsString::from_vec(vec![0xff])
            ]),
            Err(RequestError::LimitEncoding)
        ));
        let error = parse([WORKER_FLAG.into(), marker.into(), "0".into(), marker.into()])
            .expect_err("invalid numeric limit");
        assert!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<std::num::ParseIntError>())
                .is_some()
        );
        assert!(!format!("{error:?} {error}").contains(marker));
    }

    #[test]
    fn unclaimed_attachment_invocation_is_explicit() {
        let request = parse([
            WORKER_FLAG.into(),
            "/private/spool".into(),
            "0".into(),
            "4096".into(),
        ])
        .expect("arguments")
        .expect("worker mode");
        assert!(!request.claims_raster);
        assert_eq!(request.max_bytes, 4096);
    }

    #[test]
    fn normal_cli_is_not_worker_mode() {
        assert_eq!(
            parse(["serve".into(), "--prod".into()]).expect("normal CLI"),
            None
        );
        assert_eq!(parse([]).expect("no command"), None);
    }
}
