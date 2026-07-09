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

for executable in minde minde-session minde-cmd minde-msg mindectl; do
    [ -x "$out/bin/$executable" ] || fail "missing executable bin/$executable"
done

for file in \
    share/wayland-sessions/minde.desktop \
    share/minde/default-config.scm \
    share/minde/scheme/init.scm \
    share/guile/site/3.0/minde/foundation/geometry.scm \
    share/guile/site/3.0/minde/ui/prompt.scm \
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

if grep -R -F "$(pwd)" "$out/bin/minde-session" "$out/share" >/dev/null; then
    fail "installed text refers to the development checkout"
fi

echo "package contents: complete install for minde $version"
