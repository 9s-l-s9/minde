#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Capture timestamps must be in the client's system monotonic clock domain."""
import runpy
from pathlib import Path
import struct
import time

protocol = runpy.run_path(str(Path(__file__).with_name("session-lock.py")))
check, uints = protocol["check"], protocol["uints"]
client = protocol["Client"]("capture clock")
try:
    output = client.bind("wl_output")
    sources = client.bind("ext_output_image_capture_source_manager_v1")
    source = client.alloc()
    client.send(sources, 0, uints(source, output))
    manager = client.bind("ext_image_copy_capture_manager_v1")
    session = client.alloc()
    client.send(manager, 0, uints(session, source, 0))
    client.wait(lambda e: e[:2] == (session, 4), "capture constraints")
    size = client.wait(lambda e: e[:2] == (session, 0), "buffer size")[2]
    width, height = struct.unpack("=II", size)
    client.wait(lambda e: e[:2] == (session, 1) and e[2] == uints(1), "XRGB8888 support")
    buffer = client.buffer_for(width, height, 0)
    previous = 0
    for _ in range(2):
        frame = client.alloc()
        client.send(session, 0, uints(frame))
        client.send(frame, 1, uints(buffer))
        client.send(frame, 2, struct.pack("=iiii", 0, 0, width, height))
        before = time.monotonic_ns()
        client.send(frame, 3)  # capture
        event = client.wait(lambda e: e[0] == frame and e[1] in (3, 4), "ready or failed")
        after = time.monotonic_ns()
        check(event[1] == 3, f"capture failed: {event}")
        hi, lo, ns = struct.unpack("=III", client.wait(lambda e: e[:2] == (frame, 2), "presentation time")[2])
        timestamp = ((hi << 32) | lo) * 1_000_000_000 + ns
        check(ns < 1_000_000_000 and before <= timestamp <= after,
              f"capture timestamp {timestamp} outside system monotonic interval [{before}, {after}]")
        check(timestamp > previous, "capture time did not advance")
        previous = timestamp
        client.send(frame, 0)  # destroy
    print("ok - capture timestamps match system monotonic time and advance", flush=True)
finally:
    client.close()
