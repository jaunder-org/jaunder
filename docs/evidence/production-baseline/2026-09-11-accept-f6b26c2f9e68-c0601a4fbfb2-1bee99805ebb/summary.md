# Production baseline accept

- Outcome: Passed
- Harness commit: `1bee99805ebbebe9e514600a315751a53d659588`
- Source: `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`
- Manifests: seeded v1
  `59dcc0c761795d7f4bba666e938b5f90571075e2f4bc59620fa82002ac42986f`, operation
  v1 `24c6bb6542a3cc4efd27b7ca224a7b6d7fbb16d22653236f0399458cc3255434`
- Target: `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`

## Runtime identities

- deployment `accept-source-sqlite-package-source`; backend Sqlite; revision
  `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable
  `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 15:54:16 UTC] ; stop_time=[n/a] ; pid=713 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`;
  derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`;
  output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-source-sqlite-package-target`; backend Sqlite; revision
  `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`; executable
  `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 16:02:48 UTC] ; stop_time=[n/a] ; pid=703 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`;
  derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`;
  output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-target-accept-source-sqlite-sqlite-package-target`; backend
  Sqlite; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`; executable
  `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 16:03:27 UTC] ; stop_time=[n/a] ; pid=770 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`;
  derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`;
  output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-target-accept-source-sqlite-postgres-package-target`;
  backend Postgres; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`;
  executable
  `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 16:04:19 UTC] ; stop_time=[n/a] ; pid=833 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`;
  derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`;
  output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-source-postgres-package-source`; backend Postgres; revision
  `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable
  `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 16:05:05 UTC] ; stop_time=[n/a] ; pid=821 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`;
  derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`;
  output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-source-postgres-package-target`; backend Postgres; revision
  `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`; executable
  `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 16:06:00 UTC] ; stop_time=[n/a] ; pid=718 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`;
  derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`;
  output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-target-accept-source-postgres-sqlite-package-target`;
  backend Sqlite; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`;
  executable
  `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 16:06:39 UTC] ; stop_time=[n/a] ; pid=728 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`;
  derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`;
  output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-target-accept-source-postgres-postgres-package-target`;
  backend Postgres; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`;
  executable
  `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`;
  ExecStart
  `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 16:07:14 UTC] ; stop_time=[n/a] ; pid=816 ; code=(null) ; status=0/0 }`;
  installable
  `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`;
  derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`;
  output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR
  `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`

## Backups

- Sqlite → Sqlite; format 1; source schema 36; target schema 36; SHA-256
  `604b01249d2439d8e1452ebe0626b3a598e2bace771c1a6948f555f4dff8386b`
- Sqlite → Postgres; format 1; source schema 36; target schema 36; SHA-256
  `604b01249d2439d8e1452ebe0626b3a598e2bace771c1a6948f555f4dff8386b`
- Postgres → Sqlite; format 1; source schema 36; target schema 36; SHA-256
  `840f37309b9c392395b4d879fba53e7f2e32db231edfeaed89d94d8327953dce`
- Postgres → Postgres; format 1; source schema 36; target schema 36; SHA-256
  `840f37309b9c392395b4d879fba53e7f2e32db231edfeaed89d94d8327953dce`

## Package activations

- Sqlite; source SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`; target
  SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`;
  binary changed false; source schema 36; target schema 36; schema migrated
  false
- Postgres; source SHA-256
  `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`; target
  SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`;
  binary changed false; source schema 36; target schema 36; schema migrated
  false

## Failure classes

## Lifecycle

| ID                                                  | Outcome | Duration (ms) |
| --------------------------------------------------- | ------- | ------------: |
| workspace-create                                    | Passed  |             0 |
| accept-source-sqlite-start                          | Passed  |        518907 |
| accept-source-sqlite-configure-base-url             | Passed  |            51 |
| accept-source-sqlite-source-schema                  | Passed  |           125 |
| accept-source-sqlite-target-schema                  | Passed  |            72 |
| accept-target-accept-source-sqlite-sqlite-start     | Passed  |         18487 |
| accept-target-accept-source-sqlite-sqlite-park      | Passed  |          2203 |
| accept-target-accept-source-sqlite-postgres-start   | Passed  |         37119 |
| accept-target-accept-source-sqlite-postgres-park    | Passed  |          2353 |
| accept-source-sqlite-park                           | Passed  |          2103 |
| accept-source-postgres-start                        | Passed  |         33107 |
| accept-source-postgres-configure-base-url           | Passed  |            79 |
| accept-source-postgres-source-schema                | Passed  |           244 |
| accept-source-postgres-target-schema                | Passed  |           117 |
| accept-target-accept-source-postgres-sqlite-start   | Passed  |         18817 |
| accept-target-accept-source-postgres-sqlite-park    | Passed  |          2605 |
| accept-target-accept-source-postgres-postgres-start | Passed  |         21103 |
| accept-target-accept-source-postgres-postgres-park  | Passed  |          2153 |
| accept-source-postgres-park                         | Passed  |          2053 |

## Checks

| ID                        | Outcome | Duration (ms) |
| ------------------------- | ------- | ------------: |
| browser-create            | Passed  |          3456 |
| atompub                   | Passed  |         27784 |
| feeds                     | Passed  |         27784 |
| service-restart           | Passed  |           215 |
| browser-read-only         | Passed  |         24328 |
| vm-reboot                 | Passed  |         36428 |
| upgrade                   | Passed  |        493745 |
| backup                    | Passed  |           227 |
| restore-sqlite-sqlite     | Passed  |           484 |
| restore-sqlite-postgres   | Passed  |           656 |
| restore-postgres-sqlite   | Passed  |           608 |
| restore-postgres-postgres | Passed  |           654 |

## Gaps

## Findings
