#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Unmap X11 grabs and kill only the private test compositor's Xwayland child."""
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import signal
import subprocess
import time


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


input_client = load("input-lifecycle")
lifecycle = load("window-lifecycle")
protocol = input_client.protocol
check, ipc, evaluate, host = protocol.check, protocol.ipc, input_client.evaluate, input_client.host
uints = protocol.uints


def until(predicate, message):
    deadline = time.monotonic() + protocol.TIMEOUT
    while True:
        result = predicate()
        if result:
            return result
        check(time.monotonic() < deadline, message)
        time.sleep(0.02)


def managed_id(title):
    result = ipc(f'(find (lambda (id) (equal? (window-title id) "{title}")) (all-window-ids))')
    match = re.fullmatch(r"\(ok (\d+)\)", result)
    return int(match[1]) if match else None


def spawn_display(out, label, expected):
    path = out / (label + ".display")
    command = "printf '%s' \"${DISPLAY-unset}\" > " + shlex.quote(str(path))
    evaluate("(wm-spawn " + json.dumps(command) + ")")
    until(path.exists, "spawned display probe did not run")
    until(lambda: path.read_text() == expected,
          f"{label}: child DISPLAY was {path.read_text()!r}, expected {expected!r}")


def start_drag(anchor, virtual, window_id, button):
    anchor.focus()
    start = len(anchor.events)
    host("keydown", "Super_L")
    anchor.key(125, 1, start)
    evaluate(f"(begin (wm-focus-window {window_id}) (wm-warp-pointer 200 180))")
    anchor.send(virtual, 2, uints(0, button, 1))
    anchor.roundtrip()
    host("keyup", "Super_L")


class X11Client:
    def __init__(self, executable, title):
        self.title = title
        self.path = executable.parent / (title + ".log")
        self.log = self.path.open("w")
        self.process = subprocess.Popen([str(executable), "normal", title],
            stdin=subprocess.PIPE, stdout=self.log, stderr=self.log,
            text=True, env=dict(os.environ, DISPLAY=os.environ["MINDE_X11_DISPLAY"]))
        self.window_id = until(lambda: managed_id(title), "X11 client never mapped")

    def command(self, command):
        marker = f"done {command}\n"
        before = self.path.read_text().count(marker)
        self.process.stdin.write(command + "\n")
        self.process.stdin.flush()
        until(lambda: self.path.read_text().count(marker) > before, f"X11 {command} did not finish")

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
        self.process.wait(timeout=protocol.TIMEOUT)
        self.process.stdin.close()
        self.log.close()


def main():
    executable = Path(os.environ["MINDE_X11_TEST_CLIENT"])
    out = executable.parent
    startup_probe = Path(os.environ["XDG_RUNTIME_DIR"]) / "startup-display"
    until(startup_probe.exists, "startup display probe did not run")
    check(startup_probe.read_text() == "unset", "autostart inherited the host DISPLAY")
    print("ok - autostart has no DISPLAY before Xwayland is ready", flush=True)
    wm_pid = int(os.environ["MINDE_TEST_COMPOSITOR_PID"])
    xdisplay = os.environ["MINDE_X11_DISPLAY"]
    host_window = subprocess.check_output(["xdotool", "search", "--sync", "--name", "Smithay"],
                                          text=True, timeout=protocol.TIMEOUT).splitlines()[0]
    host("windowfocus", host_window)
    os.environ["MINDE_SESSION_LOCK_TEST_WINDOW"] = host_window
    anchor, observer = input_client.Client("native lifecycle anchor"), lifecycle.Observer()
    clients = []
    pixels = protocol.HostPixels()
    try:
        anchor.mapped(700)
        virtual = anchor.virtual_pointer()
        target = X11Client(executable, "x11-lifecycle-target")
        clients.append(target)
        spawn_display(out, "ready", xdisplay)
        cases = os.environ.get("MINDE_XWAYLAND_CASES", "unmap loss").split()
        if "unmap" in cases:
            for button in (272, 273):
                current = target.window_id
                handle = observer.handle(target.title)
                evaluate(f"(begin (float-window! {current} '(100 100 300 250)) (sync-frames!) #t)")
                start_drag(anchor, virtual, current, button)
                target.command("unmap")
                until(lambda: managed_id(target.title) is None, "X11 unmap stayed registered")
                start = len(anchor.events)
                evaluate("(wm-warp-pointer 800 180)")
                anchor.wait(lambda e: e[:2] == (anchor.pointer, 0), "pointer after X11 unmap", start)
                observer.wait(lambda e: e[:2] == (handle, 6), "X11 foreign handle closed")
                evaluate(f"(wm-window-geometry {current})", "#f")
                evaluate("(length (all-window-ids))", "1")
                anchor.send(virtual, 2, uints(0, button, 0))
                anchor.roundtrip()
                target.command("map")
                target.window_id = until(lambda: managed_id(target.title), "X11 remap did not register")
                check(target.window_id > current, "X11 remap reused the old managed ID")
                check(len(observer.live_handles()) == 2, "X11 remap duplicated a foreign handle")
            print("ok - X11 unmap cancels move/resize grabs and remaps with a fresh ID", flush=True)

        if "loss" in cases:
            second = X11Client(executable, "x11-lifecycle-second")
            clients.append(second)
            handles = [observer.handle(c.title) for c in clients]
            # Keep a native window in the active tile so ordinary unmap
            # policy has a focused survivor when both X11 floats disappear.
            evaluate(f"(begin (unfloat-window! {anchor.window_id})"
                     f" (float-window! {target.window_id} '(100 100 300 250))"
                     f" (float-window! {second.window_id} '(100 450 300 250)) (sync-frames!) #t)")
            evaluate('(wm-set-clipboard "native clipboard survives")')
            start_drag(anchor, virtual, target.window_id, 272)
            pointer_start = len(anchor.events)
            target.command("menu")
            try:
                until(lambda: 0x00FF00 in pixels.samples(), "override-redirect menu was not rendered")
            except AssertionError:
                subprocess.run(["grim", str(out / "menu-failure.png")], check=True,
                    env=dict(os.environ, WAYLAND_DISPLAY=str(protocol.WAYLAND)), timeout=protocol.TIMEOUT)
                print("host pixel samples:", pixels.samples(), flush=True)
                raise
            # Verify the PID is a direct child of this private compositor and
            # serves the expected nested display before signalling it.
            children = Path(f"/proc/{wm_pid}/task/{wm_pid}/children").read_text().split()
            servers = []
            for child in children:
                proc = Path("/proc") / child
                if proc.joinpath("comm").read_text().strip() == "Xwayland":
                    args = proc.joinpath("cmdline").read_bytes().split(b"\0")
                    check(xdisplay.encode() in args, "Xwayland child serves an unexpected display")
                    servers.append(int(child))
            check(len(servers) == 1, "expected one Xwayland child of the private compositor")
            os.kill(servers[0], signal.SIGKILL)
            until(lambda: ipc('(cadr (wm-runtime-info))') == '(ok "failed")',
                  "dead Xwayland still reported ready")
            evaluate("(caddr (wm-runtime-info))", "-1")
            evaluate("(all-window-ids)", f"({anchor.window_id})")
            evaluate("(focused-window-id)", str(anchor.window_id))
            for handle in handles:
                observer.wait(lambda e, handle=handle: e[:2] == (handle, 6), "dead X11 handle closed")
            check(len(observer.live_handles()) == 1, "dead X11 handles remained advertised")
            until(lambda: 0x00FF00 not in pixels.samples(), "dead X11 menu remained rendered")
            evaluate("(wm-warp-pointer 800 180)")
            anchor.wait(lambda e: e[:2] == (anchor.pointer, 0), "pointer after Xwayland loss", pointer_start)
            anchor.send(virtual, 2, uints(0, 272, 0))
            anchor.roundtrip()
            spawn_display(out, "failed", "unset")
            clipboard = subprocess.check_output(["wl-paste", "--no-newline"],
                env=dict(os.environ, WAYLAND_DISPLAY=str(protocol.WAYLAND)),
                text=True, timeout=protocol.TIMEOUT)
            check(clipboard == "native clipboard survives", "Xwayland loss cleared the native clipboard")
            status_path = Path(os.environ["XDG_RUNTIME_DIR"]) / "minde/status.json"
            until(lambda: json.loads(status_path.read_text())["runtime"]["xwayland"] == "failed",
                  "status file did not publish Xwayland failure")
            status = json.loads(status_path.read_text())
            check(status["runtime"]["xwayland"] == "failed" and status["runtime"]["xdisplay"] is None,
                  "published runtime kept the dead display")
            anchor.focus()
            start = len(anchor.events)
            evaluate('(wm-send-string "a" 0)')
            anchor.key(30, 0, start)
            print("ok - Xwayland loss removes X11 windows, handles and menus; native input survives", flush=True)
            print("ok - failed Xwayland clears status and DISPLAY in newly spawned children", flush=True)
    finally:
        host("keyup", "Super_L")
        for client in clients:
            client.close()
        for client in (anchor, observer):
            client.close()
        pixels.close()


if __name__ == "__main__":
    main()
