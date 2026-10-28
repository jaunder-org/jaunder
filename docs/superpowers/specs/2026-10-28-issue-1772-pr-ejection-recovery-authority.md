# Issue #1772: Route ejection recovery through approved owner authority

## Outcome

After a PR is ejected, Jaunder's `land` help and owner guidance will require
diagnosis and separately authorized recovery, without saying that a human must
make every technical judgment. The existing one-shot `land <PR> --reenqueue`
operation remains explicit and guarded. Watch remains observation-only, and
merge still requires the existing per-PR human authorization.

## Approach and tradeoffs

Change the wording in `xtask/src/cli.rs` (the `PrCommand` help comment around
lines 496–500), the owning PR guidance in `CONTRIBUTING.md` (the PR
watching/recovery section around lines 1177–1216), and the conflicting
technical-judgment sentence in accepted ADR-0087 (section 4). ADR-0087 should
continue to establish observer/armer separation and `land` as the merge
approval, while assigning diagnosis/recovery authorization to current owner
policy rather than imposing a universal human technical-judgment gate. These
maintained Jaunder sources are the deliverable; generated CLI help follows the
source comment.

Preserve `land <PR> --reenqueue` as an explicit, opt-in, one-shot armer action.
It remains `land`-only, defaults off, and is rejected for `watch`. Existing
queue/ejection/readiness and uncertainty guards remain in force; this wording
change adds no recovery behavior. Ordinary `land` does not requeue. `watch`
never mutates. Invoking `land` remains the distinct merge/arming action and does
not waive the qualified human approval required for that PR.

Routine technical diagnosis is owned by the assigned Lead; recovery requires
explicit authorization from the authority assigned by the governing owner
policy. For Manager-assigned work, the Manager filters/authorizes where that
policy reserves the decision; otherwise the assigned Lead may authorize recovery
within approved scope. Do not require an extra human technical judgment where
policy assigns the decision to the Lead, and do not infer that diagnosis itself
authorizes recovery. Preserve genuine human gates, including the existing per-PR
merge authorization.

External skills are comparison/policy inputs, not files to edit in this Jaunder
checkout. The maintained skill sources are under
`/home/mdorman/src/nixos/modules/users/mdorman/ai/skills/`, mapped into
installed `~/.agents/skills` by `modules/users/mdorman/ai.nix`; source ownership
and any corresponding external change belong to the NixOS coordinator/Manager.
This issue does not depend on changing those skills.

## Acceptance

- `xtask/src/cli.rs` help, `CONTRIBUTING.md`, and ADR-0087 consistently describe
  Lead-owned routine diagnosis and recovery authorization by the authority
  assigned in governing owner policy, without a universal human
  technical-judgment gate.
- They preserve the observer/armer split, explicit per-PR merge authorization,
  and the no-silent-requeue rule for both `watch` and ordinary `land`.
- Existing explicit `--reenqueue` semantics and guards are unchanged; no
  automatic requeue, extra framework, or approval exemption is introduced.
- Verify generated `cargo xtask pr --help`/relevant `land --help` wording
  against sources. Run focused parser/behavior tests around `PrCommand`
  (`xtask/src/cli.rs`) and existing ejection/re-enqueue tests
  (`xtask/src/pr/land.rs`, `watch.rs`, and `execute.rs`) as appropriate, then
  the repository's applicable check ladder. No implementation or validation run
  is authorized before spec approval.

## Boundaries

No code or behavior change, new retry/requeue mechanism, watch/land handoff
change, merge-gate change, new framework, or edit to installed/external skills.
Update ADR-0087 only to correct its technical-judgment ownership statement;
preserve its observer/armer and merge-approval decisions. Do not alter ADR-0135
or promote the explicit-reenqueue ADR draft. This is a documentation/help
correction only; spec approval is required before implementation, and later
merge approval remains separate.
