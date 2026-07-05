#!/bin/sh
set -eu

out=${MINDE_DEMO_OUT:-build/demos}
manifest="$out/manifest.json"

command -v jq >/dev/null 2>&1 || {
    echo "error: jq is required to validate demo artifacts" >&2
    exit 127
}
command -v ffprobe >/dev/null 2>&1 || {
    echo "error: ffprobe is required to validate demo videos" >&2
    exit 127
}

[ -s "$manifest" ] || {
    echo "error: missing demo manifest; run make demos" >&2
    exit 1
}
cmp -s doc/generated/demo-manifest.json "$manifest" || {
    echo "error: captured demo manifest is stale; rerun make demos" >&2
    exit 1
}

jq -e '.schema_version == 1 and (.scenarios | length > 0)' "$manifest" \
    >/dev/null

jq -r '.scenarios[].demo_id' "$manifest" |
while IFS= read -r id; do
    for extension in webm png txt; do
        [ -s "$out/$id.$extension" ] || {
            echo "error: missing demo artifact $out/$id.$extension" >&2
            exit 1
        }
    done
    duration=$(ffprobe -v error -show_entries format=duration \
        -of default=noprint_wrappers=1:nokey=1 "$out/$id.webm")
    awk -v duration="$duration" 'BEGIN { exit !(duration > 0) }' || {
        echo "error: $id.webm has no positive duration" >&2
        exit 1
    }
done

echo "demo artifacts: manifest, videos, posters, and transcripts valid"
