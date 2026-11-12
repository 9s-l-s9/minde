#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Capture completion, output changes and lock races in a private compositor."""
import json
import math
import mmap
import os
from pathlib import Path
import runpy
import struct
import subprocess
import sys

api = runpy.run_path(str(Path(__file__).with_name("session-lock.py")))
Client, check, uints, ipc = (api[name] for name in ("Client", "check", "uints", "ipc"))


class Capture(Client):
    def __init__(self):
        super().__init__("capture lifecycle")
        self.output = self.bind("wl_output")
        self.sources = self.bind("ext_output_image_capture_source_manager_v1")
        self.ext = self.bind("ext_image_copy_capture_manager_v1")
        self.wlr = self.bind("zwlr_screencopy_manager_v1", 3)
        self.outputs = self.bind("zwlr_output_manager_v1", 4)
        self.locks = self.bind("ext_session_lock_manager_v1")
        self.shm = self.bind("wl_shm")
        self.roundtrip()
        self.head = struct.unpack("=I", self.wait(
            lambda e: e[:2] == (self.outputs, 0), "output head")[2])[0]
        self.maps = []
        self.session = self.new_session()

    def close(self):
        super().close()
        for view in self.maps:
            view.close()

    def new_session(self):
        source, session = self.alloc(), self.alloc()
        self.send(self.sources, 0, uints(source, self.output))
        self.send(self.ext, 0, uints(session, source, 0))
        self.wait(lambda e: e[:2] == (session, 4), "initial constraints")
        return session

    def size(self):
        return struct.unpack("=II", [e[2] for e in self.events
                                     if e[:2] == (self.session, 0)][-1])

    def capture_buffer(self, size):
        width, height = size
        pool, buffer = self.alloc(), self.alloc()
        length = width * height * 4
        fd = os.memfd_create("capture-lifecycle", os.MFD_CLOEXEC)
        try:
            os.ftruncate(fd, length + 64)
            view = mmap.mmap(fd, length + 64)
            view[:] = b"\xa5" * len(view)
            self.send(self.shm, 0, uints(pool, length + 64), fd=fd)
            self.send(pool, 0, uints(buffer, 0, width, height, width * 4, 1))
            self.send(pool, 1)
        finally:
            os.close(fd)
        self.maps.append(view)
        return buffer, view

    def prepare(self, kind, size=None):
        frame = self.alloc()
        if kind == "ext":
            self.send(self.session, 0, uints(frame))
            size = size or self.size()
        else:
            self.send(self.wlr, 0, uints(frame, 0, self.output))
            event = self.wait(lambda e: e[0] == frame and e[1] in (3, 6),
                              "wlr buffer constraints or failure")
            if event[1] == 3:
                return (kind, frame, None, None, None)
            data = self.wait(lambda e: e[:2] == (frame, 0), "wlr buffer size")[2]
            _, width, height, _ = struct.unpack("=IIII", data)
            size = (width, height)
        buffer, view = self.capture_buffer(size)
        if kind == "ext":
            self.send(frame, 1, uints(buffer))
            self.send(frame, 2, struct.pack("=iiii", 0, 0, *size))
            request = (frame, 3, b"")
        else:
            request = (frame, 0, uints(buffer))
        return kind, frame, request, view, size

    def result(self, capture, success, reason=None):
        kind, frame, _, view, size = capture
        ready, failed = (3, 4) if kind == "ext" else (2, 3)
        event = self.wait(lambda e: e[0] == frame and e[1] in (ready, failed),
                          f"{kind} {'ready' if success else 'failed'}")
        check(event[1] == (ready if success else failed), f"wrong capture result: {event}")
        if reason is not None and kind == "ext":
            check(event[2] == uints(reason), f"wrong failure reason: {event}")
        if view is not None:
            check(view[-64:] == b"\xa5" * 64, "capture overwrote the buffer's trailing guard")
            if success:
                width, height = size
                for x, y in ((width // 2, height // 2), (width - 2, height - 2)):
                    pixel = view[(y * width + x) * 4:(y * width + x) * 4 + 3]
                    check(pixel != b"\xa5" * 3, "capture left pixels uninitialized")
                if kind == "ext":
                    check(self.wait(lambda e: e[:2] == (frame, 0), "capture transform")[2]
                          == uints(0), "upright capture must report normal transform")
                    for _, _, data in [e for e in self.events if e[:2] == (frame, 1)]:
                        x, y, w, h = struct.unpack("=iiii", data)
                        check(0 <= x < x + w <= width and 0 <= y < y + h <= height,
                              "damage exceeds capture buffer")
            else:
                check(view[:] == b"\xa5" * len(view), "failed capture wrote pixel data")
        self.send(frame, 0 if kind == "ext" else 1)
        return frame, ready, failed

    def capture(self, kind, success=True, size=None):
        capture = self.prepare(kind, size)
        if capture[2] is not None:
            self.send_batch([capture[2]])
        return self.result(capture, success)

    def configuration(self, enabled=True, transform=None, scale=None):
        self.roundtrip()
        serial = [e[2] for e in self.events if e[:2] == (self.outputs, 1)][-1]
        config = self.alloc()
        self.send(self.outputs, 0, uints(config) + serial)
        if enabled:
            head = self.alloc()
            self.send(config, 0, uints(head, self.head))
            if transform is not None:
                self.send(head, 3, uints(transform))
            if scale is not None:
                self.send(head, 4, struct.pack("=i", round(scale * 256)))
        else:
            self.send(config, 1, uints(self.head))
        return config

    def applied(self, config):
        event = self.wait(lambda e: e[0] == config and e[1] in (0, 1, 2), "configuration result")
        check(event[1] == 0, f"configuration refused: {event}")
        self.send(config, 4)
        self.roundtrip()

    def configure(self, **kwargs):
        config = self.configuration(**kwargs)
        self.send(config, 2)
        self.applied(config)

    def assert_terminal_once(self, frames):
        self.roundtrip()
        for frame, ready, failed in frames:
            check(sum(e[0] == frame and e[1] in (ready, failed) for e in self.events) == 1,
                  "a completed capture received a second terminal event")


def file_capture(path, then=""):
    result = ipc(f'(let ((token (wm-screenshot {json.dumps(str(path))}))) {then} token)')
    token = int(result.removeprefix("(ok ").removesuffix(")"))
    check(ipc(f"(wm-automation-status {token})") == "(ok (screenshot failed))",
          "file capture did not fail promptly")
    check(not path.exists(), "failed file capture created a file")


def lock_case(c):
    captures = [c.prepare(kind) for kind in ("ext", "wlr")]
    lock = c.alloc()
    # One socket write: both captures are dispatched before lock, without a
    # compositor redraw between those requests.
    c.send_batch([item[2] for item in captures] + [(c.locks, 1, uints(lock))])
    frames = [c.result(item, False) for item in captures]
    c.wait(lambda e: e[:2] == (lock, 0), "lock confirmation")
    for _ in range(8):
        frames += [c.capture(kind, False) for kind in ("ext", "wlr")]
    path = Path(os.environ["XDG_RUNTIME_DIR"]) / "locked-capture.png"
    file_capture(path)
    c.send(lock, 2)
    c.roundtrip()
    for kind in ("ext", "wlr"):
        c.capture(kind)
    c.assert_terminal_once(frames)
    check(not path.exists(), "locked file capture ran after unlock")
    print("ok - lock fails queued and new captures; unlock starts fresh", flush=True)


def remove_case(c):
    captures = [c.prepare(kind) for kind in ("ext", "wlr")]
    config = c.configuration(enabled=False)
    c.send_batch([item[2] for item in captures] + [(config, 2, b"")])
    c.applied(config)
    frames = [c.result(item, False) for item in captures]
    c.wait(lambda e: e[:2] == (c.session, 5), "stopped capture session")
    c.configure()
    c.session = c.new_session()
    for kind in ("ext", "wlr"):
        c.capture(kind)
    c.assert_terminal_once(frames)
    path = Path(os.environ["XDG_RUNTIME_DIR"]) / "removed-capture.png"
    file_capture(path, '(configure-output! "winit" #:enabled #f)')
    c.configure()
    check(not path.exists(), "removed output capture ran after re-enable")
    c.session = c.new_session()
    print("ok - output removal fails pending captures and stops sessions", flush=True)


def geometry_case(c):
    original = c.size()
    check(original[0] != original[1], "rotation test requires a non-square output")
    for transform, scale in ((1, 1), (3, 1.25), (0, 1.5), (0, 1)):
        pending = c.prepare("ext")
        late_copy = c.prepare("wlr")
        config = c.configuration(transform=transform, scale=scale)
        start = len(c.events)
        c.send_batch([pending[2], (config, 2, b"")])
        c.applied(config)
        c.result(pending, False, reason=1)  # buffer_constraints
        c.send_batch([late_copy[2]])
        c.result(late_copy, False)
        c.wait(lambda e: e[:2] == (c.session, 4), "updated constraints", start=start)
        expected = original[::-1] if transform in (1, 3) else original
        expected = tuple(math.floor(math.ceil(n / scale) * scale + 0.5) for n in expected)
        check(c.size() == expected, f"wrong upright dimensions: {c.size()} vs {expected}")
        old = c.session
        c.session = c.new_session()
        check(c.size() == expected, "new session has wrong transformed dimensions")
        c.send(c.session, 1)
        c.session = old
        c.capture("ext")
        c.capture("wlr")
        if transform == 1:
            c.capture("ext", False, size=original)
    print("ok - rotation and scale refresh constraints and retire queued frames", flush=True)


def power_case(c):
    manager, power = c.bind("zwlr_output_power_manager_v1"), c.alloc()
    c.send(manager, 0, uints(power, c.output))
    c.wait(lambda e: e[:2] == (power, 0), "initial power mode")
    captures = [c.prepare(kind) for kind in ("ext", "wlr")]
    c.send_batch([item[2] for item in captures] + [(power, 0, uints(0))])
    frames = [c.result(item, False) for item in captures]
    for kind in ("ext", "wlr"):
        frames.append(c.capture(kind, False))
    file_capture(Path(os.environ["XDG_RUNTIME_DIR"]) / "powered-off-capture.png")
    c.send(power, 0, uints(1))
    c.roundtrip()
    for kind in ("ext", "wlr"):
        c.capture(kind)
    c.assert_terminal_once(frames)
    c.send(power, 1)
    print("ok - DPMS off fails captures without stopping a reusable session", flush=True)


def resize_case(c):
    env = dict(os.environ, DISPLAY=os.environ["MINDE_CAPTURE_HOST_DISPLAY"])
    window = subprocess.check_output(["xdotool", "search", "--name", "Smithay"],
                                     env=env, timeout=5, text=True).splitlines()[0]
    original = c.size()
    for size in ((960, 720), original):
        start = len(c.events)
        subprocess.run(["xdotool", "windowsize", window, *map(str, size)],
                       env=env, timeout=5, check=True)
        c.wait(lambda e: e[:2] == (c.session, 0) and e[2] == uints(*size),
               "constraints after host resize", start=start)
        c.wait(lambda e: e[:2] == (c.session, 4), "resize constraints done", start=start)
        c.capture("ext")
    print("ok - host mode changes refresh an existing capture session", flush=True)


if __name__ == "__main__":
    c = Capture()
    try:
        cases = {"lock": lock_case, "remove": remove_case, "geometry": geometry_case,
                 "power": power_case, "resize": resize_case}
        for name in sys.argv[1:] or cases:
            cases[name](c)
    finally:
        c.close()
