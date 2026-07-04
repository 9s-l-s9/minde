#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

fail() {
    echo "release metadata: $*" >&2
    exit 1
}

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
[ -n "$version" ] || fail "Cargo.toml has no package version"

grep -q '^license = "GPL-3.0-or-later"$' Cargo.toml ||
    fail "Cargo license is not GPL-3.0-or-later"
grep -q "(version \"$version\")" guix.scm ||
    fail "Cargo and Guix versions differ"
grep -q "(version \"$version\")" guix/foundation.scm ||
    fail "Cargo and foundation package versions differ"
grep -q "(version \"$version\")" guix/ui.scm ||
    fail "Cargo and UI package versions differ"
grep -q '(license license:gpl3+)' guix.scm ||
    fail "Guix license is not GPL-3.0-or-later"
grep -q 'github.com/9s-l-s9/minde' Cargo.toml guix.scm ||
    fail "canonical repository is absent from package metadata"

for file in COPYING NOTICE CHANGELOG.md CONTRIBUTING.md SECURITY.md SUPPORT.md UNEXPECTED.md \
    LICENSES/GPL-3.0-or-later.txt LICENSES/MIT.txt; do
    [ -s "$file" ] || fail "$file is missing or empty"
done

revision=$(sed -n 's/^Pinned revision: //p' NOTICE)
[ ${#revision} -eq 40 ] || fail "NOTICE does not contain a full pinned revision"
grep -q "$revision" Cargo.toml || fail "Cargo Smithay revision differs from NOTICE"
grep -q "$revision" guix.scm || fail "Guix Smithay revision differs from NOTICE"

for file in src/state.rs src/winit.rs src/input.rs src/render.rs src/udev.rs \
    src/handlers/*.rs src/grabs/*.rs; do
    grep -q 'SPDX-License-Identifier: MIT' "$file" ||
        fail "$file lacks its retained MIT SPDX identifier"
done

echo "release metadata: ok (version $version, Smithay $revision)"
