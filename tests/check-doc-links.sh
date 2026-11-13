#!/bin/sh
set -eu

command -v rg >/dev/null 2>&1 || {
    echo "error: rg is required to validate documentation links" >&2
    exit 127
}

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
trap 'exit 1' HUP INT TERM

find README.md doc -type f -name '*.md' -print >"$scratch/documents"
while IFS= read -r document; do
    # Status 1 means no matches; any other failure invalidates the check,
    # including errors after rg has already emitted some links.
    rg --no-config -o --no-filename '\]\([^)]+\)' "$document" >"$scratch/links" || {
        status=$?
        [ "$status" -eq 1 ] || exit "$status"
    }
    while IFS= read -r target; do
        target=${target#']('}
        target=${target%')'}
        case "$target" in
            http://*|https://*|mailto:*|'#'*) continue ;;
        esac
        target=${target%%#*}
        [ -n "$target" ] || continue
        case "$target" in
            /*) resolved=$target ;;
            *) resolved=$(dirname "$document")/$target ;;
        esac
        [ -e "$resolved" ] || {
            echo "error: $document links to missing $target" >&2
            exit 1
        }
    done <"$scratch/links"
done <"$scratch/documents"

echo "documentation links: ok"
