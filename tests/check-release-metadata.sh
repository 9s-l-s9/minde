#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

[ -x scripts/mindectl ] || {
    echo "release metadata: scripts/mindectl must be executable" >&2
    exit 1
}
[ -x scripts/minde-cmd ] || {
    echo "release metadata: scripts/minde-cmd must be executable" >&2
    exit 1
}
for helper in tests/applications.sh tests/soak.sh tests/portable-e2e.sh \
    tests/lib/nested-compositor.sh scripts/generate-docs scripts/capture-demos \
    scripts/generate-api-reference.scm scripts/generate-keymap-reference.scm \
    scripts/generate-demo-manifest.scm scripts/generate-manual.scm \
    tests/check-generated-docs.sh tests/check-doc-links.sh \
    tests/check-demos.sh scripts/create-release-archives \
    scripts/check-guix-package scripts/release \
    tests/check-release-archives.sh tests/check-package-contents.sh; do
    [ -x "$helper" ] || {
        echo "release metadata: $helper must be executable" >&2
        exit 1
    }
done

fail() {
    echo "release metadata: $*" >&2
    exit 1
}

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || fail "Cargo.toml has no package version"

grep -q '^license = "GPL-3.0-or-later"$' Cargo.toml ||
    fail "Cargo license is not GPL-3.0-or-later"
grep -q "(define %project-version \"$version\")" guix.scm ||
    fail "Cargo and Guix versions differ"
grep -q "(version \"$version\")" guix/foundation.scm ||
    fail "Cargo and foundation package versions differ"
grep -q "(version \"$version\")" guix/ui.scm ||
    fail "Cargo and UI package versions differ"
grep -q '(license license:gpl3+)' guix.scm ||
    fail "Guix license is not GPL-3.0-or-later"
grep -q "Target version: \`$version\`" CHANGELOG.md ||
    fail "CHANGELOG target version differs"
grep -q "Development version: \`$version\`" README.md ||
    fail "README development version differs"
grep -q 'github.com/9s-l-s9/minde' Cargo.toml guix.scm ||
    fail "canonical repository is absent from package metadata"

for file in COPYING NOTICE CHANGELOG.md CONTRIBUTING.md SECURITY.md SUPPORT.md UNEXPECTED.md \
    LICENSES/GPL-3.0-or-later.txt LICENSES/MIT.txt; do
    [ -s "$file" ] || fail "$file is missing or empty"
done

revision=$(sed -n 's/^Pinned revision: //p' NOTICE)
[ ${#revision} -eq 40 ] || fail "NOTICE does not contain a full pinned revision"
grep -q "$revision" Cargo.toml || fail "Cargo Smithay revision differs from NOTICE"
grep -q "$revision" guix/cargo-config.toml ||
    fail "vendored Cargo configuration differs from NOTICE"

grep -q '(version 1)' scheme/default-config.scm ||
    fail "portable configuration schema is not version 1"
# Backticks are literal Markdown delimiters in these patterns.
# shellcheck disable=SC2016
grep -q '^Status and diagnostic schemas: `1`\.$' doc/releasing.md ||
    fail "release guide does not record the external schema version"
# shellcheck disable=SC2016
grep -q '^Persistent state formats: `1`\.$' doc/releasing.md ||
    fail "release guide does not record the persistent schema version"

for file in src/state.rs src/winit.rs src/input.rs src/render.rs src/udev.rs \
    src/handlers/*.rs src/grabs/*.rs; do
    grep -q 'SPDX-License-Identifier: MIT' "$file" ||
        fail "$file lacks its retained MIT SPDX identifier"
done

echo "release metadata: ok (version $version, Smithay $revision)"
