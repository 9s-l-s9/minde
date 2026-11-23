#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# guix shell -m manifest.scm python-pygobject gtk+@3 at-spi2-core dbus gobject-introspection -- sh tests/a11y-e2e.sh
set -eu
cd "$(dirname "$0")/.."
if [ "${1:-}" != --isolated ]; then
    bus_runtime=$(mktemp -d /tmp/minde-a11y-bus.XXXXXX)
    trap 'rm -rf "$bus_runtime"' EXIT
    unset DISPLAY WAYLAND_DISPLAY AT_SPI_BUS_ADDRESS
    export XDG_RUNTIME_DIR="$bus_runtime"
    dbus-run-session -- sh tests/a11y-e2e.sh --isolated
    exit $?
fi
. tests/lib/nested-compositor.sh
out=${MINDE_A11Y_E2E_OUT:-$(mktemp -d /tmp/minde-a11y-e2e.XXXXXX)}
fixture_pid=
cleanup() {
    if [ -n "$fixture_pid" ]; then
        kill "$fixture_pid" 2>/dev/null || true
        wait "$fixture_pid" 2>/dev/null || true
    fi
    nested_stop
}
trap cleanup EXIT
echo "Accessibility test logs: $out"
unset NO_AT_BRIDGE AT_SPI_BUS_ADDRESS
export GTK_A11Y=always GIO_USE_VFS=local
nested_start "$out" "${MINDE_A11Y_E2E_DISPLAY:-:86}" || exit 1
# An agent starts from the public catalog without knowing the tool's name.
scripts/mindectl search button --json >"$out/discovery.json"
tool=$(jq -er '.[] | select(.kind == "cli-tool") | .name' "$out/discovery.json")
scripts/mindectl help "$tool" --json | jq -e '.windows == "mindectl query windows --json"
    and .start == "mindectl a11y tree WINDOW-ID --depth 4"' >/dev/null
scripts/mindectl capabilities --json | jq -e --arg name "$tool" '.tools | any(.name == $name)' >/dev/null

for backend in wayland x11; do
    if [ "$backend" = x11 ]; then
        number=$(timeout 30 scripts/mindectl eval '(caddr (wm-runtime-info))')
        case "$number" in ''|*[!0-9]*) echo "XWayland unavailable: $number" >&2; exit 1;; esac
        display=:$number
    else
        display=
    fi
    title="Minde accessibility fixture — Übersicht ($backend)"
    result_dir=$(mktemp -d "$out/$backend.XXXXXX")
    env DISPLAY="$display" WAYLAND_DISPLAY="$NESTED_WAYLAND_DISPLAY" GDK_BACKEND="$backend" \
        python3 tests/clients/a11y-gtk3.py "$result_dir/submitted.json" "$title" >"$out/$backend.log" 2>&1 &
    fixture_pid=$!
    id=
    for _ in $(seq 1 50); do
        id=$(scripts/mindectl query windows --json | jq -r --arg title "$title" '.windows[]? | select(.title == $title) | .id')
        [ -n "$id" ] && break
        kill -0 "$fixture_pid"
        sleep 0.2
    done
    [ -n "$id" ]
    a11y() {
        operation=$1
        shift
        LC_ALL=C timeout 18 scripts/mindectl a11y "$operation" "$id" "$@"
    }
    ready=
    for _ in $(seq 1 30); do
        if a11y find --role push-button --name Finish >"$out/find.json" 2>"$out/find.err"; then
            ready=1
            break
        fi
        sleep 0.2
    done
    if [ -z "$ready" ]; then cat "$out/find.err" "$out/find.json" >&2; exit 1; fi
    jq -e '.ok and .result.role == "push-button" and .result.name == "Finish"' "$out/find.json"
    a11y tree --depth 4 | jq -e . >/dev/null
    if a11y click --role push-button --name Missing >"$out/missing.json" 2>"$out/missing.err"; then
        echo "Missing button unexpectedly accepted" >&2; exit 1
    fi
    jq -e '.error == "not-found"' "$out/missing.json"
    if a11y click --role push-button --name Duplicate >"$out/duplicate.json" 2>"$out/duplicate.err"; then
        echo "Ambiguous button unexpectedly accepted" >&2; exit 1
    fi
    jq -e '.error == "ambiguous"' "$out/duplicate.json"
    a11y select --role combo-box --name 'Model time units' --value hours
    a11y set-text --role text --name Capacity --value 3
    a11y focus --role push-button --name Finish
    a11y focus --role text --name Capacity
    a11y inspect --role text --name Capacity >"$out/field.json"
    jq -e '.ok and .result.text == "3" and (.result.states | index("editable") != null)' "$out/field.json"
    a11y inspect --name Password >"$out/password.json"
    jq -e '.ok and .result.text == null' "$out/password.json"
    a11y inspect --within-name Geräteeinstellungen --role push-button --name Duplicate >"$out/actions.json"
    action=$(jq -er '.result.actions[] | select(. == "click")' "$out/actions.json")
    a11y invoke --within-name Geräteeinstellungen --role push-button --name Duplicate --action "$action"
    a11y click --role push-button --name Finish
    for _ in $(seq 1 50); do
        [ -s "$result_dir/submitted.json" ] && break
        sleep 0.1
    done
    jq -e '. == ["hours", "3", ["Geräteeinstellungen"], true]' "$result_dir/submitted.json"
    wait "$fixture_pid"
    fixture_pid=
    echo "$backend GTK3 semantic form actions passed"
done
