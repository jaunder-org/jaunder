# Theme conformance fixture provenance

`roboto-regular.woff2` is copied byte-for-byte from Jaunder's existing compiler
validation fixture at `host/src/theme_package/fixtures/roboto-regular.woff2`
(commit `994028ce4`, which introduced the Theme Package compiler). It is used
only as a local, valid WOFF2 test input; no network fetch or theme repository
asset is involved.

The two raster package assets are deterministic one-pixel PNG byte literals in
`../../theme-helpers.ts`, matching the existing media/theme E2E fixture form.
