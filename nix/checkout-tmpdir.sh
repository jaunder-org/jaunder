# Bind host temp routing to the entry checkout: <checkout>/.tmp. Keep it
# shallow: Unix socket paths that tests bind beneath it are capped at 107 bytes.
# The default, ci, mutants and ci-nix shellHooks inline this file; .envrc
# sources it after `use flake`, because nix-direnv restores the pre-entry
# TMPDIR after running the shellHook.
# Exit (not return) stops startup: nix develop must not run a command, and
# direnv must not load the environment, after a failure.
if ! jaunder_checkout_root="$(git rev-parse --show-toplevel)" || [[ "$jaunder_checkout_root" != /* ]]; then
  printf 'jaunder: cannot resolve an absolute Git checkout root for TMPDIR\n' >&2
  exit 1
fi
jaunder_tmpdir="$jaunder_checkout_root/.tmp"
# Exercise directory use as well as creation; an existing path is not proof
# that host tools can create temporary files there.
if ! mkdir -p -- "$jaunder_tmpdir" || ! jaunder_tmp_probe="$(mktemp "$jaunder_tmpdir/.shell-entry.XXXXXXXXXX")"; then
  printf 'jaunder: cannot create or use TMPDIR %s\n' "$jaunder_tmpdir" >&2
  exit 1
fi
if ! rm -- "$jaunder_tmp_probe"; then
  printf 'jaunder: cannot remove TMPDIR usability probe %s\n' "$jaunder_tmp_probe" >&2
  exit 1
fi
export TMPDIR="$jaunder_tmpdir"
unset jaunder_checkout_root jaunder_tmpdir jaunder_tmp_probe
