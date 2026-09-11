# Production baseline discover

- Outcome: Passed
- Harness commit: `180760eae4bd00c6d720e9b35837a6bec44f5e90`
- Source: `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`
- Manifests: seeded v1
  `59dcc0c761795d7f4bba666e938b5f90571075e2f4bc59620fa82002ac42986f`, operation
  v1 `24c6bb6542a3cc4efd27b7ca224a7b6d7fbb16d22653236f0399458cc3255434`

## Runtime identities

- deployment `discover-source-sqlite`; backend Sqlite; revision
  `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable
  `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 15:13:19 UTC] ; stop_time=[n/a] ; pid=726 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`;
  derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`;
  output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-target-discover-source-sqlite-sqlite`; backend Sqlite;
  revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable
  `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 15:14:34 UTC] ; stop_time=[n/a] ; pid=782 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`;
  derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`;
  output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-target-discover-source-sqlite-postgres`; backend
  Postgres; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable
  `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 15:15:27 UTC] ; stop_time=[n/a] ; pid=823 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`;
  derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`;
  output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-source-postgres`; backend Postgres; revision
  `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable
  `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 15:15:56 UTC] ; stop_time=[n/a] ; pid=815 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`;
  derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`;
  output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-target-discover-source-postgres-sqlite`; backend Sqlite;
  revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable
  `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 15:16:50 UTC] ; stop_time=[n/a] ; pid=779 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`;
  derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`;
  output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `discover-target-discover-source-postgres-postgres`; backend
  Postgres; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable
  `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 15:17:22 UTC] ; stop_time=[n/a] ; pid=818 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`;
  derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`;
  output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`

## Backups

- Sqlite → Sqlite; format 1; source schema 36; target schema 36; SHA-256
  `15a0beeb31a211cd34375c4bd4e82d892b2bad09d49b70983cde490f1ff83eeb`
- Sqlite → Postgres; format 1; source schema 36; target schema 36; SHA-256
  `15a0beeb31a211cd34375c4bd4e82d892b2bad09d49b70983cde490f1ff83eeb`
- Postgres → Sqlite; format 1; source schema 36; target schema 36; SHA-256
  `2def47b6e1efc9b37893e819e0ec06e0e221898df3e88d8cb0594378ac1be569`
- Postgres → Postgres; format 1; source schema 36; target schema 36; SHA-256
  `2def47b6e1efc9b37893e819e0ec06e0e221898df3e88d8cb0594378ac1be569`

## Package activations

## Failure classes

## Lifecycle

| ID                                                      | Outcome | Duration (ms) |
| ------------------------------------------------------- | ------- | ------------: |
| workspace-create                                        | Passed  |             0 |
| discover-source-sqlite-start                            | Passed  |         42195 |
| discover-source-sqlite-configure-base-url               | Passed  |            66 |
| discover-target-discover-source-sqlite-sqlite-start     | Passed  |         26855 |
| discover-target-discover-source-sqlite-sqlite-park      | Passed  |          2103 |
| discover-target-discover-source-sqlite-postgres-start   | Passed  |         35867 |
| discover-target-discover-source-sqlite-postgres-park    | Passed  |          2153 |
| discover-source-sqlite-park                             | Passed  |          2053 |
| discover-source-postgres-start                          | Passed  |         18512 |
| discover-source-postgres-configure-base-url             | Passed  |            58 |
| discover-target-discover-source-postgres-sqlite-start   | Passed  |         18507 |
| discover-target-discover-source-postgres-sqlite-park    | Passed  |          2053 |
| discover-target-discover-source-postgres-postgres-start | Passed  |         18560 |
| discover-target-discover-source-postgres-postgres-park  | Passed  |          2153 |
| discover-source-postgres-park                           | Passed  |          2103 |

## Checks

| ID                        | Outcome | Duration (ms) |
| ------------------------- | ------- | ------------: |
| browser-create            | Passed  |          3412 |
| atompub                   | Passed  |         19461 |
| feeds                     | Passed  |         19461 |
| service-restart           | Passed  |           196 |
| browser-read-only         | Passed  |         16049 |
| vm-reboot                 | Passed  |         38421 |
| backup                    | Passed  |           719 |
| restore-sqlite-sqlite     | Passed  |           471 |
| restore-sqlite-postgres   | Passed  |           697 |
| restore-postgres-sqlite   | Passed  |           505 |
| restore-postgres-postgres | Passed  |           671 |
| upgrade                   | Skipped |             0 |

## Gaps

## Findings
