# Production baseline discover

- Outcome: Passed
- Harness commit: `8db83f585188968545503b3808a0e58e6ac6a299`
- Source: `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`
- Manifests: seeded v1 `5c20b51c71d6a835a194c395963ffe998b80aadd6dbd551a417d1db503c9b2a0`, operation v1 `85ff493e738c4379f91eaef9604fae656540cad2aed28222b83459116cd222be`

## Runtime identities

- deployment `discover-source-sqlite`; backend Sqlite; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 20:42:18 UTC] ; stop_time=[n/a] ; pid=767 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-target-discover-source-sqlite-sqlite`; backend Sqlite; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 20:43:13 UTC] ; stop_time=[n/a] ; pid=765 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-target-discover-source-sqlite-postgres`; backend Postgres; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 20:43:56 UTC] ; stop_time=[n/a] ; pid=819 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-source-postgres`; backend Postgres; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 20:44:30 UTC] ; stop_time=[n/a] ; pid=813 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-target-discover-source-postgres-sqlite`; backend Sqlite; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 20:45:23 UTC] ; stop_time=[n/a] ; pid=770 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-target-discover-source-postgres-postgres`; backend Postgres; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 20:45:54 UTC] ; stop_time=[n/a] ; pid=819 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`

## Backups

- Sqlite → Sqlite; format 1; source schema 36; target schema 36; SHA-256 `b1f2bf91c3059c4da986dae31721f94d6b39fcec46689778faa258812a800f2c`
- Sqlite → Postgres; format 1; source schema 36; target schema 36; SHA-256 `b1f2bf91c3059c4da986dae31721f94d6b39fcec46689778faa258812a800f2c`
- Postgres → Sqlite; format 1; source schema 36; target schema 36; SHA-256 `de9d551700356d3f9f876e3493c63e9d20dcda14c616bc823cbb48e54473a317`
- Postgres → Postgres; format 1; source schema 36; target schema 36; SHA-256 `de9d551700356d3f9f876e3493c63e9d20dcda14c616bc823cbb48e54473a317`

## Package activations


## Failure classes


## Lifecycle

| ID | Outcome | Duration (ms) |
| --- | --- | ---: |
| workspace-create | Passed | 0 |
| discover-source-sqlite-start | Passed | 39606 |
| discover-source-sqlite-configure-base-url | Passed | 57 |
| discover-source-sqlite-verify-proxy-security | Passed | 185 |
| discover-target-discover-source-sqlite-sqlite-start | Passed | 18456 |
| discover-target-discover-source-sqlite-sqlite-park | Passed | 1953 |
| discover-target-discover-source-sqlite-postgres-start | Passed | 30516 |
| discover-target-discover-source-sqlite-postgres-park | Passed | 2204 |
| discover-source-sqlite-park | Passed | 2053 |
| discover-source-postgres-start | Passed | 20226 |
| discover-source-postgres-configure-base-url | Passed | 67 |
| discover-source-postgres-verify-proxy-security | Passed | 195 |
| discover-target-discover-source-postgres-sqlite-start | Passed | 18518 |
| discover-target-discover-source-postgres-sqlite-park | Passed | 2053 |
| discover-target-discover-source-postgres-postgres-start | Passed | 18538 |
| discover-target-discover-source-postgres-postgres-park | Passed | 2153 |
| discover-source-postgres-park | Passed | 2153 |

## Checks

| ID | Outcome | Duration (ms) |
| --- | --- | ---: |
| browser-create | Passed | 3283 |
| atompub | Passed | 18965 |
| feeds | Passed | 18965 |
| service-restart | Passed | 228 |
| browser-read-only | Passed | 15682 |
| vm-reboot | Passed | 36567 |
| backup | Passed | 208 |
| restore-sqlite-sqlite | Passed | 462 |
| restore-sqlite-postgres | Passed | 654 |
| restore-postgres-sqlite | Passed | 467 |
| restore-postgres-postgres | Passed | 639 |
| upgrade | Skipped | 0 |

## Gaps


## Findings
