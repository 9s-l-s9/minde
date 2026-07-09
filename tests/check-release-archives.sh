#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

[ "$#" -eq 3 ] || {
    echo "usage: tests/check-release-archives.sh VERSION SOURCE VENDORED" >&2
    exit 2
}
version=$1
source_archive=$2
vendored_archive=$3
prefix=minde-$version

fail() {
    echo "release archive: $*" >&2
    exit 1
}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/minde-release-check.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
index=0
for archive in "$source_archive" "$vendored_archive"; do
    index=$((index + 1))
    listing=$tmp/archive-$index.list
    [ -s "$archive" ] || fail "missing archive: $archive"
    gzip -t "$archive" || fail "invalid gzip stream: $archive"
    tar -tzf "$archive" >"$listing"
    first=$(sed -n '1p' "$listing")
    [ "$first" = "$prefix/" ] || fail "$archive has unexpected root: $first"
    if grep -Eq "^$prefix/(\\.git|target|build|\\.claude|\\.agents|\\.codex|\\.local)(/|$)" "$listing"; then
        fail "$archive contains a workspace-only path"
    fi
done

if grep -q "^$prefix/vendor/" "$tmp/archive-1.list"; then
    fail "source archive unexpectedly contains vendor/"
fi
grep -q "^$prefix/vendor/smithay/Cargo.toml$" "$tmp/archive-2.list" ||
    fail "vendored archive lacks Smithay"

tar -xzf "$vendored_archive" -C "$tmp"
mkdir -p "$tmp/$prefix/.cargo"
cp "$tmp/$prefix/guix/cargo-config.toml" "$tmp/$prefix/.cargo/config.toml"
(
    cd "$tmp/$prefix"
    CARGO_HOME="$tmp/cargo-home" cargo metadata --locked --offline --no-deps \
        --format-version 1 >/dev/null
)

echo "release archives: structure and offline Cargo metadata passed"
