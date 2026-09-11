# Production baseline accept

- Outcome: Passed
- Harness commit: `a545df0ebd9e589f1e62f189abae244f01a276e6`
- Source: `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`
- Manifests: seeded v1 `59dcc0c761795d7f4bba666e938b5f90571075e2f4bc59620fa82002ac42986f`, operation v1 `b7a9508995faacf6979a38a29f83a1578866b5a2ef0adcf2a7cc967be3284f41`
- Target: `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`

## Runtime identities

- deployment `accept-source-sqlite-package-source`; backend Sqlite; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:47:24 UTC] ; stop_time=[n/a] ; pid=776 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-source-sqlite-package-target`; backend Sqlite; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`; executable `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:48:38 UTC] ; stop_time=[n/a] ; pid=701 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`; derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`; output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-target-accept-source-sqlite-sqlite-package-target`; backend Sqlite; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`; executable `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:49:12 UTC] ; stop_time=[n/a] ; pid=767 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`; derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`; output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-target-accept-source-sqlite-postgres-package-target`; backend Postgres; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`; executable `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:49:55 UTC] ; stop_time=[n/a] ; pid=815 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`; derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`; output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-source-postgres-package-source`; backend Postgres; revision `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`; executable `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:50:40 UTC] ; stop_time=[n/a] ; pid=819 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/f6b26c2f9e68c82504b4758778ab2bb706e7cca7#jaunder`; derivation `/nix/store/9813q6x4g2aanpffaw4bcaffvb5crrmp-jaunder-0.1.0.drv`; output `/nix/store/9lqj4lm9q94a0c9zn1xby6n1brwhan79-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-source-postgres-package-target`; backend Postgres; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`; executable `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:51:30 UTC] ; stop_time=[n/a] ; pid=722 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`; derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`; output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-target-accept-source-postgres-sqlite-package-target`; backend Sqlite; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`; executable `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:52:02 UTC] ; stop_time=[n/a] ; pid=708 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`; derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`; output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`
- deployment `accept-target-accept-source-postgres-postgres-package-target`; backend Postgres; revision `c0601a4fbfb262bc92cccfcb85861fbcb00ffe29`; executable `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder`; ExecStart `{ path=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder ; argv[]=/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0/bin/jaunder serve ; ignore_errors=no ; start_time=[Fri 2026-09-11 18:52:33 UTC] ; stop_time=[n/a] ; pid=817 ; code=(null) ; status=0/0 }`; installable `github:jaunder-org/jaunder/c0601a4fbfb262bc92cccfcb85861fbcb00ffe29#jaunder`; derivation `/nix/store/v1inzvlbh02dx0mhjhcgg147ldhsg9fg-jaunder-0.1.0.drv`; output `/nix/store/2ayvqzhsvm7ssqk53w7sapzybs66lqsi-jaunder-0.1.0`; NAR `sha256-7j1khrveJW61w1KsdP4vdbZ6KCMVjKyiX+DPCv8Q1/0=`; SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`

## Backups

- Sqlite → Sqlite; format 1; source schema 36; target schema 36; SHA-256 `577b15fe8f498c296efbf41bc255e12b43ad3717aa1582dc0224a6f9ddae06b8`
- Sqlite → Postgres; format 1; source schema 36; target schema 36; SHA-256 `577b15fe8f498c296efbf41bc255e12b43ad3717aa1582dc0224a6f9ddae06b8`
- Postgres → Sqlite; format 1; source schema 36; target schema 36; SHA-256 `22096c703d6c4b0b04b85d25899b2991b4f05b50fdc9eb32551f302dfcb2bd4b`
- Postgres → Postgres; format 1; source schema 36; target schema 36; SHA-256 `22096c703d6c4b0b04b85d25899b2991b4f05b50fdc9eb32551f302dfcb2bd4b`

## Package activations

- Sqlite; source SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`; target SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`; binary changed false; source schema 36; target schema 36; schema migrated false
- Postgres; source SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`; target SHA-256 `d8f982bf03bfaa5a146d46bfcd5423432829d5ccc9577fbd29944679f1abfd41`; binary changed false; source schema 36; target schema 36; schema migrated false

## Failure classes


## Lifecycle

| ID | Outcome | Duration (ms) |
| --- | --- | ---: |
| workspace-create | Passed | 0 |
| accept-source-sqlite-start | Passed | 259636 |
| accept-source-sqlite-configure-base-url | Passed | 56 |
| accept-source-sqlite-verify-proxy-security | Passed | 206 |
| accept-source-sqlite-source-schema | Passed | 76 |
| accept-source-sqlite-target-schema | Passed | 69 |
| accept-target-accept-source-sqlite-sqlite-start | Passed | 21093 |
| accept-target-accept-source-sqlite-sqlite-park | Passed | 1953 |
| accept-target-accept-source-sqlite-postgres-start | Passed | 29957 |
| accept-target-accept-source-sqlite-postgres-park | Passed | 2053 |
| accept-source-sqlite-park | Passed | 1903 |
| accept-source-postgres-start | Passed | 28624 |
| accept-source-postgres-configure-base-url | Passed | 52 |
| accept-source-postgres-verify-proxy-security | Passed | 189 |
| accept-source-postgres-source-schema | Passed | 126 |
| accept-source-postgres-target-schema | Passed | 119 |
| accept-target-accept-source-postgres-sqlite-start | Passed | 18459 |
| accept-target-accept-source-postgres-sqlite-park | Passed | 1953 |
| accept-target-accept-source-postgres-postgres-start | Passed | 18480 |
| accept-target-accept-source-postgres-postgres-park | Passed | 2053 |
| accept-source-postgres-park | Passed | 2003 |

## Checks

| ID | Outcome | Duration (ms) |
| --- | --- | ---: |
| browser-create | Passed | 3364 |
| atompub | Passed | 26895 |
| feeds | Passed | 26895 |
| service-restart | Passed | 220 |
| browser-read-only | Passed | 23531 |
| vm-reboot | Passed | 35525 |
| upgrade | Passed | 58878 |
| backup | Passed | 175 |
| restore-sqlite-sqlite | Passed | 483 |
| restore-sqlite-postgres | Passed | 623 |
| restore-postgres-sqlite | Passed | 445 |
| restore-postgres-postgres | Passed | 608 |

## Gaps


## Findings
