#!/usr/bin/env bash
set -euo pipefail

workflow=.github/workflows/theme-repository.yml
fixture_workflow=.github/workflows/theme-repository-fixture.yml
semver_tag_regex="$(sed -n "s/^          semver_tag_regex='\(.*\)'$/\1/p" "$workflow")"

if ! grep -Fxq '      - v0.0.0-theme-repository-fixture.[1-9]*' "$fixture_workflow"; then
  echo "$fixture_workflow must trigger the SemVer fixture namespace" >&2
  exit 1
fi

if [[ -z "$semver_tag_regex" ]]; then
  echo "could not find the release-tag validator in $workflow" >&2
  exit 1
fi

assert_accepts() {
  local tag="$1"
  if ! [[ "$tag" =~ $semver_tag_regex ]]; then
    echo "expected $tag to be accepted" >&2
    exit 1
  fi
}

assert_rejects() {
  local tag="$1"
  if [[ "$tag" =~ $semver_tag_regex ]]; then
    echo "expected $tag to be rejected" >&2
    exit 1
  fi
}

for tag in \
  v0.0.0 \
  v1.2.3 \
  v12.34.56 \
  v0.0.0-theme-repository-fixture.1 \
  v1.2.3-rc.1 \
  v1.2.3-0 \
  v1.2.3-alpha.01beta \
  v1.2.3+build.5 \
  v1.2.3-rc.1+build.5; do
  assert_accepts "$tag"
done

for tag in \
  theme-repository-fixture-1 \
  v01.2.3 \
  v1.02.3 \
  v1.2.03 \
  v1.2 \
  v1.2.3- \
  v1.2.3-01 \
  v1.2.3-alpha.01 \
  v1.2.3-rc.01.1 \
  v1.2.3-rc..1 \
  v1.2.3+ \
  v1.2.3+build..5; do
  assert_rejects "$tag"
done

release_block="$(
  sed -n '/^      - name: Create immutable release$/,$p' "$workflow" \
    | sed -n '/^          set -euo pipefail$/,$s/^          //p'
)"

if [[ -z "$release_block" ]]; then
  echo "could not find the immutable-release shell block in $workflow" >&2
  exit 1
fi

run_existing_release_case() {
  local name="$1"
  local tag="$2"
  local draft="$3"
  local prerelease="$4"
  local asset_count="$5"
  local downloaded_bytes="$6"
  local expected_status="$7"
  local expected_edit="$8"
  local expects_upload="$9"
  local expects_download="${10}"
  local temp
  temp="$(mktemp -d)"
  trap 'rm -rf "$temp"' RETURN
  mkdir -p "$temp/bin"
  printf 'package bytes\n' >"$temp/theme-package.zip"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'case "$1 $2" in' \
    '  "release view")' \
    '    case " $* " in' \
    '      *" --json assets "*) printf "%s\\n" "$MOCK_ASSET_COUNT" ;;' \
    '      *" --json isDraft "*) printf "%s\\n" "$MOCK_DRAFT" ;;' \
    '      *" --json isPrerelease "*) printf "%s\\n" "$MOCK_PRERELEASE" ;;' \
    '      *) exit 0 ;;' \
    '    esac' \
    '    ;;' \
    '  "release download")' \
    '    printf "%s\\n" "$*" >>"$MOCK_GH_LOG"' \
    '    for ((index = 1; index <= $#; index++)); do' \
    '      if [[ "${!index}" = --dir ]]; then next_index=$((index + 1)); directory="${!next_index}"; break; fi' \
    '    done' \
    '    if test "$MOCK_DOWNLOADED_BYTES" = identical; then cp "$PACKAGE" "$directory/theme-package.zip"; else printf "different bytes\\n" >"$directory/theme-package.zip"; fi' \
    '    ;;' \
    '  "release upload"|"release edit") printf "%s\\n" "$*" >>"$MOCK_GH_LOG" ;;' \
    '  *) echo "unexpected gh invocation: $*" >&2; exit 1 ;;' \
    'esac' >"$temp/bin/gh"
  chmod +x "$temp/bin/gh"
  : >"$temp/gh.log"

  local status
  set +e
  PATH="$temp/bin:$PATH" \
    TAG="$tag" \
    PACKAGE="$temp/theme-package.zip" \
    RUNNER_TEMP="$temp" \
    MOCK_ASSET_COUNT="$asset_count" \
    MOCK_DOWNLOADED_BYTES="$downloaded_bytes" \
    MOCK_DRAFT="$draft" \
    MOCK_PRERELEASE="$prerelease" \
    MOCK_GH_LOG="$temp/gh.log" \
    bash -c "$release_block" >"$temp/workflow.out" 2>"$temp/workflow.err"
  status=$?
  set -e
  if ((status != expected_status)); then
    echo "$name expected status $expected_status, got $status" >&2
    sed 's/^/'"$name: "'/g' "$temp/workflow.err" >&2
    exit 1
  fi

  local actual_edit
  actual_edit="$(grep '^release edit ' "$temp/gh.log" || :)"
  if [[ "$actual_edit" != "$expected_edit" ]]; then
    echo "$name expected edit ${expected_edit@Q}, got ${actual_edit@Q}" >&2
    exit 1
  fi
  if [[ "$expects_upload" = true ]]; then
    if ! grep -Fxq "release upload $tag $temp/theme-package.zip#theme-package.zip" "$temp/gh.log"; then
      echo "$name did not upload the missing asset" >&2
      exit 1
    fi
  elif grep -q '^release upload ' "$temp/gh.log"; then
    echo "$name unexpectedly uploaded an asset" >&2
    exit 1
  fi
  if [[ "$expects_download" = true ]]; then
    if ! grep -Fxq "release download $tag --pattern theme-package.zip --dir $temp/existing-theme-package" "$temp/gh.log"; then
      echo "$name did not download the existing asset" >&2
      exit 1
    fi
  elif grep -q '^release download ' "$temp/gh.log"; then
    echo "$name unexpectedly downloaded an asset" >&2
    exit 1
  fi
}

run_existing_release_case stable_matching v1.2.3 false false 0 identical 0 '' true false
run_existing_release_case stable_mismatched_draft v1.2.3 true true 0 identical 0 \
  'release edit v1.2.3 --draft=false --prerelease=false' true false
run_existing_release_case prerelease_matching_draft v1.2.3-rc.1 true true 0 identical 0 \
  'release edit v1.2.3-rc.1 --draft=false' true false
run_existing_release_case prerelease_mismatched v1.2.3-rc.1 false false 0 identical 0 \
  'release edit v1.2.3-rc.1 --prerelease=true' true false
run_existing_release_case identical_asset v1.2.3-rc.1 true false 1 identical 0 \
  'release edit v1.2.3-rc.1 --draft=false --prerelease=true' false true
run_existing_release_case different_asset v1.2.3 true true 1 different 1 '' false true
run_existing_release_case duplicate_assets v1.2.3 true true 2 identical 1 '' false false

run_absent_release_case() {
  local name="$1"
  local tag="$2"
  local expects_prerelease="$3"
  local temp
  temp="$(mktemp -d)"
  trap 'rm -rf "$temp"' RETURN
  mkdir -p "$temp/bin"
  printf 'package bytes\n' >"$temp/theme-package.zip"
  printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' 'case "$1 $2" in' \
    '  "release view") exit 1 ;;' \
    '  "release create"|"release upload"|"release download"|"release edit") printf "%s\\n" "$*" >>"$MOCK_GH_LOG" ;;' \
    '  *) echo "unexpected gh invocation: $*" >&2; exit 1 ;;' \
    'esac' >"$temp/bin/gh"
  chmod +x "$temp/bin/gh"
  : >"$temp/gh.log"

  PATH="$temp/bin:$PATH" \
    TAG="$tag" \
    PACKAGE="$temp/theme-package.zip" \
    RUNNER_TEMP="$temp" \
    MOCK_GH_LOG="$temp/gh.log" \
    bash -c "$release_block"

  local expected_create
  expected_create="release create $tag $temp/theme-package.zip#theme-package.zip --title $tag"
  if [[ "$expects_prerelease" = true ]]; then
    expected_create+=" --prerelease"
  fi
  if ! grep -Fxq "$expected_create" "$temp/gh.log"; then
    echo "$name expected create ${expected_create@Q}" >&2
    exit 1
  fi
  if grep -Eq '^release (upload|download|edit) ' "$temp/gh.log"; then
    echo "$name mutated an existing release before creation" >&2
    exit 1
  fi
  if [[ "$(wc -l <"$temp/gh.log")" -ne 1 ]]; then
    echo "$name made unexpected release calls" >&2
    exit 1
  fi
}

run_absent_release_case stable v1.2.3 false
run_absent_release_case prerelease v1.2.3-rc.1 true
run_absent_release_case stable_build v1.2.3+build.5 false
run_absent_release_case prerelease_build v1.2.3-rc.1+build.5 true
