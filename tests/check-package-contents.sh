#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
set -eu

[ "$#" -eq 2 ] || {
    echo "usage: tests/check-package-contents.sh STORE-PATH VERSION" >&2
    exit 2
}
out=$1
version=$2

fail() {
    echo "package contents: $*" >&2
    exit 1
}

for executable in minde minde-session minde-cmd minde-msg mindectl minde-agent; do
    [ -x "$out/bin/$executable" ] || fail "missing executable bin/$executable"
done

for file in \
    share/wayland-sessions/minde.desktop \
    share/minde/default-config.scm \
    share/minde/scheme/init.scm \
    share/guile/site/3.0/minde/foundation/geometry.scm \
    share/guile/site/3.0/minde/ui/prompt.scm \
    share/guile/site/3.0/minde/control.scm \
    share/guile/site/3.0/minde/control-client.scm \
    share/guile/site/3.0/minde/control-json.scm \
    share/guile/site/3.0/minde/keysyms.scm \
    share/doc/minde/doc/agent-control.md \
    share/doc/minde/README.md \
    share/doc/minde/CHANGELOG.md \
    share/doc/minde/COPYING \
    share/doc/minde/LICENSES/GPL-3.0-or-later.txt \
    share/doc/minde/LICENSES/MIT.txt \
    share/doc/minde/doc/generated/manual.html; do
    [ -s "$out/$file" ] || fail "missing $file"
done

actual=$("$out/bin/minde" --version)
case $actual in
    "minde $version ("*")") ;;
    *) fail "unexpected --version output: $actual" ;;
esac

config_dir=$(mktemp -d)
trap 'rm -rf "$config_dir"' EXIT HUP INT TERM
cat >"$config_dir/keys.scm" <<'EOF'
(minde-config (version 1) (prefix () "Page_Up") (bindings ("M-C-g" windows)))
EOF
(
    # Installed validation must find its pinned library and modules without
    # inheriting a development shell or another installed session's paths.
    unset GUIX_ENVIRONMENT GUILE_LOAD_PATH GUILE_LOAD_COMPILED_PATH \
        LD_LIBRARY_PATH LTDL_LIBRARY_PATH MINDE_GUILE MINDE_SCHEME_DIR
    "$out/bin/mindectl" check-config "$config_dir/keys.scm"
    printf '(minde-config (version 1) (prefix () "Pritn") (bindings))\n' >"$config_dir/invalid.scm"
    if "$out/bin/mindectl" check-config "$config_dir/invalid.scm" >"$config_dir/error" 2>&1; then
        fail "standalone checker accepted an unknown XKB key"
    fi
    grep -q 'unknown XKB key name' "$config_dir/error" || fail "standalone checker could not validate keys"
)

if grep -R -F "$(pwd)" "$out/bin/minde-session" "$out/share" >/dev/null; then
    fail "installed text refers to the development checkout"
fi

echo "package contents: complete install for minde $version"
