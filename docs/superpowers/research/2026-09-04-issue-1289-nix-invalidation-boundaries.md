# Issue 1289: pre-change Nix invalidation-boundary measurements

## Scope and source material

This is the durable, normalized pre-change record for issue #1289. It is derived
from the six saved measurement sidecars in `.xtask/measurements/`:
`warm-baseline`, `docs-only`, `web-only`, `high-stack-rust`, `low-stack-rust`,
and `low-stack-macros`. Those ignored JSON files remain the source material;
this report preserves their result rows and failure evidence.

## Reproduction metadata

- Date: 2026-09-04.
- Baseline revision: `168edad2d2ac44bd9662d02787c82e4718101afc` (the
  `origin/main` revision on which this branch was created).
- System: `x86_64-linux`.
- Command: `cargo xtask --json validate --no-e2e --allow-dirty`.
- One warm-up run preceded the saved warm baseline.
- The Nix store was not purged.
- Each perturbation was applied and measured one at a time, then restored before
  the next arm.
- Marker procedure (exact contents):
  - `docs/DESIGN.md`: `<!-- Nix reuse measurement: docs-only. -->`
  - `web/src/app/render.rs`: `// Nix reuse measurement: web-only.`
  - `server/src/lib.rs`: `// Nix reuse measurement: high-stack-rust.`
  - `common/src/text.rs`: `// Nix reuse measurement: low-stack-rust.`
  - `macros/src/lib.rs`: `// Nix reuse measurement: low-stack-macros.`

Each table includes every sidecar step carrying `nix`; derivation paths are
recorded in full.

## Warm baseline

- Marker: None (unmodified baseline)
- Overall outcome: **ok**
- Overall duration: **136013 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | -------- |
| `nix-static-checks`           | `.#checks.x86_64-linux.static-checks`           | `/nix/store/sqy3qzw009wa263a36zjr11r0ddx0lv8-static-checks.drv`                               | reused      | 812 ms   |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/f4lzyq4g396wb81dk6hidzvr3jsjn4sl-jaunder-site.drv`                                | reused      | 41696 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/zqd822vlj1pa1xfsq0s65cc8lxiif0id-jaunder-wasm-tests-test-0.1.0.drv`               | reused      | 666 ms   |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/qhgqir4j4nl5yk2lr34xz8k3fkjjcxhs-jaunder-coverage-0.1.0.drv`                      | reused      | 499 ms   |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/biv4zc7bcqr5jyrsb2canpsw4g5wwma9-jaunder-coverage-gate.drv`                       | reused      | 502 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/f17vh78p6xbs66mdr31lm79vy4yin7xj-jaunder-doctests-0.1.0.drv`                      | reused      | 467 ms   |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/acc54vc1qvrmkx25p2yskldq3xhrm8xj-jaunder-doctests-gate.drv`                       | reused      | 471 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/imrm6l6jh2i0lz4l51r9fn7sdi9c6br6-vm-test-run-jaunder-elisp-coverage-producer.drv` | reused      | 704 ms   |

## Docs-only marker

- Marker: docs/DESIGN.md — `<!-- Nix reuse measurement: docs-only. -->`
- Overall outcome: **ok**
- Overall duration: **451615 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration  |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | --------- |
| `nix-static-checks`           | `.#checks.x86_64-linux.static-checks`           | `/nix/store/4ar1j7lj6g8wc271c4fylq89fdchgx3p-static-checks.drv`                               | realized    | 292424 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/f4lzyq4g396wb81dk6hidzvr3jsjn4sl-jaunder-site.drv`                                | reused      | 48557 ms  |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/zqd822vlj1pa1xfsq0s65cc8lxiif0id-jaunder-wasm-tests-test-0.1.0.drv`               | reused      | 5872 ms   |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/qhgqir4j4nl5yk2lr34xz8k3fkjjcxhs-jaunder-coverage-0.1.0.drv`                      | reused      | 3363 ms   |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/biv4zc7bcqr5jyrsb2canpsw4g5wwma9-jaunder-coverage-gate.drv`                       | reused      | 3620 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/f17vh78p6xbs66mdr31lm79vy4yin7xj-jaunder-doctests-0.1.0.drv`                      | reused      | 2704 ms   |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/acc54vc1qvrmkx25p2yskldq3xhrm8xj-jaunder-doctests-gate.drv`                       | reused      | 2967 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/imrm6l6jh2i0lz4l51r9fn7sdi9c6br6-vm-test-run-jaunder-elisp-coverage-producer.drv` | reused      | 8537 ms   |

## Web-only marker

- Marker: web/src/app/render.rs — `// Nix reuse measurement: web-only.`
- Overall outcome: **ok**
- Overall duration: **1202618 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration  |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | --------- |
| `nix-static-checks`           | `.#checks.x86_64-linux.static-checks`           | `/nix/store/azsifqgawqiqhraax6im2yjn0mpam714-static-checks.drv`                               | realized    | 325700 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/2k5429x4rx5x4mkmh05mqhr10rdqqjpf-jaunder-site.drv`                                | realized    | 135660 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/22ildhr894mj3gbzr0z7s2zrn1iypf34-jaunder-wasm-tests-test-0.1.0.drv`               | realized    | 26011 ms  |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/155074is4kg3m8kr19b2lgk1ma64nzh9-jaunder-coverage-0.1.0.drv`                      | realized    | 254519 ms |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/mbkkr5cq1zgjpwm1x08p631kpzzc8m72-jaunder-coverage-gate.drv`                       | realized    | 7242 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/5av9ibay9jwbvmki22zvfzk83xq5fbxa-jaunder-doctests-0.1.0.drv`                      | realized    | 93452 ms  |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/yidsnqm1zam26m0a1vii5r04axldv1zi-jaunder-doctests-gate.drv`                       | realized    | 3925 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/faivqjb4nrp1l3pv61vsyfbclzik0h7r-vm-test-run-jaunder-elisp-coverage-producer.drv` | realized    | 201525 ms |

## High-stack Rust (server-only) marker

- Marker: server/src/lib.rs — `// Nix reuse measurement: high-stack-rust.`
- Overall outcome: **ok**
- Overall duration: **1127649 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration  |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | --------- |
| `nix-static-checks`           | `.#checks.x86_64-linux.static-checks`           | `/nix/store/mpilvxxjpam7cb7vjqkapdhmf5sjkhm1-static-checks.drv`                               | realized    | 280593 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/67xwds7wvmm2mm6xmmq7i5byvpbvbj8y-jaunder-site.drv`                                | realized    | 115597 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/dj564948mz7w41ka3vy55ljgg5b5gy6r-jaunder-wasm-tests-test-0.1.0.drv`               | realized    | 22773 ms  |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/lcf8wz1kkajzrxldz1ymxjgfa9lpmikz-jaunder-coverage-0.1.0.drv`                      | realized    | 216210 ms |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/cv51jsr64yx70jr2r3pza715v4s3420j-jaunder-coverage-gate.drv`                       | realized    | 5752 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/spkqidlc2agn4p5hd35qrzwmslj6sm8m-jaunder-doctests-0.1.0.drv`                      | realized    | 114297 ms |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/5nlvwpxd13a69wa7sazzbhsbc3s252gg-jaunder-doctests-gate.drv`                       | realized    | 3903 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/arqgn15gsf4pxwznr0kq0kkr23fi6ahg-vm-test-run-jaunder-elisp-coverage-producer.drv` | realized    | 263267 ms |

## Low-stack Rust (common) marker

- Marker: common/src/text.rs — `// Nix reuse measurement: low-stack-rust.`
- Overall outcome: **failed**
- Overall duration: **1696432 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration  |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | --------- |
| `nix-static-checks`           | `.#checks.x86_64-linux.static-checks`           | `/nix/store/xfyfzpjzahr2jwjgsgraajmb41j7g9kw-static-checks.drv`                               | realized    | 396116 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/5hf713x8i8g6622s6i42rz5cyxa56850-jaunder-site.drv`                                | realized    | 183155 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/q8xghdqm9ppdsd3l48garwh7xrhm36ip-jaunder-wasm-tests-test-0.1.0.drv`               | realized    | 50098 ms  |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/4p5q6qi94v7ypqcxsv18idz3rdswxn0s-jaunder-coverage-0.1.0.drv`                      | realized    | 354919 ms |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/21bwm0dv611kl6rln205sw9ja9ps64ln-jaunder-coverage-gate.drv`                       | realized    | 7425 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/xhxrq6cl7a5gf1ppf3h9l1dgbfgb7l9j-jaunder-doctests-0.1.0.drv`                      | realized    | 176441 ms |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/2pv8fprf94w35gaq4ivx3mgxlwarvgvd-jaunder-doctests-gate.drv`                       | realized    | 7872 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/s1rzsrnsbbm0yvlrg7y3q07m0gjzg6dn-vm-test-run-jaunder-elisp-coverage-producer.drv` | realized    | 306896 ms |

### Failure

```text
coverage (0 ms)
0 uncovered line(s), 0 guard violation(s), 1 CRAP over threshold
  CRAP over threshold:
    server/src/commands/lifecycle.rs::cmd_serve crap=53.66
  → reduce the function's complexity or improve its coverage; if this is approved
    drift, add `// crap:allow: <reason>` within the function's span.
```

## Low-stack macros marker

- Marker: macros/src/lib.rs — `// Nix reuse measurement: low-stack-macros.`
- Overall outcome: **ok**
- Overall duration: **1562717 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration  |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | --------- |
| `nix-static-checks`           | `.#checks.x86_64-linux.static-checks`           | `/nix/store/d9inwkng3j95w3901bwri2m7hdwmh79l-static-checks.drv`                               | realized    | 371250 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/az6nl2x6dw1iq0vb38garg34mqrfm0ky-jaunder-site.drv`                                | realized    | 146999 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/36kzbfpyackgqc8lh8b3jzqdzssnxb1g-jaunder-wasm-tests-test-0.1.0.drv`               | realized    | 30191 ms  |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/izidj4pig40xa412yj5n6wh10ihwaxra-jaunder-coverage-0.1.0.drv`                      | realized    | 299837 ms |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/gwgq89pd4m5l5dzhz6gqsbkmzqpnrdn4-jaunder-coverage-gate.drv`                       | realized    | 10654 ms  |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/g6p7fafhwp6c6znbpw5krvzw9sx11sd0-jaunder-doctests-0.1.0.drv`                      | realized    | 161362 ms |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/fa13wkpdqlcbqr10dxhx4qgbg9n8qq6m-jaunder-doctests-gate.drv`                       | realized    | 10969 ms  |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/5gzzb92wzqgnfgq20q3mn6q823zw7fw3-vm-test-run-jaunder-elisp-coverage-producer.drv` | realized    | 270757 ms |

## Pre-change conclusion

The warmed pre-change graph reused every recorded Nix step in the baseline. A
docs-only marker realized only `nix-static-checks` (292424 ms); its other Nix
rows were reused. The web-only marker realized client `wasm-tests` (26011 ms),
and the high-stack Rust marker realized `wasm-budget`/`.#site` (115597 ms) and
client `wasm-tests` (22773 ms). Both low-stack perturbations changed every
recorded Nix derivation identity and realized every recorded Nix step. The
`low-stack-rust` arm failed only at the host `coverage` consumer recorded above;
the `low-stack-macros` arm completed successfully and is the successful
low-stack representative.

## Post-change evidence

**Pending implementation.** Post-change measurements have not yet been recorded.
