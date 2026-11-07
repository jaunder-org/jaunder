{ pkgs }:
pkgs.writeShellApplication {
  name = "setup-e2e-node-modules";
  runtimeInputs = [ pkgs.coreutils ];
  text = ''
    types=''${1:?expected pinned type-dependency node_modules}
    playwright=''${2:?expected pinned Playwright test package}
    dest=end2end/node_modules

    # Never populate through a stale destination link into another tree.
    if [[ -L "$dest" ]]; then
      rm -- "$dest"
    fi
    mkdir -p -- "$dest"
    # Refresh only current visible packages; unrelated local entries survive.
    for entry in "$types"/*; do
      [[ -e "$entry" || -L "$entry" ]] || continue
      target="$dest/''${entry##*/}"
      rm -rf -- "$target"
      ln -s -- "$entry" "$target"
    done
    # Browser parity requires Nix's Playwright, not the npm closure's copy.
    rm -rf -- "$dest/@playwright"
    mkdir -p -- "$dest/@playwright"
    ln -s -- "$playwright" "$dest/@playwright/test"
  '';
}
