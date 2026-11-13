#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Exercise actual Make recipes without building archives or querying hardware.
set -eu
cd "$(dirname "$0")/.."
if [ "${1:-all}" = all ]; then
    failed=0
    for scenario in archives hardware lint; do
        sh "$PWD/tests/make-gates-test.sh" "$scenario" || failed=1
    done
    exit "$failed"
fi
make_binary=$(command -v make)
shell_binary=$(command -v sh)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/minde-make-gates.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
trap 'exit 1' HUP INT TERM
cp Makefile "$scratch/"
# Every unrelated check is inert. ShellCheck still inspects real shell syntax,
# including deliberately invalid launcher fixtures below.
for file in check debug-tty.sh scripts/* tests/*.sh tests/lib/*.sh; do
    [ -f "$file" ] || continue
    case $file in *.scm) continue ;; esac
    mkdir -p "$scratch/$(dirname "$file")"
    printf '#!/bin/sh\nexit 0\n' >"$scratch/$file"
done
mkdir -p "$scratch/bin" "$scratch/no-shellcheck"
ln -s "$shell_binary" "$scratch/no-shellcheck/sh"
cat >"$scratch/bin/guix" <<'EOF'
#!/bin/sh
touch verifier-ran
[ "${GATE_CASE:-good}" != verifier-failure ]
EOF
chmod +x "$scratch/bin/guix"
cat >"$scratch/scripts/create-release-archives" <<'EOF'
#!/bin/sh
set -eu
printf '%s\n' "$2" >>builders-ran
mkdir -p "$2"
printf 'source\n' >"$2/minde-$1.tar.gz"
printf 'vendored\n' >"$2/minde-$1-vendored.tar.gz"
case ${GATE_CASE:-good}:$2 in
    first-build:*/one|second-build:*/two) exit 23 ;;
    source-difference:*/two) printf 'different\n' >"$2/minde-$1.tar.gz" ;;
    vendor-difference:*/two) printf 'different\n' >"$2/minde-$1-vendored.tar.gz" ;;
esac
EOF
cat >"$scratch/scripts/hardware-report" <<'EOF'
#!/bin/sh
printf '/fixture/report with spaces\n'
[ "${GATE_CASE:-good}" != hardware-failure ]
EOF
cd "$scratch"
PATH="$scratch/bin:$PATH"
export PATH
printf 'version = "1.2.3"\n' >Cargo.toml
fail() { echo "FAIL - $*" >&2; exit 1; }
run() { "$make_binary" --no-print-directory -s "$@" >stdout 2>stderr; }
case $1 in
archives)
    for GATE_CASE in first-build second-build source-difference vendor-difference verifier-failure; do
        export GATE_CASE
        rm -f builders-ran verifier-ran
        if run check-release-archives VERSION=1.2.3; then fail "archive gate ignored $GATE_CASE"; fi
        expected=2
        [ "$GATE_CASE" != first-build ] || expected=1
        [ "$(wc -l <builders-ran)" -eq "$expected" ] || fail 'continued after a failed archive build'
        if [ "$GATE_CASE" = verifier-failure ]; then
            [ -e verifier-ran ] || fail 'final verifier was not exercised'
        else
            [ ! -e verifier-ran ] || fail "ran final verifier after $GATE_CASE"
        fi
        echo "ok - archive gate rejects $GATE_CASE"
    done
    export GATE_CASE=good
    for version in 1.2.3 ''; do
        rm -f builders-ran verifier-ran
        run check-release-archives VERSION="$version" || { cat stderr >&2; fail 'identical archive builds'; }
        [ "$(wc -l <builders-ran)" -eq 2 ] || fail 'did not build twice'
        [ -e verifier-ran ] || fail 'successful comparisons skipped the verifier'
        echo "ok - identical archives pass (VERSION=$version)"
    done
    ;;
hardware)
    export GATE_CASE=hardware-failure
    if run check-hardware; then fail 'hardware gate ignored report failure'; fi
    if grep -q 'hardware snapshot:' stdout; then fail 'printed a success message after report failure'; fi
    echo 'ok - hardware report failure stops the recipe'
    export GATE_CASE=good
    run check-hardware || fail 'successful hardware report'
    grep -Fq 'hardware snapshot: /fixture/report with spaces' stdout || fail 'lost report path'
    echo 'ok - hardware report success preserves the path'
    ;;
lint)
    if PATH="$scratch/no-shellcheck" run check-static; then fail 'static gate accepted missing ShellCheck'; fi
    grep -iq 'shellcheck.*required' stderr || { cat stderr >&2; fail 'missing-tool diagnostic'; }
    echo 'ok - static gate requires ShellCheck'
    for launcher in scripts/run-nested scripts/minde-agent; do
        cat >"$launcher" <<'EOF'
#!/bin/sh
value=$1
printf '%s\n' $value
EOF
        if run check-static; then fail "static gate did not lint $launcher"; fi
        grep -Fq "$launcher" stdout || { cat stdout stderr >&2; fail "missing $launcher diagnostic"; }
        echo "ok - static gate detects an error in $launcher"
        printf '#!/bin/sh\nexit 0\n' >"$launcher"
    done
    run check-static || { cat stdout stderr >&2; fail 'valid shell fixtures'; }
    echo 'ok - valid shell fixtures pass the static gate'
    ;;
*) fail "unknown scenario: $1" ;;
esac
