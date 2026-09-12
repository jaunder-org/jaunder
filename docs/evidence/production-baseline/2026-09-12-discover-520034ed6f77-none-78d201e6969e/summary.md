# Production baseline discover

- Outcome: Passed
- Harness commit: `78d201e6969ed17b8a54b87d3de24be6afa1f5ee`
- Source: `520034ed6f778a854cd5f8708419c1b2b700ad9f`
- Manifests: seeded v1 `18778e4aa1411fdb784cc689c15dd27fcb5e0049ca4b0452368c198de759f831`, operation v1 `b2521f1f228babf6690ae2ba063c0f394d4e578c4b61e81bb2c0864e0627a534`

## Runtime identities

- deployment `discover-source-sqlite`; backend Sqlite; revision `520034ed6f778a854cd5f8708419c1b2b700ad9f`; executable `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Sat 2026-09-12 17:17:46 UTC] ; stop_time=[n/a] ; pid=836 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/520034ed6f778a854cd5f8708419c1b2b700ad9f#jaunder`; derivation `/nix/store/hbarxa3lfdypspr9vb521l668jwqz5dn-jaunder-0.1.0.drv`; output `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0`; NAR `sha256-Ild/LocnNHExFGYk08WMy2ei/kFwzVxQQR2y0CiJEok=`; SHA-256 `fbfdd65242ba35b4fd8aad574756526dd56a5a65c7358e4fb53d9ca2187bc3c8`
- deployment `discover-target-discover-source-sqlite-sqlite`; backend Sqlite; revision `520034ed6f778a854cd5f8708419c1b2b700ad9f`; executable `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Sat 2026-09-12 17:19:42 UTC] ; stop_time=[n/a] ; pid=814 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/520034ed6f778a854cd5f8708419c1b2b700ad9f#jaunder`; derivation `/nix/store/hbarxa3lfdypspr9vb521l668jwqz5dn-jaunder-0.1.0.drv`; output `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0`; NAR `sha256-Ild/LocnNHExFGYk08WMy2ei/kFwzVxQQR2y0CiJEok=`; SHA-256 `fbfdd65242ba35b4fd8aad574756526dd56a5a65c7358e4fb53d9ca2187bc3c8`
- deployment `discover-target-discover-source-sqlite-postgres`; backend Postgres; revision `520034ed6f778a854cd5f8708419c1b2b700ad9f`; executable `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Sat 2026-09-12 17:21:24 UTC] ; stop_time=[n/a] ; pid=833 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/520034ed6f778a854cd5f8708419c1b2b700ad9f#jaunder`; derivation `/nix/store/hbarxa3lfdypspr9vb521l668jwqz5dn-jaunder-0.1.0.drv`; output `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0`; NAR `sha256-Ild/LocnNHExFGYk08WMy2ei/kFwzVxQQR2y0CiJEok=`; SHA-256 `fbfdd65242ba35b4fd8aad574756526dd56a5a65c7358e4fb53d9ca2187bc3c8`
- deployment `discover-source-postgres`; backend Postgres; revision `520034ed6f778a854cd5f8708419c1b2b700ad9f`; executable `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Sat 2026-09-12 17:22:33 UTC] ; stop_time=[n/a] ; pid=840 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/520034ed6f778a854cd5f8708419c1b2b700ad9f#jaunder`; derivation `/nix/store/hbarxa3lfdypspr9vb521l668jwqz5dn-jaunder-0.1.0.drv`; output `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0`; NAR `sha256-Ild/LocnNHExFGYk08WMy2ei/kFwzVxQQR2y0CiJEok=`; SHA-256 `fbfdd65242ba35b4fd8aad574756526dd56a5a65c7358e4fb53d9ca2187bc3c8`
- deployment `discover-target-discover-source-postgres-sqlite`; backend Sqlite; revision `520034ed6f778a854cd5f8708419c1b2b700ad9f`; executable `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Sat 2026-09-12 17:23:47 UTC] ; stop_time=[n/a] ; pid=793 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/520034ed6f778a854cd5f8708419c1b2b700ad9f#jaunder`; derivation `/nix/store/hbarxa3lfdypspr9vb521l668jwqz5dn-jaunder-0.1.0.drv`; output `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0`; NAR `sha256-Ild/LocnNHExFGYk08WMy2ei/kFwzVxQQR2y0CiJEok=`; SHA-256 `fbfdd65242ba35b4fd8aad574756526dd56a5a65c7358e4fb53d9ca2187bc3c8`
- deployment `discover-target-discover-source-postgres-postgres`; backend Postgres; revision `520034ed6f778a854cd5f8708419c1b2b700ad9f`; executable `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Sat 2026-09-12 17:24:54 UTC] ; stop_time=[n/a] ; pid=846 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/520034ed6f778a854cd5f8708419c1b2b700ad9f#jaunder`; derivation `/nix/store/hbarxa3lfdypspr9vb521l668jwqz5dn-jaunder-0.1.0.drv`; output `/nix/store/p8rmfih7jci5rd527pi4lwnnhjnzpl0i-jaunder-0.1.0`; NAR `sha256-Ild/LocnNHExFGYk08WMy2ei/kFwzVxQQR2y0CiJEok=`; SHA-256 `fbfdd65242ba35b4fd8aad574756526dd56a5a65c7358e4fb53d9ca2187bc3c8`

## Backups

- Sqlite → Sqlite; format 1; source schema 36; target schema 36; SHA-256 `d971b25771296c28ba42fa0496354a70b5a6d9d1b5f36c3f641e6d2a1d7b0040`
- Sqlite → Postgres; format 1; source schema 36; target schema 36; SHA-256 `d971b25771296c28ba42fa0496354a70b5a6d9d1b5f36c3f641e6d2a1d7b0040`
- Postgres → Sqlite; format 1; source schema 36; target schema 36; SHA-256 `db3f21c9420011a5bde8984329e182fb39cd726008fc191018c6fca569abc018`
- Postgres → Postgres; format 1; source schema 36; target schema 36; SHA-256 `db3f21c9420011a5bde8984329e182fb39cd726008fc191018c6fca569abc018`

## Package activations


## Failure classes


## Lifecycle

| ID | Outcome | Duration (ms) |
| --- | --- | ---: |
| workspace-create | Passed | 0 |
| discover-source-sqlite-start | Passed | 135253 |
| discover-source-sqlite-configure-base-url | Passed | 235 |
| discover-source-sqlite-verify-proxy-security | Passed | 830 |
| discover-target-discover-source-sqlite-sqlite-start | Passed | 39998 |
| discover-target-discover-source-sqlite-sqlite-park | Passed | 4692 |
| discover-target-discover-source-sqlite-postgres-start | Passed | 61007 |
| discover-target-discover-source-sqlite-postgres-park | Passed | 2756 |
| discover-source-sqlite-park | Passed | 2505 |
| discover-source-postgres-start | Passed | 40750 |
| discover-source-postgres-configure-base-url | Passed | 76 |
| discover-source-postgres-verify-proxy-security | Passed | 251 |
| discover-target-discover-source-postgres-sqlite-start | Passed | 21997 |
| discover-target-discover-source-postgres-sqlite-park | Passed | 3925 |
| discover-target-discover-source-postgres-postgres-start | Passed | 42557 |
| discover-target-discover-source-postgres-postgres-park | Passed | 5653 |
| discover-source-postgres-park | Passed | 7662 |

## Checks

| ID | Outcome | Duration (ms) |
| --- | --- | ---: |
| browser-create | Passed | 8356 |
| atompub | Passed | 50517 |
| feeds | Passed | 50517 |
| service-restart | Passed | 417 |
| browser-read-only | Passed | 42161 |
| vm-reboot | Passed | 52942 |
| backup | Passed | 385 |
| restore-sqlite-sqlite | Passed | 1576 |
| restore-sqlite-postgres | Passed | 877 |
| restore-postgres-sqlite | Passed | 1059 |
| restore-postgres-postgres | Passed | 2036 |
| upgrade | Skipped | 0 |

## Gaps


## Findings
