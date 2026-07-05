#!/bin/sh
set -eu

links=$(mktemp)
trap 'rm -f "$links"' EXIT HUP INT TERM

find README.md doc -type f -name '*.md' -print |
while IFS= read -r document; do
    rg -o --no-filename '\]\([^)]+\)' "$document" 2>/dev/null |
        sed 's/^](//; s/)$//' >"$links" || :
    while IFS= read -r target; do
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
    done <"$links"
done

echo "documentation links: ok"
