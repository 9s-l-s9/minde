#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Exercise the real Make recipe and Guile optimizer in an isolated tree.
set -eu
cd "$(dirname "$0")/.."
scratch=$(mktemp -d "${TMPDIR:-/tmp}/minde-scheme-cache.XXXXXX")
trap 'rm -rf "$scratch"' EXIT
trap 'exit 1' HUP INT TERM
mkdir -p "$scratch/project with spaces/scripts" "$scratch/project with spaces/scheme/cache-fixture"
cp Makefile "$scratch/project with spaces/"
cp scripts/mindectl "$scratch/project with spaces/scripts/"
if [ -f scripts/compile-scheme ]; then
    cp scripts/compile-scheme "$scratch/project with spaces/scripts/"
fi
cd "$scratch/project with spaces"
export GUILE_AUTO_COMPILE=0 XDG_CACHE_HOME="$scratch/user-cache"
fail() { echo "FAIL - $*" >&2; exit 1; }
build() {
    make --no-print-directory -s compile-scheme >build.log 2>&1 || {
        cat build.log >&2
        return 1
    }
}
provider() {
    printf '(define-module (cache-fixture %s) #:export (K f))\n(define K %s)\n(define (f x) (+ x %s))\n' "$1" "$2" "$2" >"scheme/cache-fixture/$1.scm"
}
consumer() {
    printf '(define-module (cache-fixture %s) #:use-module (cache-fixture %s) #:export (g))\n(define (g) (list K (f 10)))\n' "$1" "$2" >"scheme/cache-fixture/$1.scm"
}
expect() {
    result=$(GUILE_LOAD_COMPILED_PATH="$PWD/build/ccache" guile -q --no-auto-compile -L scheme -c '
      (set! %compile-fallback-path #f)
      (use-modules (cache-fixture z-consumer) (cache-fixture a-consumer))
      (write (list ((@ (cache-fixture z-consumer) g))
                   ((@ (cache-fixture a-consumer) g))))')
    [ "$result" = "(($1 $((10 + $1))) ($1 $((10 + $1))))" ] || fail "expected both consumers to use $1, got $result"
}
# Cover both lexical orders: only a previously compiled provider exposes
# its constants and procedure bodies to Guile's cross-module optimizer.
provider a-provider 1
provider z-provider 1
consumer z-consumer a-provider
consumer a-consumer z-provider
build || fail 'initial compilation'
expect 1
echo 'ok - initial cache works in both compilation orders'
unchanged=$(stat -c '%y' build/ccache/cache-fixture/z-consumer.go)
build || fail 'unchanged compilation'
[ "$(stat -c '%y' build/ccache/cache-fixture/z-consumer.go)" = "$unchanged" ] || fail 'unchanged tree was rebuilt'
echo 'ok - unchanged inputs reuse the cache'
touch scheme/cache-fixture/z-consumer.scm
build || fail 'timestamp-only change'
[ -z "$(find scheme/cache-fixture/z-consumer.scm -newer build/ccache/cache-fixture/z-consumer.go -print)" ] || fail 'bytecode predates its source'
expect 1
echo 'ok - timestamp-only changes keep bytecode fresh for Guile'
provider a-provider 2
provider z-provider 2
build || fail 'dependency change'
expect 2
echo 'ok - dependency changes update unchanged consumers'

# Even a clean rebuild must ignore installed bytecode and Guile's fallback
# cache. Preserve source timestamps so Guile would otherwise accept them.
cp -R build/ccache "$scratch/stale-cache"
export GUILE_LOAD_COMPILED_PATH="$scratch/stale-cache"
for name in a-provider z-provider; do
    src="scheme/cache-fixture/$name.scm"
    fallback=$(guile -q --no-auto-compile -c '(display (string-append %compile-fallback-path (canonicalize-path (cadr (command-line))) ".go"))' "$src")
    mkdir -p "$(dirname "$fallback")"
    cp "build/ccache/cache-fixture/$name.go" "$fallback"
    touch -r "$src" "$scratch/source-time"
    provider "$name" 3
    touch -r "$scratch/source-time" "$src"
done
build || fail 'preserved timestamp edit'
expect 3
echo 'ok - content changes invalidate caches even with preserved timestamps'
echo 'ok - inherited and fallback bytecode cannot contaminate compilation'
rm build/ccache/cache-fixture/z-consumer.go
build || fail 'missing bytecode'
expect 3
echo 'ok - missing output is rebuilt'
provider temporary 9
build || fail 'added source'
[ -f build/ccache/cache-fixture/temporary.go ] || fail 'new source was omitted'
mv scheme/cache-fixture/temporary.scm scheme/cache-fixture/renamed.scm
build || fail 'renamed source'
[ ! -e build/ccache/cache-fixture/temporary.go ] || fail 'renamed bytecode survived'
[ -f build/ccache/cache-fixture/renamed.go ] || fail 'renamed source was omitted'
rm scheme/cache-fixture/renamed.scm
build || fail 'removed source'
[ ! -e build/ccache/cache-fixture/renamed.go ] || fail 'deleted bytecode survived'
echo 'ok - source additions, renames and deletions update the cache'
complete=$(find build/ccache -type f -exec sha256sum {} + | LC_ALL=C sort)
provider a-provider 4
provider z-provider 4
printf '(\n' >scheme/cache-fixture/zz-broken.scm
if make --no-print-directory -s compile-scheme >build.log 2>&1; then fail 'invalid Scheme compiled successfully'; fi
[ "$(find build/ccache -type f -exec sha256sum {} + | LC_ALL=C sort)" = "$complete" ] || fail 'failed compilation replaced the complete cache'
rm scheme/cache-fixture/zz-broken.scm
build || fail 'retry after failed compilation'
expect 4
echo 'ok - compilation failure is fatal and a retry rebuilds dependents'

# The checkout's CLI must load this cache instead of interpreting its client
# modules on every call; an installed tree keeps its own bytecode.
mkdir -p scheme/minde
: >scheme/minde/control-client.scm
cat >"$scratch/guile-stub" <<'STUB'
#!/bin/sh
printf '%s\n' "$GUILE_LOAD_COMPILED_PATH"
STUB
chmod +x "$scratch/guile-stub"
compiled_path() {
    env -u GUILE_LOAD_COMPILED_PATH XDG_RUNTIME_DIR="$scratch" \
        MINDE_GUILE="$scratch/guile-stub" sh scripts/mindectl eval '#t'
}
[ "$(compiled_path)" = "$PWD/scripts/../build/ccache" ] ||
    fail "checkout CLI does not load the development cache: $(compiled_path)"
mkdir -p lib/guile/3.0/site-ccache
[ "$(compiled_path)" = "$PWD/scripts/../lib/guile/3.0/site-ccache" ] ||
    fail "installed bytecode does not take precedence: $(compiled_path)"
echo 'ok - the command-line client loads compiled modules in a checkout'
