#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Inject real EGL call failures in a private nested compositor."""
import os
from pathlib import Path
import runpy
import struct
import subprocess
import sys
import time

api = runpy.run_path(str(Path(__file__).with_name("session-lock.py")))
Client, HostPixels, uints = (api[name] for name in ("Client", "HostPixels", "uints"))
fault = Path(os.environ["MINDE_TEST_EGL_FAULT"])
log = Path(os.environ["MINDE_TEST_COMPOSITOR_LOG"])


def wait_for(predicate, description):
    deadline = time.monotonic() + 5
    while not predicate():
        if time.monotonic() >= deadline:
            raise AssertionError(description)
        time.sleep(0.01)


def arm(stage, kind="temporary", repeat="once"):
    candidate = fault.with_suffix(".new")
    candidate.write_text(f"{stage} {kind} {repeat} {os.environ['MINDE_TEST_TARGET_PID']}\n")
    candidate.replace(fault)


def color_is(pixels, color):
    return all(pixel == color for pixel in pixels.samples())


def paint(client, color, size=None):
    callback = client.alloc()
    client.send(client.surface, 3, uints(callback))
    client.paint(client.surface, *(size or client.configured_size), color)
    return callback


def presented(client, callback, pixels, color):
    client.wait(lambda event: event[:2] == (callback, 0), "frame after retry")
    wait_for(lambda: color_is(pixels, color), f"retry did not present {color:#x}")


def output_command(*args):
    env = dict(os.environ, DISPLAY="", WAYLAND_DISPLAY=os.environ["NESTED_WAYLAND_DISPLAY"])
    result = subprocess.run(args, env=env, timeout=5, capture_output=True, text=True)
    assert result.returncode == 0, f"{args}: {result.stdout}{result.stderr}"


def main():
    pixels = HostPixels()
    desktop = Client("desktop for rendering failures")
    owner = None
    try:
        api["ipc"]("(wm-clear-message)")
        desktop.toplevel()
        desktop.map_on_configure = False
        wait_for(lambda: color_is(pixels, 0xFF00FF), "initial desktop not visible")
        if sys.argv[1] == "render":
            for stage, color in (("render", 0x004488), ("swap", 0x884400)):
                arm(stage)
                callback = paint(desktop, color)
                wait_for(lambda: not fault.exists(), f"{stage} failure was not injected")
                presented(desktop, callback, pixels, color)
                print(f"ok - desktop recovers from {stage} failure without another commit", flush=True)

            # Repeated failures must retain the old pixels and withhold frame
            # callbacks; removing the fault must wake recovery by itself.
            before = log.read_text().count("egl-fault: injected render")
            arm("render", repeat="hold")
            callback = paint(desktop, 0x228844)
            wait_for(lambda: log.read_text().count("egl-fault: injected render") >= before + 3,
                     "temporary render failures were not retried")
            desktop.roundtrip()
            assert not any(event[:2] == (callback, 0) for event in desktop.events)
            assert color_is(pixels, 0x884400), "failed frame changed the visible desktop"
            fault.unlink()
            presented(desktop, callback, pixels, 0x228844)
            print("ok - repeated failures withhold frame callbacks and retry automatically", flush=True)

            for stage in ("render", "swap"):
                for tool, off, on in (
                    ("wlopm", ("--off", "winit"), ("--on", "winit")),
                    ("wlr-randr", ("--output", "winit", "--off"), ("--output", "winit", "--on")),
                ):
                    arm(stage)
                    output_command(tool, *off)
                    wait_for(lambda: not fault.exists(), f"{tool} {stage} failure not injected")
                    wait_for(lambda: color_is(pixels, 0), f"{tool} did not recover a black frame")
                    output_command(tool, *on)
                    desktop.roundtrip()
                    presented(desktop, paint(desktop, 0x228844), pixels, 0x228844)
                    print(f"ok - {tool} blackout recovers from {stage} failure", flush=True)

            # A lock may be confirmed only after a successful black swap.
            for stage in ("render", "swap"):
                marker = f"egl-fault: injected {stage}"
                before = log.read_text().count(marker)
                arm(stage, repeat="hold")
                owner = Client("lock owner during rendering failures")
                lock = owner.request_lock()
                wait_for(lambda: log.read_text().count(marker) >= before + 3,
                         "lock blackout was not retried")
                owner.roundtrip()
                assert not any(event[:2] == (lock, 0) for event in owner.events), "failed frame confirmed lock"
                fault.unlink()
                owner.wait(lambda event: event[:2] == (lock, 0), "lock confirmation after retry")
                assert color_is(pixels, 0), "lock confirmed before blackout was visible"
                print(f"ok - lock confirmation waits through repeated {stage} failures", flush=True)
                if stage == "render":
                    owner.send(lock, 2)
                    owner.roundtrip()
                    owner.close()
                    owner = None
                    presented(desktop, paint(desktop, 0x228844), pixels, 0x228844)
            lock_surface = owner.lock_surface(lock)
            configure = next(event[2] for event in owner.events if event[:2] == (lock_surface, 0))
            _, width, height = struct.unpack_from("=III", configure)
            size = (width, height)
            wait_for(lambda: color_is(pixels, 0x004080), "initial lock surface not visible")
            for stage, color in (("render", 0x224466), ("swap", 0x446622)):
                arm(stage)
                callback = paint(owner, color, size)
                wait_for(lambda: not fault.exists(), f"lock {stage} failure not injected")
                presented(owner, callback, pixels, color)
                print(f"ok - lock surface recovers from {stage} failure", flush=True)

        # Context loss is terminal, including while locked. The shell driver
        # checks the compositor's actual exit status and absence of a panic.
        arm(sys.argv[1], "fatal")
        paint(owner or desktop, 0x123456, size if owner else None)
        wait_for(lambda: not fault.exists(), "context-loss failure was not injected")
        connection = (owner or desktop).sock
        while connection.recv(65536):
            pass
        print(f"ok - {sys.argv[1]} context loss disconnects clients cleanly", flush=True)
    finally:
        fault.unlink(missing_ok=True)
        if owner:
            owner.close()
        desktop.close()
        pixels.close()


if __name__ == "__main__":
    main()
