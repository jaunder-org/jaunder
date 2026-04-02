Please also reference the following rules as needed. The list below is provided in TOON format, and `@` stands for the project root directory.

rules[8]:
  - path: @.gemini/memories/architecture.md
  - path: @.gemini/memories/commands.md
  - path: @.gemini/memories/coverage.md
  - path: @.gemini/memories/database.md
  - path: @.gemini/memories/planning.md
  - path: @.gemini/memories/rust.md
    applyTo[1]: *.rs
  - path: @.gemini/memories/technology.md
  - path: @.gemini/memories/testing.md

# Additional Conventions Beyond the Built-in Functions

As this project's AI coding tool, you must follow the additional conventions below, in addition to the built-in functions.

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
