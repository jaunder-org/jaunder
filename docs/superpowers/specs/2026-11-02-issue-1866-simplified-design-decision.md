# #1866 Dependabot Cargo security updates — scope/approval packet

**Status:** Approved native-only implementation scope. The human responded
**“Proceed”** directly to this exact minimal plan; that approval authorizes only
the configuration and setting described below. The Manager reconciled scope:
this supersedes the earlier new daily/manual `cargo-deny` workflow requirement
and all custom detector/publisher/repair/reconciliation requirements. The
rejected original remains unchanged as historical evidence; it is not the active
design. Implementation has begun under that approval.

## Outcome and scope

Enable GitHub-native Dependabot Cargo **security updates** for manifests at `/`,
`/tools`, and `/xtask`. Its advisory-triggered PRs receive milestone #34,
**Dependency security automation**, and use ordinary CI and human review.
`cargo-deny` remains the backstop for repository policy/advisories and findings
without a matching supported Dependabot alert. Dependabot is not assumed to
cover or reconcile every `cargo-deny` finding.

No custom detector, publisher, repair engine, cross-source reconciliation, or
routine Cargo version-update PRs. Leave existing GitHub Actions updates
unchanged. Leave #1858's manual repair untouched. Existing matching native
Dependabot PR behavior is GitHub's supported PR/update path; do not add custom
PR reuse, branch repair, or human-PR linkage machinery.

## Proposed minimal configuration

Add this Cargo entry to `.github/dependabot.yml`, preserving the existing GitHub
Actions entry verbatim:

```yaml
- package-ecosystem: "cargo"
  directories:
    - "/"
    - "/tools"
    - "/xtask"
  schedule:
    interval: "daily"
  open-pull-requests-limit: 0
  milestone: 34
```

`directories` targets the three manifest locations. Dependabot passes
`milestone` as the GitHub issue-update API's milestone number, so use #34's
number (`34`), not its database ID (`19142616`). GitHub recommends
`open-pull-requests-limit: 0` to disable routine version-update PRs while
leaving security updates enabled. Daily schedule configures the version-update
schedule, not security-update cadence; security updates are alert-triggered. No
regular Cargo update PRs are intended.

## Repository setting and validation

After approval, enable repository **automated security fixes / Dependabot
security updates** (human/authorized repository-setting operation; do not alter
other security settings). Initial API readback was
`enabled:false, paused:false`. The approved operation changes only this
security-updates setting; permission/access failures must be recorded without
changing other settings. Alerts were previously confirmed enabled;
graph/org-policy prerequisites remain to verify before relying on updates.
Milestone readback confirms open milestone #34, ID `19142616`.

Validate YAML/configuration and read back the setting/config. Confirm normal CI
and human review on an actual generated Dependabot PR if one occurs. Do not
manufacture an advisory/PR or claim a generated PR, CI result, or real-world
security-update proof when none exists; document that limitation. No auto-merge,
suppressions, policy weakening, or new App.

## Approval and boundaries

Approval provenance: human **“Proceed”** in direct response to this exact
native-only plan. Manager's explicit reconciliation supersedes the earlier new
daily/manual `cargo-deny` workflow requirement. Existing `cargo-deny` gates
remain the backstop, unchanged. No custom detector, publisher, repair engine,
cross-source reconciliation, extra permissions/App/credentials, auto-merge,
suppressions, or interference with #1858. Native Dependabot security updates do
not claim scheduled advisory scans or complete `cargo-deny` coverage. Generated
PRs remain subject to normal CI and human review; actual generated PR/CI
behavior is unobserved unless such a PR occurs.
