# Outcome

Update `.mutants-loop/PROTOCOL.md` so a failed mutation-test iteration preserves
earlier and uncertain work instead of recommending whole-file rollback. The
protocol will match the maintained `jaunder-mutants` skill's ownership-proven
recovery rule.

# Approach and tradeoffs

Before any iteration edits—including journal writes or formatter-capable
commands—the operator records the dirty, staged, and untracked inventory and
diff. On a failed gate, retain failure evidence and undo only changes
demonstrably owned by that iteration, using that pre-iteration record. Preserve
pre-existing and uncertain edits, including formatter mutations; a red gate or
deliberate skip does not establish ownership. The direct Lead owns the current
inventory and resolves uncertain preservation. If safe selective undo cannot be
established, report the blocker and wait for Lead direction rather than
discarding work or advancing. This uses the existing inventory/diff and Lead
authority; it adds no rollback machinery or bypass.

# Acceptance

- `.mutants-loop/PROTOCOL.md` no longer recommends unconditional whole-file
  rollback.
- Recovery directs selective undo only for changes proven to belong to this
  iteration and preserves earlier, uncertain, and formatter changes.
- Uncertain ownership routes to the direct Lead; no human inventory request or
  silent discard is introduced.
- Existing skip and three-strikes guidance remains intact.
- Explicit no-merge / user merge-approval guidance remains intact.
- Compare the entire tracked protocol against the maintained current
  `jaunder-mutants` skill; no contradictory recovery advice remains.
- Validate the docs-only change with the contributor guide's Markdown-sensitive
  precommit checks: Prettier, sequence/identifier collision checks, ADR bundle,
  documentation links, and flow-document parity, using the repository's
  supported command path.

# Boundaries

Only `.mutants-loop/PROTOCOL.md` changes; the maintained skill is the reference,
not a target for edits. No implementation, gate bypass, rollback framework,
blanket archive/stash escape, issue-scope expansion, or merge operation. Human
spec approval precedes implementation; cold review and merge approval remain
Lead-owned gates.
