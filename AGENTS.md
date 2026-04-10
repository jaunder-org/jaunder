Please also reference the following rules as needed. The list below is provided in TOON format, and `@` stands for the project root directory.

rules[8]:
  - path: @.opencode/memories/architecture.md
  - path: @.opencode/memories/commands.md
  - path: @.opencode/memories/coverage.md
  - path: @.opencode/memories/database.md
  - path: @.opencode/memories/planning.md
  - path: @.opencode/memories/rust.md
    applyTo[1]: *.rs
  - path: @.opencode/memories/technology.md
  - path: @.opencode/memories/testing.md

- **ALWAYS** seek clarification if you are uncertain about your instructions.
- **EVERY** commit should be an atomic unit of work.
- **ALWAYS** commit a refactor before the changes that uses it.
- **ALWAYS** request review from the user before committing.
- **ALWAYS** verify after each step:
  1. `cargo build` succeeds.
  2. `cargo nextest run` passes.
  3. `cargo clippy -- -D warnings` is clean.
  4. `scripts/check-converage` succeeds.
  5. `nix flake check` passes.
