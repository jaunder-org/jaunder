# The host-run qualification executable. Nix builds it from its real Cargo path
# closure and binds the executable to its own output; it never executes inside a
# derivation (ADR-0239, ADR-0246).
{ mkHostTool }:
let
  package = mkHostTool {
    pname = "qualification-host";
    member = "tools/qualification";
    bins = [ "qualification-host" ];
    # The evidence schema is compiled into the executable.
    extraSourceAdmits = relative: relative == "docs/production-baseline.schema.json";
    env.JAUNDER_QUALIFICATION_BUILD_NIX_OUTPUT = builtins.placeholder "out";
  };
in
assert package.sourceAdmits "tools/qualification/src/main.rs" "regular";
assert package.sourceAdmits "docs/production-baseline.schema.json" "regular";
assert !(package.sourceAdmits "tools/devtool/src/main.rs" "regular");
package
