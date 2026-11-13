#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Run the actual documentation/demo validators against isolated artifacts.
set -eu
cd "$(dirname "$0")/.."
root=$PWD
if [ "${1:-all}" = all ]; then
    failed=0
    for scenario in links demos generated; do
        sh "$root/tests/document-gates-test.sh" "$scenario" || failed=1
    done
    exit "$failed"
fi
shell_binary=$(command -v sh)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/minde-document-gates.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$scratch/bin" "$scratch/doc/generated" "$scratch/artifacts"
cd "$scratch"
fail() { echo "FAIL - $*" >&2; exit 1; }
run() { "$shell_binary" "$root/tests/check-$1.sh" >stdout 2>stderr; }
reject() {
    status=0
    run "$1" || status=$?
    [ "$status" -eq "$2" ] || { cat stdout stderr >&2; fail "$1 returned $status, expected $2"; }
    [ ! -s stdout ] || fail "$1 printed success after failure"
}
case $1 in
generated)
    # Exercise the actual generation driver, testing-reference generator and
    # comparison gate. Other renderers have fixed output to keep this fixture
    # small; the command inventory and file comparisons remain real.
    mkdir scripts
    cp "$root/scripts/generate-docs" "$root/scripts/generate-testing-reference" scripts/
    cp "$root/check" check
    cat >bin/guile <<'EOF'
#!/bin/sh
printf 'rendered fixture\n'
EOF
    cat >scripts/generate-packaging-reference <<'EOF'
#!/bin/sh
printf 'packaging fixture\n' >"$1"
EOF
    chmod +x bin/guile
    PATH="$scratch/bin:$PATH"
    export PATH
    printf '.PHONY: check\n# Initial gate.\ncheck:\n' >Makefile
    scripts/generate-docs
    [ -s doc/testing.md ] || fail 'default generation omitted doc/testing.md'
    [ ! -e doc/generated/testing.md ] || fail 'default generation duplicated the testing reference'
    cp doc/testing.md artifacts/current.md
    run generated-docs || { cat stdout stderr >&2; fail 'fresh generated documents rejected'; }
    cmp doc/testing.md artifacts/current.md || fail 'checking fresh documents changed the source'
    echo 'ok - default generation and fresh comparison preserve document locations'

    printf '\nSTALE LINE\n' >>doc/testing.md
    cp doc/testing.md artifacts/stale.md
    if run generated-docs; then fail 'stale testing reference passed comparison'; fi
    grep -q 'doc/testing.md is stale' stderr || fail 'missing stale testing-reference diagnostic'
    cmp doc/testing.md artifacts/stale.md || fail 'checking stale documents rewrote the source'
    echo 'ok - stale testing reference fails and retains its original bytes'

    cp artifacts/current.md doc/testing.md
    printf '\n.PHONY: new-gate\n# New gate.\nnew-gate:\n' >>Makefile
    if run generated-docs; then fail 'changed command inventory passed comparison'; fi
    grep -q 'doc/testing.md is stale' stderr || fail 'missing changed-inventory diagnostic'
    cmp doc/testing.md artifacts/current.md || fail 'checking changed inventory rewrote the source'
    echo 'ok - changed command inventory fails without repairing the source'

    scripts/generate-docs "$scratch/output with spaces"
    [ -s 'output with spaces/testing.md' ] || fail 'explicit output directory lacks testing reference'
    grep -q 'new-gate' 'output with spaces/testing.md' || fail 'explicit output lacks current inventory'
    cmp doc/testing.md artifacts/current.md || fail 'explicit output generation changed the source'
    for artifact in doc/generated/*; do
        cmp "$artifact" "output with spaces/${artifact##*/}" || fail 'explicit output omitted an artifact'
    done
    scripts/generate-docs
    run generated-docs || { cat stdout stderr >&2; fail 'regeneration did not repair the reference'; }
    echo 'ok - explicit output stays isolated and default regeneration repairs drift'
    ;;
links)
    # A restricted PATH exercises missing-tool behavior even when the host
    # has ripgrep installed. All other validator commands remain real.
    for tool in sh mktemp find rm dirname sed cat; do
        ln -s "$(command -v "$tool")" "bin/$tool"
    done
    printf '[broken](doc/missing.md)\n' >README.md
    printf 'No links.\n' >doc/page.md
    PATH="$scratch/bin" reject doc-links 127
    grep -q 'rg.*required' stderr || fail 'missing ripgrep diagnostic'
    echo 'ok - missing ripgrep fails before declaring documents valid'
    reject doc-links 1
    grep -q 'README.md links to missing doc/missing.md' stderr || fail 'missing target diagnostic'
    echo 'ok - missing local link target fails'
    printf '[page](doc/page.md#section) [web](https://example.invalid) [mail](mailto:example@example.invalid) [anchor](#section)\n' >README.md
    printf '[parent](../README.md)\n' >doc/page.md
    run doc-links || { cat stderr >&2; fail 'valid local and external links'; }
    printf 'No links.\n' >doc/page.md
    run doc-links || fail 'link-free document'
    echo 'ok - valid links and documents without links pass'
    cat >bin/rg <<'EOF'
#!/bin/sh
printf '](doc/page.md)\n'
echo 'injected extraction error' >&2
exit 2
EOF
    chmod +x bin/rg
    PATH="$scratch/bin:$PATH" reject doc-links 2
    grep -q 'injected extraction error' stderr || fail 'extraction diagnostic was hidden'
    echo 'ok - extraction failure remains fatal after partial output'
    rm bin/rg
    printf 'No links.\n' >README.md
    rm -r doc
    reject doc-links 1
    echo 'ok - failed document discovery cannot produce a success report'
    ;;
demos)
    # ffprobe supplies one controlled duration; jq supplies the known
    # scenario ID. The validator still checks real files and manifest bytes.
    cat >bin/jq <<'EOF'
#!/bin/sh
[ "$1" != -r ] || { printf 'example\n'; exit "${JQ_STATUS:-0}"; }
EOF
    cat >bin/ffprobe <<'EOF'
#!/bin/sh
printf '%s\n' "$PROBE_DURATION"
exit "${PROBE_STATUS:-0}"
EOF
    chmod +x bin/jq bin/ffprobe
    PATH="$scratch/bin:$PATH"
    export PATH MINDE_DEMO_OUT="$scratch/artifacts"
    printf '{"schema_version":1,"scenarios":[{"demo_id":"example"}]}\n' >doc/generated/demo-manifest.json
    cp doc/generated/demo-manifest.json artifacts/manifest.json
    for extension in webm png txt; do printf 'fixture\n' >"artifacts/example.$extension"; done
    for PROBE_DURATION in N/A abc '' 0 0.000000 -1 NaN Inf 1abc 1e2 '+1' ' 1' '1 ' '1
2'; do
        export PROBE_DURATION
        reject demos 1
        grep -q 'example.webm has no positive duration' stderr || fail 'invalid duration diagnostic'
        printf 'ok - rejects duration [%s]\n' "$PROBE_DURATION"
    done
    for PROBE_DURATION in 1 2.500000 0.000001; do
        export PROBE_DURATION
        run demos || { cat stderr >&2; fail "valid duration $PROBE_DURATION"; }
        grep -q 'demo artifacts:.*valid' stdout || fail 'missing success report'
        printf 'ok - accepts duration [%s]\n' "$PROBE_DURATION"
    done
    PROBE_STATUS=23 reject demos 23
    echo 'ok - failed probe is fatal even with a valid-looking duration'
    JQ_STATUS=2 reject demos 2
    echo 'ok - failed manifest extraction is fatal even with a valid-looking ID'
    ;;
*) fail "unknown scenario: $1" ;;
esac
