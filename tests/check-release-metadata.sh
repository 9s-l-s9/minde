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
    scripts/check-guix-package scripts/hardware-report scripts/release \
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
. release/contract.env

[ "$MINDE_RELEASE_VERSION" = "$version" ] ||
    fail "frozen release contract and Cargo versions differ"

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
grep -q "(version $MINDE_CONFIG_SCHEMA)" scheme/default-config.scm ||
    fail "portable configuration differs from the frozen contract"
grep -q "(version $MINDE_CONFIG_SCHEMA)" scheme/minde/config.scm ||
    fail "configuration validator differs from the frozen contract"
grep -q "status-schema-version $MINDE_STATUS_SCHEMA" scheme/minde/status.scm ||
    fail "status implementation differs from the frozen contract"
grep -q "minde-layouts $MINDE_PERSISTENT_SCHEMA" scheme/minde/layouts.scm ||
    fail "layout persistence differs from the frozen contract"
grep -q "minde-placement-rules $MINDE_PERSISTENT_SCHEMA" \
    scheme/minde/groups.scm ||
    fail "placement persistence differs from the frozen contract"
grep -q "report_schema_version=$MINDE_REPORT_SCHEMA" scripts/mindectl ||
    fail "diagnostic report differs from the frozen contract"

api_hash=$(sha256sum doc/generated/api-reference.md | cut -d ' ' -f 1)
[ "$api_hash" = "$MINDE_API_SHA256" ] ||
    fail "public API changed after the RC freeze"
keymap_hash=$(sha256sum doc/generated/keybindings.md | cut -d ' ' -f 1)
[ "$keymap_hash" = "$MINDE_KEYMAP_SHA256" ] ||
    fail "portable keymap changed after the RC freeze"
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
