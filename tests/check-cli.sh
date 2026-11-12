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

config_dir=$(mktemp -d)
trap 'rm -rf "$config_dir"' EXIT HUP INT TERM
# A developer's installed session may export a different Scheme bundle.
# This gate exercises the checkout's command schemas with the built binary.
MINDE_SCHEME_DIR=$(unset CDPATH; cd -- "$(dirname -- "$0")/../scheme" && pwd)
GUILE_AUTO_COMPILE=0
XDG_CACHE_HOME=$config_dir/cache
export MINDE_SCHEME_DIR GUILE_AUTO_COMPILE XDG_CACHE_HOME
cat >"$config_dir/control.scm" <<'EOF'
(minde-config (version 1) (prefix (ctrl) "t")
 (bindings ("d" desktop-snapshot) ("w" windows)))
EOF
"$binary" --check-config "$config_dir/control.scm"
cat >"$config_dir/arguments.scm" <<'EOF'
(minde-config (version 1) (prefix (ctrl) "t")
 (bindings ("f" focus-window!)))
EOF
if "$binary" --check-config "$config_dir/arguments.scm" >"$config_dir/error" 2>&1; then
    echo "CLI contract: accepted a key binding that requires arguments" >&2
    exit 1
fi
grep -q 'key binding command requires arguments' "$config_dir/error"

for key in Retrun Space esc c-x Ctrl-x C- NoSymbol; do
    printf '(minde-config (version 1) (prefix () "Print") (bindings ("%s" windows)))\n' "$key" \
        >"$config_dir/invalid-key.scm"
    if "$binary" --check-config "$config_dir/invalid-key.scm" >"$config_dir/error" 2>&1; then
        echo "CLI contract: accepted invalid binding key: $key" >&2
        exit 1
    fi
    grep -Eq 'XKB key name|unknown key modifier' "$config_dir/error"
done
cat >"$config_dir/duplicate.scm" <<'EOF'
(minde-config (version 1) (prefix (ctrl) "t")
 (bindings ("M-C-Page_Up" windows) ("C-M-Prior" desktop-snapshot)))
EOF
if "$binary" --check-config "$config_dir/duplicate.scm" >"$config_dir/error" 2>&1; then
    echo "CLI contract: accepted duplicate canonical bindings" >&2
    exit 1
fi
grep -q 'duplicate binding key' "$config_dir/error"
cat >"$config_dir/prefix.scm" <<'EOF'
(minde-config (version 1) (prefix () "Pritn") (bindings))
EOF
if "$binary" --check-config "$config_dir/prefix.scm" >"$config_dir/error" 2>&1; then
    echo "CLI contract: accepted invalid prefix key" >&2
    exit 1
fi
grep -q 'unknown XKB key name' "$config_dir/error"

for setting in '#f' '(ctrl alt) "BackSpace"' '(ctrl alt shift) "Escape"'; do
    printf '(minde-config (version 1) (prefix (ctrl) "t") (emergency-exit %s) (bindings))\n' \
        "$setting" >"$config_dir/emergency.scm"
    "$binary" --check-config "$config_dir/emergency.scm"
done
cat >"$config_dir/emergency.scm" <<'EOF'
(minde-config (version 1) (prefix (ctrl) "t")
 (emergency-exit (ctrl alt) "Backspace") (bindings))
EOF
if "$binary" --check-config "$config_dir/emergency.scm" >"$config_dir/error" 2>&1; then
    echo "CLI contract: accepted invalid emergency exit key" >&2
    exit 1
fi
grep -q 'unknown XKB key name' "$config_dir/error"

echo "CLI contract: ok ($version)"
