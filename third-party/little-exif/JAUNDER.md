# Maintained little_exif dependency

Jaunder maintains this local source dependency until a reviewed upstream release
provides the checked HEIF metadata extent API and the XML security update. No
external fork or package publication is required to build or deploy Jaunder.

## Provenance and grants

The base is the crates.io `little_exif` 0.6.23 archive, SHA256
`21eeb58b22d31be8dc5c625004fcd4b9b385cd3c05df575f523bcca382c51122`, from
upstream revision `2d6d73acea033630a1cf0056d23b9da81e0638c8`. Its authorship,
source notices and `MIT OR Apache-2.0` grant remain intact. `LICENSE-MIT` and
`LICENSE-APACHE` are verbatim upstream license files. Jaunder's additions use
the repository's existing `GPL-3.0-only` grant, recorded separately in
`LICENSE-JAUNDER`; the maintained package declares
`(MIT OR Apache-2.0) AND GPL-3.0-only`. This does not relicense upstream's code
or assert new contribution/relicensing authority. `Cargo.toml.orig` retains the
original published manifest for comparison.

## Patch boundary

The added `Metadata::heif_metadata_regions` API exposes typed EXIF, XMP and item
name extents. It checks container bounds, containment, item identities and
nonempty overlap/image-alias risks. Unsupported metadata descriptors and
locations fail closed. The caller replaces metadata in place, without invoking
the generic container writer or changing encoded image items.

The dependency upgrades `quick-xml` from 0.37.5 to 0.41.0 to address
RUSTSEC-2026-0194 and RUSTSEC-2026-0195. Fixture construction helpers are not
included in this maintained source. Upstream code outside this patch retains its
upstream conventions; first-party callers and added behavior remain subject to
Jaunder's standards and regression proof.

The host doctest gate explicitly includes this excluded local dependency and
reconciles its examples against execution. Its eight examples run rather than
using `no_run`; file examples use private owned-fixture copies. Macro-generated
method examples live on the module so the source scanner and rustdoc agree on
one population. The standalone `Cargo.lock` pins this host proof's dependencies.

## Ownership and updates

Jaunder maintainers own the local patch. Every update must compare against the
identified upstream release, preserve notices and grants, run the native
sanitizer's owned-format, rendering, privacy, malformed-container and extent
regressions, and check dependency advisories/licenses. Pin the replacement
release exactly in `host/Cargo.toml`; remove this tree only after that release
passes the same independent observers and ingress/worker acceptance tests.

This local maintenance home is not a claim of exhaustive HEIF compatibility or
compressed-stream validation. Publishing a fork or contributing this patch to
another repository requires separate owner authorization.
