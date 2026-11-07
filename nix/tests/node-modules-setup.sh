#!/usr/bin/env bash
# Exercise the installed executable, not a copy of its implementation.
set -euo pipefail
setup=$1
types=$2
playwright=$3
root=$(mktemp -d)
cd "$root"
mkdir -p closure/{@types/node,@playwright/test,typescript,.bin} pinned-playwright
printf 'hidden\n' > closure/.package-lock.json
printf 'closure\n' > closure/@playwright/test/sentinel
fixture_types="$root/closure"
fixture_playwright="$root/pinned-playwright"

check_links() {
  test -d end2end/node_modules
  test ! -L end2end/node_modules
  test "$(readlink end2end/node_modules/typescript)" = "$fixture_types/typescript"
  test "$(readlink end2end/node_modules/@types)" = "$fixture_types/@types"
  test ! -L end2end/node_modules/@playwright
  test "$(readlink end2end/node_modules/@playwright/test)" = "$fixture_playwright"
  test ! -e end2end/node_modules/.bin
  test ! -e end2end/node_modules/.package-lock.json
  test "$(< closure/@playwright/test/sentinel)" = closure
}

# An absent destination is populated; reruns refresh current entries only.
test ! -e end2end/node_modules
"$setup" "$fixture_types" "$fixture_playwright"
check_links
printf 'unrelated\n' > end2end/node_modules/local-sentinel
rm end2end/node_modules/typescript
mkdir end2end/node_modules/typescript
printf 'obsolete\n' > end2end/node_modules/typescript/old
"$setup" "$fixture_types" "$fixture_playwright"
check_links
test "$(< end2end/node_modules/local-sentinel)" = unrelated
"$setup" "$fixture_types" "$fixture_playwright"
check_links
test "$(< end2end/node_modules/local-sentinel)" = unrelated

# The destination link itself must go, not anything in its external referent.
mv end2end/node_modules preserved-directory
mkdir -p external/typescript external/@playwright/test
printf 'external\n' > external/typescript/sentinel
printf 'external-playwright\n' > external/@playwright/test/sentinel
cp -a external external-before
ln -s "$root/external" end2end/node_modules
"$setup" "$fixture_types" "$fixture_playwright"
check_links
diff -r external-before external
test "$(< preserved-directory/local-sentinel)" = unrelated

# Also prove both real Nix pins are accepted without substituting an npm copy.
"$setup" "$types" "$playwright"
test "$(readlink end2end/node_modules/@types)" = "$types/@types"
test "$(readlink end2end/node_modules/@playwright/test)" = "$playwright"
printf 'absent, refresh, repeat, dot omission, referent safety, and real pins passed\n'
