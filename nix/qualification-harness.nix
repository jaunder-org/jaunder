# Host-only qualification inputs. Nix owns each member; this output joins their
# existing owners without introducing an input to any application/check producer.
{
  pkgs,
  xtask,
  testSupport,
  browser,
}:
let
  browserRunner = pkgs.writeShellApplication {
    name = "qualification-playwright";
    text = ''
      export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
      exec ${pkgs.nodejs}/bin/node ./node_modules/@playwright/test/cli.js "$@"
    '';
  };
in
(pkgs.linkFarm "jaunder-qualification-harness" [
  { name = "bin/xtask"; path = "${xtask}/bin/xtask"; }
  { name = "bin/test-support"; path = "${testSupport}/bin/test-support"; }
  { name = "bin/node"; path = "${pkgs.nodejs}/bin/node"; }
  { name = "bin/playwright"; path = "${browserRunner}/bin/qualification-playwright"; }
  { name = "browser"; path = browser; }
  { name = "browsers"; path = pkgs.playwright-driver.browsers; }
  # Manifest evidence reads the definition owned by the selected helper build.
  { name = "seed-manifest-source"; path = "${testSupport.src}/test-support/src/lib.rs"; }
  # VM expressions are evaluated from the immutable source pin. Bind their
  # concern-owned definitions here as well as recording realized VM outputs.
  { name = "vm-inputs"; path = ./.; }
  { name = "vm-export"; path = ../flake.nix; }
  { name = "vm-lock"; path = ../flake.lock; }
]).overrideAttrs (_: {
  passthru.tests.browser-startup = pkgs.runCommand "qualification-browser-startup" { } ''
    # Load the real suite from a writable copy. Using the store CLI instead of
    # the copied CLI creates duplicate Playwright globals and fails this proof.
    cp -r ${browser} browser
    chmod -R u+w browser
    cd browser
    export HOME="$TMPDIR"
    ${browserRunner}/bin/qualification-playwright test --list > "$out"
  '';
})
