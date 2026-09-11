# Production baseline discover

- Outcome: Passed
- Harness commit: `929ec3e26e30d6ce489bf38efaa5bc557b9043c7`
- Source: `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`
- Manifests: seeded v1
  `59dcc0c761795d7f4bba666e938b5f90571075e2f4bc59620fa82002ac42986f`, operation
  v1 `b7a9508995faacf6979a38a29f83a1578866b5a2ef0adcf2a7cc967be3284f41`

## Runtime identities

- deployment `discover-source-sqlite`; backend Sqlite; revision
  `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable
  `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:23:38 UTC] ; stop_time=[n/a] ; pid=768 ; code=(null) ; status=0/0 }`;
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
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:24:33 UTC] ; stop_time=[n/a] ; pid=709 ; code=(null) ; status=0/0 }`;
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
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:25:14 UTC] ; stop_time=[n/a] ; pid=817 ; code=(null) ; status=0/0 }`;
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
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:25:49 UTC] ; stop_time=[n/a] ; pid=831 ; code=(null) ; status=0/0 }`;
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
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:26:46 UTC] ; stop_time=[n/a] ; pid=712 ; code=(null) ; status=0/0 }`;
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
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:27:16 UTC] ; stop_time=[n/a] ; pid=818 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`;
  derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`;
  output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`

## Backups

- Sqlite → Sqlite; format 1; source schema 36; target schema 36; SHA-256
  `59ce7faddfbeca94f5b94546e8d13e91ab76a5ee18d709caa9427863b1cad430`
- Sqlite → Postgres; format 1; source schema 36; target schema 36; SHA-256
  `59ce7faddfbeca94f5b94546e8d13e91ab76a5ee18d709caa9427863b1cad430`
- Postgres → Sqlite; format 1; source schema 36; target schema 36; SHA-256
  `42611bdf41185dd80fb70142d05337389ed86cbb0749bdc7e8015a1b297d607b`
- Postgres → Postgres; format 1; source schema 36; target schema 36; SHA-256
  `42611bdf41185dd80fb70142d05337389ed86cbb0749bdc7e8015a1b297d607b`

## Package activations

## Failure classes

## Lifecycle

| ID                                                      | Outcome | Duration (ms) |
| ------------------------------------------------------- | ------- | ------------: |
| workspace-create                                        | Passed  |             0 |
| discover-source-sqlite-start                            | Passed  |         41272 |
| discover-source-sqlite-configure-base-url               | Passed  |            64 |
| discover-source-sqlite-verify-proxy-security            | Passed  |           203 |
| discover-target-discover-source-sqlite-sqlite-start     | Passed  |         18501 |
| discover-target-discover-source-sqlite-sqlite-park      | Passed  |          2053 |
| discover-target-discover-source-sqlite-postgres-start   | Passed  |         25608 |
| discover-target-discover-source-sqlite-postgres-park    | Passed  |          2153 |
| discover-source-sqlite-park                             | Passed  |          2153 |
| discover-source-postgres-start                          | Passed  |         20132 |
| discover-source-postgres-configure-base-url             | Passed  |           254 |
| discover-source-postgres-verify-proxy-security          | Passed  |           752 |
| discover-target-discover-source-postgres-sqlite-start   | Passed  |         18493 |
| discover-target-discover-source-postgres-sqlite-park    | Passed  |          2053 |
| discover-target-discover-source-postgres-postgres-start | Passed  |         18478 |
| discover-target-discover-source-postgres-postgres-park  | Passed  |          2103 |
| discover-source-postgres-park                           | Passed  |          2053 |

## Checks

| ID                        | Outcome | Duration (ms) |
| ------------------------- | ------- | ------------: |
| browser-create            | Passed  |          5142 |
| atompub                   | Passed  |         21620 |
| feeds                     | Passed  |         21620 |
| service-restart           | Passed  |           352 |
| browser-read-only         | Passed  |         16478 |
| vm-reboot                 | Passed  |         36026 |
| backup                    | Passed  |           203 |
| restore-sqlite-sqlite     | Passed  |           462 |
| restore-sqlite-postgres   | Passed  |           664 |
| restore-postgres-sqlite   | Passed  |           470 |
| restore-postgres-postgres | Passed  |           627 |
| upgrade                   | Skipped |             0 |

## Gaps

## Findings
