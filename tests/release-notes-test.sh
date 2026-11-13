#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Run the release driver with real Git/checksums and lightweight build fixtures.
set -eu
cd "$(dirname "$0")/.."
scratch=$(mktemp -d "${TMPDIR:-/tmp}/minde-release-notes-test.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$scratch/scripts" "$scratch/bin"
cp scripts/release "$scratch/scripts/"
for tool in make guix; do
    printf '#!/bin/sh\nprintf "%%s\\n" "$*" >> %s-ran\n' "$tool" >"$scratch/bin/$tool"
done
cat >"$scratch/scripts/create-release-archives" <<'EOF'
#!/bin/sh
set -eu
touch archives-ran
printf 'source\n' >"$2/minde-$1.tar.gz"
printf 'vendored\n' >"$2/minde-$1-vendored.tar.gz"
EOF
cat >"$scratch/scripts/check-guix-package" <<'EOF'
#!/bin/sh
touch package-ran
EOF
chmod +x "$scratch/bin/"* "$scratch/scripts/"*
cd "$scratch"
PATH="$scratch/bin:$PATH"
export PATH MINDE_RELEASE_ENV=1 MINDE_RELEASE_GATES=check
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES
git init -q -b main
git config user.name 'minde release test'
git config user.email 'minde-test@example.invalid'
git config core.hooksPath /dev/null
printf '%s\n' '/build/' '/*-ran' '/stdout' '/stderr' >.gitignore
printf '%s\n' '[package]' 'version = "1.0.0-rc1"' >Cargo.toml
version=1.0.0-rc1
output=build/release/$version
fail() { echo "FAIL - $*" >&2; exit 1; }
commit_fixture() {
    git add .
    git commit --allow-empty -qm 'Release notes fixture'
    rm -f make-ran guix-ran archives-ran package-ran
}
reject() {
    commit_fixture
    mkdir -p "$output"
    printf 'previous notes\n' >"$output/RELEASE-NOTES.md"
    printf 'previous checksums\n' >"$output/SHA256SUMS"
    if sh scripts/release "$version" >stdout 2>stderr; then fail "accepted $1"; fi
    grep -Fq 'nonempty ## Unreleased section' stderr || { cat stderr >&2; fail "wrong error for $1"; }
    [ "$(cat "$output/RELEASE-NOTES.md")" = 'previous notes' ] || fail "overwrote notes for $1"
    [ "$(cat "$output/SHA256SUMS")" = 'previous checksums' ] || fail "overwrote checksums for $1"
    for marker in make-ran guix-ran archives-ran package-ran; do
        [ ! -e "$marker" ] || fail "ran $marker for $1"
    done
    echo "ok - reject $1 before builds or output changes"
}
accept() {
    commit_fixture
    sh scripts/release "$version" >stdout 2>stderr || { cat stderr >&2; fail "rejected $1"; }
    notes=$output/RELEASE-NOTES.md
    grep -Fq -- "$2" "$notes" || fail "lost release content for $1"
    # Markdown code delimiters are intentional literal characters.
    # shellcheck disable=SC2016
    grep -Fq "$(printf 'Revision: `%s`' "$(git rev-parse HEAD)")" "$notes" || fail 'wrong revision in notes'
    if grep -Fq 'historical entry' "$notes"; then fail 'included a different release'; fi
    grep -Fq '## Artifacts' "$notes" || fail 'missing artifact section'
    [ "$(wc -l <"$output/SHA256SUMS")" -eq 3 ] || fail 'incomplete checksums'
    (cd "$output" && sha256sum --check --status SHA256SUMS) || fail 'invalid release checksums'
    for marker in make-ran guix-ran archives-ran package-ran; do
        [ -e "$marker" ] || fail "skipped $marker for $1"
    done
    echo "ok - $1 keeps notes, revision and checksums"
}

printf '# Changelog\n\n## 1.0.0-rc1\n\n- historical entry\n' >CHANGELOG.md
reject 'missing Unreleased heading'
printf '# Changelog\n\n## Unreleased\n \t \n\n## 0.9.0\n- historical entry\n' >CHANGELOG.md
reject 'whitespace-only changes'
printf '# Changelog\n\n## Unreleased\n' >CHANGELOG.md
reject 'empty section at EOF'
# shellcheck disable=SC2016
printf '# Changelog\n\n## Unreleased\n\nTarget version: `1.0.0-rc1`.\n\n### Fixed\n' >CHANGELOG.md
reject 'heading and version scaffolding'
printf '# Changelog\n\n## Unreleased notes\n\n- A change\n' >CHANGELOG.md
reject 'prefix heading'
printf '# Changelog\n\n## Unreleased\n- First\n\n## Unreleased\n- Second\n' >CHANGELOG.md
reject 'duplicate Unreleased sections'
unset MINDE_RELEASE_ENV
reject 'invalid notes outside the Guix environment'
export MINDE_RELEASE_ENV=1

cat >CHANGELOG.md <<'EOF'
# Changelog

## Unreleased

Target version: `1.0.0-rc1`.

### Fixed

- Preserve 100% of backslashes like C:\path and Grüße.

## 0.9.0

- historical entry
EOF
accept 'formatted change entries' '- Preserve 100% of backslashes like C:\path and Grüße.'
printf '# Changelog\n\n## Unreleased  \n\nParagraph changes also work.\n' >CHANGELOG.md
accept 'paragraph change with trailing heading spaces' 'Paragraph changes also work.'
