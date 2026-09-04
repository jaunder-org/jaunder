# Issue 1289: pre- and post-change Nix invalidation-boundary measurements

## Scope and source material

This is the durable, normalized pre- and post-change record for issue #1289. The
ignored JSON sidecars in `.xtask/measurements/` are the source material: the
pre-change `warm-baseline`, `docs-only`, `web-only`, `high-stack-rust`,
`low-stack-rust`, and `low-stack-macros` arms, followed by identically named
`post-` arms. This report preserves their result rows and failure evidence.

## Reproduction metadata

- Date: 2026-09-04.
- Pre-change baseline revision: `168edad2d2ac44bd9662d02787c82e4718101afc` (the
  `origin/main` revision on which this branch was created).
- Post-change baseline revision: `6cdb765742474ad3a3212eb39a59935ab13a13f9`
  (committed Task 4 HEAD).
- System: `x86_64-linux`.
- Pre-change command: `cargo xtask --json validate --no-e2e --allow-dirty`.
- Post-change command:
  `devtool run -- cargo xtask --json validate --no-e2e --allow-dirty`.
- One unrecorded warm-up run preceded each saved warm baseline. The Nix store
  was not purged. Each perturbation was applied and measured one at a time, then
  removed to restore the source bytes before the next arm.
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

The post-change sequence used the command and marker procedure above after an
unrecorded warm-up. Every arm completed `ok`; the historical pre-change `common`
coverage-consumer failure did not recur and is retained above as data. All rows
below are copied from the six parseable `post-*.json` sidecars.

### Warm baseline

- Marker: None (unmodified committed Task 4 baseline)
- Overall outcome: **ok**
- Overall duration: **160112 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | -------- |
| `nix-static-docs`             | `.#checks.x86_64-linux.static-docs`             | `/nix/store/23pdijyd9hypijriqn9n25i2762mksx1-static-docs.drv`                                 | reused      | 864 ms   |
| `nix-static-code`             | `.#checks.x86_64-linux.static-code`             | `/nix/store/ffyvm2lj7ga73zql98d6yqd70hhw6f71-static-code.drv`                                 | reused      | 1518 ms  |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/xc8g35b5d4f54zw8q40gj6i8w7dy1wfr-jaunder-site.drv`                                | reused      | 39131 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/albnqi2635g1k4kg2hspwysy7nbwmfwh-jaunder-wasm-tests-test-0.1.0.drv`               | reused      | 845 ms   |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/rl39gi0bmxy30b7l2a70mvrkxl29rkp9-jaunder-coverage-0.1.0.drv`                      | reused      | 648 ms   |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/f92hr68dn506ygrqxdy48cg698iqilja-jaunder-coverage-gate.drv`                       | reused      | 560 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/w797wqxvfdgibrrzmnizzpn5im8v1k05-jaunder-doctests-0.1.0.drv`                      | reused      | 502 ms   |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/4a97h1zspwpgi5ax63a7wxds3g1pf4l2-jaunder-doctests-gate.drv`                       | reused      | 498 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/ybhdxn1j70kp2lh0d8zfjsww9c19cpgg-vm-test-run-jaunder-elisp-coverage-producer.drv` | reused      | 947 ms   |

### Docs-only marker

- Marker: `docs/DESIGN.md` — `<!-- Nix reuse measurement: docs-only. -->`
- Overall outcome: **ok**
- Overall duration: **235607 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | -------- |
| `nix-static-docs`             | `.#checks.x86_64-linux.static-docs`             | `/nix/store/v25qcsgcxc85miyhs5q5ffkr3b6vwm5z-static-docs.drv`                                 | realized    | 19994 ms |
| `nix-static-code`             | `.#checks.x86_64-linux.static-code`             | `/nix/store/ffyvm2lj7ga73zql98d6yqd70hhw6f71-static-code.drv`                                 | reused      | 13186 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/xc8g35b5d4f54zw8q40gj6i8w7dy1wfr-jaunder-site.drv`                                | reused      | 57660 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/albnqi2635g1k4kg2hspwysy7nbwmfwh-jaunder-wasm-tests-test-0.1.0.drv`               | reused      | 5331 ms  |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/rl39gi0bmxy30b7l2a70mvrkxl29rkp9-jaunder-coverage-0.1.0.drv`                      | reused      | 3786 ms  |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/f92hr68dn506ygrqxdy48cg698iqilja-jaunder-coverage-gate.drv`                       | reused      | 9070 ms  |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/w797wqxvfdgibrrzmnizzpn5im8v1k05-jaunder-doctests-0.1.0.drv`                      | reused      | 3241 ms  |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/4a97h1zspwpgi5ax63a7wxds3g1pf4l2-jaunder-doctests-gate.drv`                       | reused      | 2809 ms  |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/ybhdxn1j70kp2lh0d8zfjsww9c19cpgg-vm-test-run-jaunder-elisp-coverage-producer.drv` | reused      | 8106 ms  |

### Web-only marker

- Marker: `web/src/app/render.rs` — `// Nix reuse measurement: web-only.`
- Overall outcome: **ok**
- Overall duration: **1160689 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration  |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | --------- |
| `nix-static-docs`             | `.#checks.x86_64-linux.static-docs`             | `/nix/store/23pdijyd9hypijriqn9n25i2762mksx1-static-docs.drv`                                 | reused      | 2362 ms   |
| `nix-static-code`             | `.#checks.x86_64-linux.static-code`             | `/nix/store/pv2m2vqc5zkql3czrlvv3pqhhw1y3fy3-static-code.drv`                                 | realized    | 279192 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/lp9fmybj6ziyjgwfvp3apj6a13356aan-jaunder-site.drv`                                | realized    | 117031 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/albnqi2635g1k4kg2hspwysy7nbwmfwh-jaunder-wasm-tests-test-0.1.0.drv`               | reused      | 5389 ms   |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/7bf6fzmnqr9fbi5mp4425ylgd0mici2m-jaunder-coverage-0.1.0.drv`                      | realized    | 248933 ms |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/kcjbgp4nsggjqhdmm6g2iapsc2cm3s2s-jaunder-coverage-gate.drv`                       | realized    | 5688 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/a5jf1mpr4nn0in82078kbz87icfqdnv8-jaunder-doctests-0.1.0.drv`                      | realized    | 117034 ms |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/w7ihky7lslf9kqlfzbya876zk0gx4n3g-jaunder-doctests-gate.drv`                       | realized    | 4852 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/sv0j9al5zdrjxd945zcmyw1lpmqffvin-vm-test-run-jaunder-elisp-coverage-producer.drv` | realized    | 271031 ms |

### High-stack Rust marker

- Marker: `server/src/lib.rs` — `// Nix reuse measurement: high-stack-rust.`
- Overall outcome: **ok**
- Overall duration: **1122763 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration  |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | --------- |
| `nix-static-docs`             | `.#checks.x86_64-linux.static-docs`             | `/nix/store/23pdijyd9hypijriqn9n25i2762mksx1-static-docs.drv`                                 | reused      | 2572 ms   |
| `nix-static-code`             | `.#checks.x86_64-linux.static-code`             | `/nix/store/fvmlz7myknyl1s6pnhgkj7lddhsnj0xg-static-code.drv`                                 | realized    | 331471 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/xc8g35b5d4f54zw8q40gj6i8w7dy1wfr-jaunder-site.drv`                                | reused      | 49518 ms  |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/albnqi2635g1k4kg2hspwysy7nbwmfwh-jaunder-wasm-tests-test-0.1.0.drv`               | reused      | 6350 ms   |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/8mb8hhpjci47kjyqngqhw4f1l08xy5r5-jaunder-coverage-0.1.0.drv`                      | realized    | 253549 ms |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/2pbcs7l2vkmi65c9v0ishjm2xqlbdsg2-jaunder-coverage-gate.drv`                       | realized    | 5633 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/ic199f3jxchr23cxn1kipcfii9l37vrg-jaunder-doctests-0.1.0.drv`                      | realized    | 136836 ms |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/yp5hb67apwhb9wf1fqfq9nkc60ppv7p7-jaunder-doctests-gate.drv`                       | realized    | 5204 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/pj97ypr5br6bjd512y18n82rwl8a6ld9-vm-test-run-jaunder-elisp-coverage-producer.drv` | realized    | 203527 ms |

### Low-stack Rust marker

- Marker: `common/src/text.rs` — `// Nix reuse measurement: low-stack-rust.`
- Overall outcome: **ok**
- Overall duration: **1063105 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration  |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | --------- |
| `nix-static-docs`             | `.#checks.x86_64-linux.static-docs`             | `/nix/store/23pdijyd9hypijriqn9n25i2762mksx1-static-docs.drv`                                 | reused      | 2211 ms   |
| `nix-static-code`             | `.#checks.x86_64-linux.static-code`             | `/nix/store/rivhip4byyklpmskcy8shwjlksnj5kmv-static-code.drv`                                 | realized    | 241622 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/bj6im344khyvyrk4z4xwdg1kb7p691w4-jaunder-site.drv`                                | realized    | 105731 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/h4ix80s6l41v1k3jwl4ksjwddcalhnbr-jaunder-wasm-tests-test-0.1.0.drv`               | realized    | 21681 ms  |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/z0zdqwpyc5jm0yq1y0qi4p8zpj9f4acy-jaunder-coverage-0.1.0.drv`                      | realized    | 221778 ms |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/5qz0m55slb9wk9ky1x253nih0hr5m210-jaunder-coverage-gate.drv`                       | realized    | 5450 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/qvn3adc10smh9fv46ivmdm8d5r2vak6z-jaunder-doctests-0.1.0.drv`                      | realized    | 108277 ms |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/bcnm8j8h264pkf9qsswzda5yrmpw15kb-jaunder-doctests-gate.drv`                       | realized    | 5854 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/365ai0nd8wnks0f78mq80cvkrvp6pdp5-vm-test-run-jaunder-elisp-coverage-producer.drv` | realized    | 238807 ms |

### Low-stack macros marker

- Marker: `macros/src/lib.rs` — `// Nix reuse measurement: low-stack-macros.`
- Overall outcome: **ok**
- Overall duration: **1087196 ms**

| Step                          | Installable                                     | Full derivation path                                                                          | Realization | Duration  |
| ----------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------- | ----------- | --------- |
| `nix-static-docs`             | `.#checks.x86_64-linux.static-docs`             | `/nix/store/23pdijyd9hypijriqn9n25i2762mksx1-static-docs.drv`                                 | reused      | 2645 ms   |
| `nix-static-code`             | `.#checks.x86_64-linux.static-code`             | `/nix/store/7xm1h8flhf4nqpwpx24zw9bg49kpmddy-static-code.drv`                                 | realized    | 253260 ms |
| `wasm-budget`                 | `.#site`                                        | `/nix/store/b1zj8w28v946asgvan9rpvgq5vir98wc-jaunder-site.drv`                                | realized    | 104718 ms |
| `wasm-tests`                  | `.#checks.x86_64-linux.wasm-tests`              | `/nix/store/v0c0ncpfyljf85wcbqkbn1waknacs8s3-jaunder-wasm-tests-test-0.1.0.drv`               | realized    | 22495 ms  |
| `nix-coverage`                | `.#checks.x86_64-linux.coverage`                | `/nix/store/3ypl8vs56whiysaqvyqnr2yprxgdxyap-jaunder-coverage-0.1.0.drv`                      | realized    | 218941 ms |
| `nix-coverage-gate`           | `.#checks.x86_64-linux.coverage-gate`           | `/nix/store/27ija1h87lpnxkndkgqxca28lcyrzcbb-jaunder-coverage-gate.drv`                       | realized    | 4624 ms   |
| `nix-doctests`                | `.#checks.x86_64-linux.doctests`                | `/nix/store/1wvybzl0jjdy743z70nxjnf34bhyy3mg-jaunder-doctests-0.1.0.drv`                      | realized    | 103244 ms |
| `nix-doctests-gate`           | `.#checks.x86_64-linux.doctests-gate`           | `/nix/store/6j8gbxydivj2ddyd6nb5rcr0w4s5gs66-jaunder-doctests-gate.drv`                       | realized    | 5295 ms   |
| `nix-elisp-coverage-producer` | `.#checks.x86_64-linux.elisp-coverage-producer` | `/nix/store/3cfhbcnidbw3lm83ksyp06xilrlg5k42-vm-test-run-jaunder-elisp-coverage-producer.drv` | realized    | 258187 ms |

## Before/after conclusion

The saved warmed baseline reused every recorded Nix output. Relative to that
baseline's identities, docs-only changed and realized only `static-docs`;
`static-code`, `.#site`, `wasm-tests`, and all unrelated Nix checks retained
their identities and reused outputs. Web-only changed and realized `static-code`
and `.#site`, while `static-docs` and `wasm-tests` retained their identities and
reused outputs. High-stack Rust changed and realized `static-code` plus the
broad product coverage, doctest, and Elisp checks, while `static-docs`,
`.#site`, and `wasm-tests` retained their identities and reused outputs.
`common` and `macros` each changed and realized every checked boundary that
depends on their source — `static-code`, site, wasm tests, coverage, doctests,
and Elisp — while the independent `static-docs` boundary reused. Thus the
post-change graph matches the required docs/web/server/low-stack matrix and
removes the pre-change server-to-site/wasm and web-to-wasm invalidation.

The rejected candidates remain unjustified by the measurements: per-ecosystem
static fan-out would add at least three Nix evaluations, source-tree staging
copies, and process boundaries to every validation without a representative
Elisp-, tools-, or end2end-only observed saving. Splitting coverage would repeat
some or all of its measured 216.2–354.9-second instrumented compile/test pass
and require a merged verdict; splitting doctests would duplicate its measured
93.5–176.4-second compilation or lose workspace feature unification and fence
reconciliation. Splitting the Elisp producer would add a second NixOS VM boot to
the measured 201.5–306.9-second combined producer and a second artifact handoff.
E2E is already split into four backend/browser derivations plus its aggregate,
so it is not a candidate.
