# Production baseline accept

- Outcome: Passed
- Harness commit: `d928dae715271f7ac1904dbe6f1492c2eb77bb56`
- Source: `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`
- Manifests: seeded v1 `5c20b51c71d6a835a194c395963ffe998b80aadd6dbd551a417d1db503c9b2a0`, operation v1 `85ff493e738c4379f91eaef9604fae656540cad2aed28222b83459116cd222be`
- Target: `e7ac9c02fbc6418239c62cca5072713b4cdbee15`

## Runtime identities

- deployment `accept-source-sqlite-package-source`; backend Sqlite; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 21:24:14 UTC] ; stop_time=[n/a] ; pid=711 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-source-sqlite-package-target`; backend Sqlite; revision `e7ac9c02fbc6418239c62cca5072713b4cdbee15`; executable `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 21:25:25 UTC] ; stop_time=[n/a] ; pid=701 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/e7ac9c02fbc6418239c62cca5072713b4cdbee15#jaunder`; derivation `/nix/store/3fpgsqnaccplwahfxjd7sbsy5z7h76dn-jaunder-0.1.0.drv`; output `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0`; NAR `sha256-BHc0M0V7IIacdu6zAIYvfqoXCnX/m0PV6BHwJpm0UZE=`; SHA-256 `6844c4436dfd30f8a7806fbdf9220d2df0819a2962e377676a4a5bbb5a72f29e`
- deployment `accept-target-accept-source-sqlite-sqlite-package-target`; backend Sqlite; revision `e7ac9c02fbc6418239c62cca5072713b4cdbee15`; executable `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 21:25:58 UTC] ; stop_time=[n/a] ; pid=712 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/e7ac9c02fbc6418239c62cca5072713b4cdbee15#jaunder`; derivation `/nix/store/3fpgsqnaccplwahfxjd7sbsy5z7h76dn-jaunder-0.1.0.drv`; output `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0`; NAR `sha256-BHc0M0V7IIacdu6zAIYvfqoXCnX/m0PV6BHwJpm0UZE=`; SHA-256 `6844c4436dfd30f8a7806fbdf9220d2df0819a2962e377676a4a5bbb5a72f29e`
- deployment `accept-target-accept-source-sqlite-postgres-package-target`; backend Postgres; revision `e7ac9c02fbc6418239c62cca5072713b4cdbee15`; executable `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 21:26:44 UTC] ; stop_time=[n/a] ; pid=819 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/e7ac9c02fbc6418239c62cca5072713b4cdbee15#jaunder`; derivation `/nix/store/3fpgsqnaccplwahfxjd7sbsy5z7h76dn-jaunder-0.1.0.drv`; output `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0`; NAR `sha256-BHc0M0V7IIacdu6zAIYvfqoXCnX/m0PV6BHwJpm0UZE=`; SHA-256 `6844c4436dfd30f8a7806fbdf9220d2df0819a2962e377676a4a5bbb5a72f29e`
- deployment `accept-source-postgres-package-source`; backend Postgres; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 21:27:29 UTC] ; stop_time=[n/a] ; pid=820 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-source-postgres-package-target`; backend Postgres; revision `e7ac9c02fbc6418239c62cca5072713b4cdbee15`; executable `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 21:28:19 UTC] ; stop_time=[n/a] ; pid=751 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/e7ac9c02fbc6418239c62cca5072713b4cdbee15#jaunder`; derivation `/nix/store/3fpgsqnaccplwahfxjd7sbsy5z7h76dn-jaunder-0.1.0.drv`; output `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0`; NAR `sha256-BHc0M0V7IIacdu6zAIYvfqoXCnX/m0PV6BHwJpm0UZE=`; SHA-256 `6844c4436dfd30f8a7806fbdf9220d2df0819a2962e377676a4a5bbb5a72f29e`
- deployment `accept-target-accept-source-postgres-sqlite-package-target`; backend Sqlite; revision `e7ac9c02fbc6418239c62cca5072713b4cdbee15`; executable `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 21:28:53 UTC] ; stop_time=[n/a] ; pid=767 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/e7ac9c02fbc6418239c62cca5072713b4cdbee15#jaunder`; derivation `/nix/store/3fpgsqnaccplwahfxjd7sbsy5z7h76dn-jaunder-0.1.0.drv`; output `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0`; NAR `sha256-BHc0M0V7IIacdu6zAIYvfqoXCnX/m0PV6BHwJpm0UZE=`; SHA-256 `6844c4436dfd30f8a7806fbdf9220d2df0819a2962e377676a4a5bbb5a72f29e`
- deployment `accept-target-accept-source-postgres-postgres-package-target`; backend Postgres; revision `e7ac9c02fbc6418239c62cca5072713b4cdbee15`; executable `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 21:29:25 UTC] ; stop_time=[n/a] ; pid=816 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/e7ac9c02fbc6418239c62cca5072713b4cdbee15#jaunder`; derivation `/nix/store/3fpgsqnaccplwahfxjd7sbsy5z7h76dn-jaunder-0.1.0.drv`; output `/nix/store/r64179hz02c8vjdb2h05k1i20rgdb88s-jaunder-0.1.0`; NAR `sha256-BHc0M0V7IIacdu6zAIYvfqoXCnX/m0PV6BHwJpm0UZE=`; SHA-256 `6844c4436dfd30f8a7806fbdf9220d2df0819a2962e377676a4a5bbb5a72f29e`

## Backups

- Sqlite → Sqlite; format 1; source schema 36; target schema 36; SHA-256 `fb61045ea8785ab12c57ce622c1b0ba93103ae0c0d722f9a6a91a4b62b89fe0f`
- Sqlite → Postgres; format 1; source schema 36; target schema 36; SHA-256 `fb61045ea8785ab12c57ce622c1b0ba93103ae0c0d722f9a6a91a4b62b89fe0f`
- Postgres → Sqlite; format 1; source schema 36; target schema 36; SHA-256 `9aa84c68582df76cd853f6cef8bbe7c4099086587c9fb97cf88ecb4549de87ac`
- Postgres → Postgres; format 1; source schema 36; target schema 36; SHA-256 `9aa84c68582df76cd853f6cef8bbe7c4099086587c9fb97cf88ecb4549de87ac`

## Package activations

- Sqlite; source SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`; target SHA-256 `6844c4436dfd30f8a7806fbdf9220d2df0819a2962e377676a4a5bbb5a72f29e`; binary changed true; source schema 36; target schema 36; schema migrated false
- Postgres; source SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`; target SHA-256 `6844c4436dfd30f8a7806fbdf9220d2df0819a2962e377676a4a5bbb5a72f29e`; binary changed true; source schema 36; target schema 36; schema migrated false

## Failure classes


## Lifecycle

| ID | Outcome | Duration (ms) |
| --- | --- | ---: |
| workspace-create | Passed | 0 |
| accept-source-sqlite-start | Passed | 41584 |
| accept-source-sqlite-configure-base-url | Passed | 57 |
| accept-source-sqlite-verify-proxy-security | Passed | 197 |
| accept-source-sqlite-source-schema | Passed | 79 |
| accept-source-sqlite-target-schema | Passed | 75 |
| accept-target-accept-source-sqlite-sqlite-start | Passed | 18523 |
| accept-target-accept-source-sqlite-sqlite-park | Passed | 2003 |
| accept-target-accept-source-sqlite-postgres-start | Passed | 31828 |
| accept-target-accept-source-sqlite-postgres-park | Passed | 2154 |
| accept-source-sqlite-park | Passed | 2003 |
| accept-source-postgres-start | Passed | 29733 |
| accept-source-postgres-configure-base-url | Passed | 63 |
| accept-source-postgres-verify-proxy-security | Passed | 172 |
| accept-source-postgres-source-schema | Passed | 122 |
| accept-source-postgres-target-schema | Passed | 120 |
| accept-target-accept-source-postgres-sqlite-start | Passed | 18507 |
| accept-target-accept-source-postgres-sqlite-park | Passed | 2053 |
| accept-target-accept-source-postgres-postgres-start | Passed | 18530 |
| accept-target-accept-source-postgres-postgres-park | Passed | 2103 |
| accept-source-postgres-park | Passed | 2055 |

## Checks

| ID | Outcome | Duration (ms) |
| --- | --- | ---: |
| browser-create | Passed | 3312 |
| atompub | Passed | 27833 |
| feeds | Passed | 27833 |
| service-restart | Passed | 215 |
| browser-read-only | Passed | 24521 |
| vm-reboot | Passed | 35917 |
| upgrade | Passed | 54785 |
| backup | Passed | 137 |
| restore-sqlite-sqlite | Passed | 483 |
| restore-sqlite-postgres | Passed | 665 |
| restore-postgres-sqlite | Passed | 518 |
| restore-postgres-postgres | Passed | 645 |

## Gaps


## Findings
