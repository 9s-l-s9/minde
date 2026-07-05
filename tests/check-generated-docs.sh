#!/bin/sh
set -eu

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

scripts/generate-docs "$tmp"

for document in api-reference.md keybindings.md demo-manifest.json manual.html; do
    if ! diff -u "doc/generated/$document" "$tmp/$document"; then
        echo "error: generated documentation is stale; run make docs" >&2
        exit 1
    fi
done

echo "generated documentation: current"
