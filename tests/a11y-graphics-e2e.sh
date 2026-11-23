#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Real desktop smoke test. Run with GIMP 3.2 and Inkscape 1.4 on PATH:
# guix shell -m manifest.scm python-pygobject gtk+@3 at-spi2-core dbus gobject-introspection gimp inkscape -- sh tests/a11y-graphics-e2e.sh
set -eu
cd "$(dirname "$0")/.."
if [ "${1:-}" != --isolated ]; then
    runtime=$(mktemp -d)
    # Invoked by the EXIT trap below.
    # shellcheck disable=SC2317
    remove_bus_runtime() {
        # GVfs may still be releasing its private FUSE mount after D-Bus exits.
        for _ in $(seq 1 50); do
            if ! mountpoint -q "$runtime/gvfs" && rm -rf --one-file-system "$runtime"; then
                return 0
            fi
            sleep 0.1
        done
        echo "Private runtime still busy: $runtime" >&2
        return 1
    }
    trap remove_bus_runtime EXIT
    unset DISPLAY WAYLAND_DISPLAY AT_SPI_BUS_ADDRESS
    export XDG_RUNTIME_DIR="$runtime"
    dbus-run-session -- sh tests/a11y-graphics-e2e.sh --isolated
    exit $?
fi

. tests/lib/nested-compositor.sh
nested_output_directory "${MINDE_A11Y_E2E_OUT:-}" a11y-graphics
gimp_pid=
inkscape_pid=
cleanup() {
    for pid in "$gimp_pid" "$inkscape_pid"; do
        [ -z "$pid" ] || kill "$pid" 2>/dev/null || true
    done
    for pid in "$gimp_pid" "$inkscape_pid"; do
        [ -z "$pid" ] || wait "$pid" 2>/dev/null || true
    done
    nested_stop
}
nested_install_cleanup cleanup
unset NO_AT_BRIDGE AT_SPI_BUS_ADDRESS
export LC_ALL=C.UTF-8 GTK_A11Y=always GIO_USE_VFS=local
nested_start "$OUT" "${MINDE_A11Y_E2E_DISPLAY:-:88}"
# HOME and XDG config/cache/state directories are private to nested clients.
export XDG_DATA_HOME="$NESTED_RT/data"
mkdir -p "$XDG_DATA_HOME"
printf 'P3\n2 1\n255\n255 0 0 0 255 0\n' >"$OUT/test.ppm"
printf '<svg xmlns="http://www.w3.org/2000/svg" width="30" height="40"/>' >"$OUT/test.svg"
nested_wayland_exec gimp --new-instance --no-splash --no-fonts "$OUT/test.ppm" >"$OUT/gimp.log" 2>&1 &
gimp_pid=$!
nested_wayland_exec inkscape "$OUT/test.svg" >"$OUT/inkscape.log" 2>&1 &
inkscape_pid=$!

# A new GIMP profile opens a welcome dialog. Match the role and title separately
# from the main image window so that the test detects a wrongly closed window.
windows() { scripts/mindectl query desktop --json; }
wait_window() {
    for _ in $(seq 1 120); do
        id=$(windows | jq -r --arg pattern "$1" '[.windows[]? | select(.title | test($pattern)) | .id] | if length == 1 then .[0] else empty end')
        if [ -n "$id" ]; then printf '%s\n' "$id"; return 0; fi
        sleep 0.5
    done
    return 1
}
ink=$(wait_window 'test\.svg.*Inkscape')
# Registration on the accessibility bus can lag behind the mapped window.
ready() {
    for _ in $(seq 1 30); do
        if scripts/mindectl a11y tree "$1" --depth 0 | jq -e '.ok' >/dev/null; then return 0; fi
        sleep 0.5
    done
    return 1
}
ready "$ink"
ink_zoom() {
    operation=$1
    shift
    scripts/mindectl a11y "$operation" "$ink" --depth 4 --role check-box --name 'Zoom with window size' "$@"
}
ink_zoom inspect >"$OUT/before.json"
action=$(jq -er '.result.actions[] | select(. == "click")' "$OUT/before.json")
ink_zoom invoke --action "$action" | jq -e '.ok'
ink_zoom inspect >"$OUT/after.json"
jq -es 'all(.[]; .ok) and ((.[0].result.states | index("checked") != null) != (.[1].result.states | index("checked") != null))' \
    "$OUT/before.json" "$OUT/after.json" >/dev/null

welcome=$(wait_window '^Welcome to GIMP')
frame=$(wait_window 'test.*GIMP$')
ready "$welcome"
scripts/mindectl a11y inspect "$welcome" --role push-button --name Close | jq -e '.result.actions | index("click") != null'
scripts/mindectl a11y invoke "$welcome" --role push-button --name Close --action click | jq -e '.ok'
for _ in $(seq 1 20); do
    windows >"$OUT/windows.json"
    if jq -e --argjson id "$welcome" '[.windows[]?.id] | index($id) == null' "$OUT/windows.json" >/dev/null; then break; fi
    sleep 0.2
done
jq -e --argjson id "$welcome" --argjson frame "$frame" \
    '[.windows[]?.id] | index($id) == null and index($frame) != null' "$OUT/windows.json"
echo "Inkscape checkbox changed; GIMP welcome dialog closed and image remained open."
