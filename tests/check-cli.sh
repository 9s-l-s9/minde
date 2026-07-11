#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

binary=${1:-target/debug/minde}
[ -x "$binary" ] || {
    echo "CLI contract: executable not found: $binary" >&2
    exit 1
}

help=$("$binary" --help)
version=$("$binary" --version)

printf '%s\n' "$help" | grep -q '^Usage: minde \[OPTION\]$'
printf '%s\n' "$help" | grep -q -- '--tty'
printf '%s\n' "$help" | grep -q -- '--winit'
# Semver with an optional pre-release suffix ("1.0.0-rc1").
printf '%s\n' "$version" |
    grep -Eq '^minde [0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)? \([^()]+\)$'

set +e
error=$("$binary" --definitely-invalid 2>&1)
status=$?
set -e
[ "$status" -eq 2 ] || {
    echo "CLI contract: invalid option exited $status, expected 2" >&2
    exit 1
}
printf '%s\n' "$error" | grep -q 'unknown option: --definitely-invalid'
printf '%s\n' "$error" | grep -q "Try 'minde --help'"

echo "CLI contract: ok ($version)"
