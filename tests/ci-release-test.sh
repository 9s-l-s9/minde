#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Real Git history and release commands, with publication replaced by argv capture.
set -eu
cd "$(dirname "$0")/.."
scratch=$(mktemp -d "${TMPDIR:-/tmp}/minde-ci-release-test.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$scratch/scripts" "$scratch/bin"
cp scripts/ci scripts/ci-release "$scratch/scripts/"
cat >"$scratch/check" <<'EOF'
#!/bin/sh
touch gate-ran
exit 83
EOF
cat >"$scratch/bin/gh" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" >published-args
EOF
chmod +x "$scratch/check" "$scratch/bin/gh"
cd "$scratch"
PATH="$scratch/bin:$PATH"
export PATH
# Keep fixtures independent of the maintainer's signing, hooks and identity.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES
git init -q -b main
git config user.name 'minde release test'
git config user.email 'minde-test@example.invalid'
git config core.hooksPath /dev/null
printf '%s\n' '[package]' 'version = "1.0.0-rc1"' >Cargo.toml
git add Cargo.toml
git commit -qm 'Initial release candidate'
base=$(git rev-parse HEAD)
git tag -a v1.0.0-rc1 -m 'Release candidate'
git commit --allow-empty -qm 'Later main commit'
main=$(git rev-parse HEAD)
git update-ref refs/remotes/origin/main "$main"
git checkout -q --detach "$base"
export MINDE_CI_ENV=1 GITHUB_ACTIONS=true
export GITHUB_REF_NAME=v1.0.0-rc1 GITHUB_REF=refs/tags/v1.0.0-rc1 GITHUB_SHA="$base"
unset MINDE_CI_OUTPUT
fail() { echo "FAIL - $*" >&2; exit 1; }
check() { sh scripts/ci-release "$@" >result 2>error; }
reject() {
    for mode in --check --publish; do
        rm -f published-args
        if check "$mode"; then fail "accepted $1 ($mode)"; fi
        grep -Fq "$2" error || { cat error >&2; fail "wrong error for $1"; }
        [ ! -e published-args ] || fail "published $1"
    done
    rm -f gate-ran
    if sh scripts/ci --release-artifacts >result 2>error; then fail "CI accepted $1"; fi
    grep -Fq "$2" error || { cat error >&2; fail "wrong CI error for $1"; }
    [ ! -e gate-ran ] || fail "CI ran gates for $1"
    echo "ok - reject $1 before gates and publication"
}
reaches_gate() {
    rm -f gate-ran
    status=0
    sh scripts/ci "$@" >result 2>error || status=$?
    if [ "$status" -ne 83 ] || [ ! -e gate-ran ]; then
        cat error >&2
        fail 'valid CI path missed gates'
    fi
}
publish() {
    artifact_dir=${MINDE_CI_OUTPUT:-build/ci/artifacts}
    mkdir -p "$artifact_dir"
    for name in "minde-$1.tar.gz" "minde-$1-vendored.tar.gz" SHA256SUMS; do
        printf '%s\n' fixture >"$artifact_dir/$name"
    done
    check --publish || { cat error >&2; fail "publish $1"; }
    printf '%s\n' release create "$GITHUB_REF_NAME" \
        "$artifact_dir/minde-$1.tar.gz" "$artifact_dir/minde-$1-vendored.tar.gz" "$artifact_dir/SHA256SUMS" \
        --verify-tag --title "$GITHUB_REF_NAME" --notes "Source archives for $GITHUB_SHA." >expected-args
    [ "$2" = stable ] || printf '%s\n' --prerelease >>expected-args
    cmp expected-args published-args || fail "incorrect publication arguments for $1"
    echo "ok - $1 publication arguments"
}

check --check || { cat error >&2; fail 'annotated ancestor tag'; }
reaches_gate --release-artifacts
publish 1.0.0-rc1 prerelease
GITHUB_SHA=$(git rev-parse refs/tags/v1.0.0-rc1)
check --check || { cat error >&2; fail 'annotated event revision'; }
GITHUB_SHA=$base
echo 'ok - annotated tag on an older main commit'

GITHUB_REF_NAME=v9.9.9 GITHUB_REF=refs/tags/v9.9.9
reject 'version mismatch' 'does not match Cargo version'
GITHUB_REF_NAME=v1.0.0-rc1 GITHUB_REF=refs/heads/main
reject 'branch event' 'version-tag event is required'
GITHUB_REF=refs/tags/v1.0.0-rc1
GITHUB_SHA=$main
reject 'event revision mismatch' 'does not match the event revision'
GITHUB_SHA=
reject 'missing event revision' 'event revision is missing'
GITHUB_SHA=missing-event-object
reject 'unknown event revision' 'event revision must resolve'
GITHUB_SHA=$base
git checkout -q --detach "$main"
reject 'checkout mismatch' 'does not match the checkout'
git checkout -q --detach "$base"
git tag -d v1.0.0-rc1 >/dev/null
reject 'missing tag' 'release tag must resolve'
git tag -a v1.0.0-rc1 -m 'Release candidate'
git update-ref -d refs/remotes/origin/main
reject 'missing main history' 'origin/main is missing'
git update-ref refs/remotes/origin/main "$main"
git commit --allow-empty -qm 'Unmerged feature commit'
GITHUB_SHA=$(git rev-parse HEAD)
git tag -f v1.0.0-rc1 >/dev/null
reject 'tag outside main' 'not on origin/main'
git checkout -q --detach "$base"
git tag -f v1.0.0-rc1 >/dev/null
GITHUB_SHA=$base

rm -f build/ci/artifacts/SHA256SUMS published-args
if check --publish; then fail 'published missing artifacts'; fi
grep -Fq 'missing release artifact' error || fail 'wrong missing-artifact error'
[ ! -e published-args ] || fail 'publication ran without checksums'
echo 'ok - publication requires matching artifacts'

# Stable tags, including hyphens confined to build metadata, are not prereleases.
for version in 1.0.0 1.0.0+build-one; do
    printf '%s\n' '[package]' "version = \"$version\"" >Cargo.toml
    git add Cargo.toml
    git commit -qm 'Stable version'
    GITHUB_SHA=$(git rev-parse HEAD)
    GITHUB_REF_NAME=v$version GITHUB_REF=refs/tags/v$version
    git tag "$GITHUB_REF_NAME"
    git update-ref refs/remotes/origin/main "$GITHUB_SHA"
    # Spaces in a caller-selected output directory must stay inside argv.
    export MINDE_CI_OUTPUT='custom artifacts'
    publish "$version" stable
done

# Untagged local artifact checks and normal CI gates keep their existing scope.
GITHUB_REF=refs/heads/main GITHUB_REF_NAME=main
reaches_gate --ci-scheme
unset GITHUB_ACTIONS GITHUB_REF GITHUB_REF_NAME GITHUB_SHA
reaches_gate --release-artifacts
echo 'ok - ordinary CI and local archive checks do not require release tags'
