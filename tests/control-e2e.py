#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Use only the documented client interface against the isolated compositor."""
import json
import pathlib
import statistics
import struct
import subprocess
import sys
import time

out = pathlib.Path(sys.argv[1])
measurements = []
selector_measurements = []


def cli(*args, success=True):
    started = time.monotonic()
    result = subprocess.run(
        ["scripts/mindectl", *args, "--json"],
        capture_output=True, text=True, timeout=8, check=False,
    )
    measurements.append({"command": args[0], "ms": (time.monotonic() - started) * 1000,
                         "response_bytes": len(result.stdout.encode())})
    if success:
        assert result.returncode == 0, (args, result.stdout, result.stderr)
    else:
        assert result.returncode != 0, (args, result.stdout, result.stderr)
    return json.loads(result.stdout)


def snapshot():
    return cli("query", "desktop")


def select_action(request, *args):
    started = time.monotonic()
    result = subprocess.run(["scripts/minde-agent", "select", request, *args],
                            capture_output=True, text=True, timeout=10, check=False)
    assert result.returncode == 0, (request, result.stdout, result.stderr)
    selector_measurements.append({"request": request,
                                  "ms": (time.monotonic() - started) * 1000})
    return json.loads(result.stdout)


def window(state, identity):
    return next(w for w in state["windows"] if w["id"] == identity)


def call(name, arguments, request=None, success=True):
    extra = []
    if request:
        extra = ["--session", request["session"], "--revision", str(request["revision"]),
                 "--request-id", request["request-id"]]
    return cli("call", name, arguments, *extra, success=success)


catalog = cli("capabilities")
names = [entry["name"] for entry in catalog["actions"]]
for operation in ("windows", "focus-window!", "move-window-to-group!",
                  "set-window-floating!", "set-window-fullscreen!", "split-frame!",
                  "apply-layout!", "close-window!", "screenshot"):
    assert operation in names, operation
    description = cli("help", operation)
    assert description["parameters"] is not False

state = snapshot()
assert len(state["windows"]) == 2, state
first = next(w["id"] for w in state["windows"] if w["app-id"] == "control-one")
second = next(w["id"] for w in state["windows"] if w["app-id"] == "control-two")
assert window(state, first)["visible"] is False
assert window(state, first)["title"] == window(state, second)["title"]

focus = call("focus-window!", f"((window . {first}))")
assert focus["status"] == "applied", focus
state = snapshot()
assert state["focused-window"] == first and state["focused-group"] == " I "

# The selector shares live discovery and the same guarded executor. Dry runs
# and ambiguous duplicate titles must leave the desktop unchanged.
dry_run = select_action("focus control-two")
assert dry_run["status"] == "dry-run" and dry_run["arguments"]["window"] == second
assert select_action("focus Same title")["reason"] == "ambiguous"
assert snapshot()["focused-window"] == first
selected = select_action("focus control-two", "--execute", "--allow", "focus-window!")
assert selected["status"] == "submitted" and selected["result"]["status"] == "applied", selected
assert snapshot()["focused-window"] == second
call("focus-window!", f"((window . {first}))")

request = cli("request")
floating = call("set-window-floating!", f"((window . {first}) (enabled . #t))", request)
assert floating["status"] == "applied", floating
assert window(snapshot(), first)["floating"] is True
again = call("set-window-floating!", f"((window . {first}) (enabled . #t))", request)
assert again == floating, (again, floating)
assert call("set-window-floating!", f"((window . {first}) (enabled . #t))")["status"] == "unchanged"
conflict = call("set-window-floating!", f"((window . {first}) (enabled . #f))", request, success=False)
assert conflict["error"]["code"] == "request-id-conflict"
assert window(snapshot(), first)["floating"] is True

request = cli("request")
call("switch-group!", '((group . " II "))')
stale = call("close-window!", f"((window . {first}))", request, success=False)
assert stale["error"]["code"] == "stale-state", stale
assert len(snapshot()["windows"]) == 2

call("focus-window!", f"((window . {first}))")
path = out / "window.png"
capture = call("screenshot", f'((path . "{path}") (window . {first}))')
assert capture["status"] == "pending", capture
finished = cli("wait", capture["request-id"], "--session", capture["session"], "--timeout-ms", "4000")
assert finished["status"] == "applied", finished
png = path.read_bytes()
assert png[:8] == b"\x89PNG\r\n\x1a\n"
width, height = struct.unpack(">II", png[16:24])
geometry = window(snapshot(), first)["geometry"]
assert (width, height) == tuple(geometry[2:]), ((width, height), geometry)

# A visible target validates successfully, but writing into a missing parent
# directory fails asynchronously and must remain observable through its receipt.
failed_capture = call("screenshot", f'((path . "{out}/missing-parent/capture.png") (window . {first}))')
assert failed_capture["status"] == "pending", failed_capture
failed = cli("wait", failed_capture["request-id"], "--session", failed_capture["session"],
             "--timeout-ms", "4000", success=False)
assert failed["error"]["code"] == "operation-failed", failed

before = snapshot()
call("move-window-to-group!", f'((window . {first}) (group . " II "))')
after = snapshot()
assert window(after, first)["group"] == " II " and after["focused-group"] == " I "
assert after["revision"] > before["revision"]
events = cli("events-since", str(before["event-sequence"]), "--session", before["session"])
assert not events["gap"] and events["sequence"] >= before["event-sequence"]

hidden_capture = call("screenshot", f'((path . "{out}/offscreen.png") (window . {first}))', success=False)
assert hidden_capture["error"]["code"] == "target-unavailable", hidden_capture

bad = call("close-window!", '((window . "wrong type"))', success=False)
assert bad["error"]["code"] == "invalid-argument"
assert len(snapshot()["windows"]) == 2

close = call("close-window!", f"((window . {second}))")
done = cli("wait", close["request-id"], "--session", close["session"], "--timeout-ms", "4000")
assert done["status"] == "applied", done
assert [w["id"] for w in snapshot()["windows"]] == [first]
unknown = call("focus-window!", f"((window . {second}))", success=False)
assert unknown["error"]["code"] == "unknown-window"

latencies = sorted(row["ms"] for row in measurements)
report = {"scenario": "isolated-native-wayland", "passed": True,
          "cli_calls": len(measurements), "p50_ms": statistics.median(latencies),
          "p95_ms": latencies[min(len(latencies) - 1, int(len(latencies) * .95))],
          "max_response_bytes": max(row["response_bytes"] for row in measurements),
          "deterministic_selector_calls": selector_measurements,
          "measurements": measurements}
(out / "results.json").write_text(json.dumps(report, indent=2) + "\n")
print(f"control-e2e: all checks passed; {len(measurements)} CLI calls, "
      f"p50={report['p50_ms']:.1f}ms p95={report['p95_ms']:.1f}ms")
