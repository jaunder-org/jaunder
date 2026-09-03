use macros::NumNewtype;
use serde::{Deserialize, Serialize};

use crate::root_relative_url::RootRelativeUrl;

use super::{ContentHash, ContentType, Filename};

/// The maximum accepted upload size, in bytes (site config `media.max_file_size_bytes`).
/// A positive `i64` — a zero/negative limit is nonsensical — enforced by the
/// `NumNewtype`-generated validating `FromStr`/serde. Default 50 MiB.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 1,
    default = 52_428_800, // 50 MiB
    error = "media max file size must be a positive number of bytes"
)]
pub struct MaxFileSize(i64);

/// The per-user upload quota, in bytes (site config `media.user_quota_bytes`).
/// A positive `i64`, like [`MaxFileSize`]; a distinct type so a per-file limit and a
/// per-user quota can't be transposed. Default 1 GiB.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 1,
    default = 1_073_741_824, // 1 GiB
    error = "media user quota must be a positive number of bytes"
)]
pub struct UserQuota(i64);

/// A non-negative count of bytes — a *measured/stored* size (a media file's byte length,
/// a user's total upload usage), the actual-value counterpart to the [`MaxFileSize`] /
/// [`UserQuota`] *limits*. `min = 0` (an empty object is 0 bytes) and no `default` (it is
/// measured, never a config fallback). Unlike the limits — which are only ever built from
/// config strings — a `ByteSize` is built from a runtime `i64` (a DB column, a `SUM`), so it
/// relies on the `NumNewtype` validating `TryFrom<i64>` door.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = i64,
    min = 0,
    error = "byte size must be a non-negative number of bytes"
)]
pub struct ByteSize(i64);

/// Metadata for a successfully stored upload — the server-fn wire value (#517), living
/// here (not in `server`) so it is nameable on the wasm client. `storage`'s
/// `MediaManager` and `web`'s `media::upload` return it directly; `AtomPub` consumes its
/// identity to load and serialize the stored record. Every field is a validated `common`
/// newtype, so each re-validates on deserialize — including `url`, the derived serve path,
/// which is a
/// [`RootRelativeUrl`][crate::root_relative_url::RootRelativeUrl] because being *derived*
/// is not a reason to leave it stringly (ADR-0063 §5), and because the derivation is only
/// well-formed thanks to [`path`]'s encoding, which the type is what pins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedMedia {
    pub sha256: ContentHash,
    pub filename: Filename,
    pub content_type: ContentType,
    pub size_bytes: ByteSize,
    pub url: RootRelativeUrl,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the whole `NumNewtype`-generated surface of a positive-`i64` (min-1) byte
    /// newtype `T`: parse accept/trim, reject `0`/negative/non-integer with the domain
    /// message, `Default`, `Display` round-trip, `From<Self> for i64`, and the
    /// transparent-`i64` serde bridge (round-trip + wire-rejection of `0`). Both byte-limit
    /// types share this shape, so one generic assertion replaces two near-identical tests.
    /// The DTO does not serde these on the host build, so this is the reachability for that
    /// generated code. Written via `From`/`.ok()`/`.err()` (no `unwrap`), so it needs no
    /// lint exception.
    fn assert_positive_byte_newtype<T>(default: i64, err_prefix: &str)
    where
        T: ::core::str::FromStr
            + ::core::default::Default
            + ::core::fmt::Display
            + ::core::fmt::Debug
            + ::core::marker::Copy
            + ::core::cmp::PartialEq
            + ::serde::Serialize
            + ::serde::de::DeserializeOwned,
        T::Err: ::core::fmt::Display,
        i64: ::core::convert::From<T>,
    {
        // parse accepts and trims
        assert_eq!("5".parse::<T>().map(i64::from).ok(), Some(5));
        assert_eq!("  100  ".parse::<T>().map(i64::from).ok(), Some(100));
        // parse rejects 0, negatives, and non-integers...
        for bad in ["0", "-1", "abc", "1.5"] {
            assert!(bad.parse::<T>().is_err(), "{bad} should reject");
        }
        // ...with the domain message
        assert!(
            "0".parse::<T>()
                .err()
                .is_some_and(|e| e.to_string().starts_with(err_prefix))
        );
        // Default, and From<Self> for i64
        let d = T::default();
        assert_eq!(i64::from(d), default);
        // Display round-trips through FromStr
        assert_eq!(d.to_string().parse::<T>().ok(), Some(d));
        // serde: bare integer, round-trip, wire-rejection of 0
        assert_eq!(serde_json::to_string(&d).ok(), Some(default.to_string()));
        assert_eq!(
            serde_json::from_str::<T>("42").map(i64::from).ok(),
            Some(42)
        );
        assert!(serde_json::from_str::<T>("0").is_err());
    }

    #[test]
    fn max_file_size_surface() {
        assert_positive_byte_newtype::<MaxFileSize>(52_428_800, "media max file size");
    }

    #[test]
    fn user_quota_surface() {
        assert_positive_byte_newtype::<UserQuota>(1_073_741_824, "media user quota");
    }

    #[test]
    fn byte_size_surface() {
        // `ByteSize` has its own test — it is min-0 (accepts `0`) and has no `default`, so it
        // cannot use `assert_positive_byte_newtype` (min-1, `Default`-requiring). Drives every
        // generated branch for coverage.
        assert_eq!("0".parse::<ByteSize>().map(i64::from).ok(), Some(0));
        assert_eq!(
            "  2048  ".parse::<ByteSize>().map(i64::from).ok(),
            Some(2048)
        );
        for bad in ["-1", "abc", "1.5"] {
            assert!(bad.parse::<ByteSize>().is_err(), "{bad} should reject");
        }
        assert!(
            "-1".parse::<ByteSize>()
                .err()
                .is_some_and(|e| e.to_string().starts_with("byte size"))
        );
        // Display round-trips through FromStr
        let b = "4096".parse::<ByteSize>().unwrap();
        assert_eq!(b.to_string().parse::<ByteSize>().ok(), Some(b));
        // From<Self> for i64
        assert_eq!(i64::from(b), 4096);
        // serde: transparent integer, round-trip, and wire-rejection of a *negative* (the
        // deserialize min-guard arm — `0` is accepted, so a negative is what reaches it)
        assert_eq!(serde_json::to_string(&b).ok(), Some("4096".to_string()));
        assert_eq!(
            serde_json::from_str::<ByteSize>("11").map(i64::from).ok(),
            Some(11)
        );
        assert!(serde_json::from_str::<ByteSize>("-1").is_err());
        // the new validating `TryFrom<i64>` door — accept 0, reject negative
        assert_eq!(ByteSize::try_from(0i64).map(i64::from).ok(), Some(0));
        assert!(ByteSize::try_from(-1i64).is_err());
    }
}
