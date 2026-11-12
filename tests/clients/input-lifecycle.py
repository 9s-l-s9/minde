#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Held input must be released on cancellation, client death and session lock.

Run through session-lock-e2e.sh, in its private Xvfb/compositor instance.
Reuse the raw Wayland transport and lock client instead of duplicating them.
"""
import importlib.util
import os
from pathlib import Path
import re
import struct
import subprocess
import time

spec = importlib.util.spec_from_file_location("lock_client", Path(__file__).with_name("session-lock.py"))
protocol = importlib.util.module_from_spec(spec)
spec.loader.exec_module(protocol)
check, ipc, uints = protocol.check, protocol.ipc, protocol.uints


def evaluate(expression, expected="#t"):
    result = ipc(expression)
    check(result == f"(ok {expected})", f"{expression}: {result}")


def host(*args):
    subprocess.run(["xdotool", *args], check=True, timeout=protocol.TIMEOUT)


class Client(protocol.Client):
    def __init__(self, name):
        super().__init__(name)
        seat = self.bind("wl_seat", 5)
        self.pointer = self.alloc()
        self.send(seat, 0, uints(self.pointer))

    def mapped(self, x):
        self.toplevel()
        result = ipc(f'(find (lambda (id) (equal? (window-title id) "{self.name}")) (all-window-ids))')
        match = re.fullmatch(r"\(ok (\d+)\)", result)
        check(match, f"window ID: {result}")
        self.window_id = int(match[1])
        evaluate(f"(begin (float-window! {self.window_id} (list {x} 100 300 250))"
                 f" (sync-frames!) (rust-call 'wm-focus-window {self.window_id}))")
        self.wait(lambda e: e[:2] == (self.keyboard, 1), "keyboard enter")
        self.x = x + 100
        self.roundtrip()

    def focus(self):
        evaluate(f"(rust-call 'wm-focus-window {self.window_id})")
        self.roundtrip()

    def point(self):
        evaluate(f"(rust-call 'wm-warp-pointer {self.x} 180)")
        self.roundtrip()
        focus = [e for e in self.events if e[0] == self.pointer and e[1] in (0, 1)]
        check(focus and focus[-1][1] == 0
              and struct.unpack_from("=II", focus[-1][2])[1] == self.surface,
              f"{self.name}: pointer focus stuck after releasing grab")

    def key(self, code, pressed, start):
        self.wait(lambda e: e[:2] == (self.keyboard, 3)
                  and struct.unpack_from("=IIII", e[2])[2:] == (code, pressed),
                  f"key {code} state {pressed}", start)

    def modifiers(self):
        self.roundtrip()
        events = self.keyboard_events(opcodes=(4,))
        check(events, "no keyboard modifier state")
        return struct.unpack("=IIIII", events[-1][2])[1:4]

    def buttons(self, start=0):
        return [struct.unpack("=IIII", data)[2:]
                for obj, opcode, data in self.events[start:]
                if (obj, opcode) == (self.pointer, 3)]

    def button(self, code, pressed, start):
        self.wait(lambda e: e[:2] == (self.pointer, 3)
                  and struct.unpack("=IIII", e[2])[2:] == (code, pressed),
                  f"button {code} state {pressed}", start)

    def virtual_pointer(self):
        manager = self.bind("zwlr_virtual_pointer_manager_v1")
        pointer = self.alloc()
        self.send(manager, 0, uints(0, pointer))
        self.roundtrip()
        return pointer


def typing(first, second, client):
    # A send starts synchronously; change focus in the SAME IPC evaluation,
    # before its 8 ms key-up timer can run. This is deterministic cancellation.
    for command, released in [('wm-send-string "AAAA" 0', (30, 42)),
                              ('wm-send-key 4 "v"', (29,)),
                              ('wm-send-string "🦀" 0', (47, 29))]:
        first.focus()
        second.roundtrip()
        start = len(second.events)
        evaluate(f"(begin ({command}) (rust-call 'wm-focus-window {second.window_id}) #t)")
        for code in released:
            second.key(code, 0, start)
        check(all(struct.unpack_from("=IIII", event[2])[3] == 0
                  for event in second.keyboard_events(start, (3,))),
              "cancelled typing delivered a new press to the next window")
        check(second.modifiers() == (0, 0, 0), f"modifier remained held after {command}")
        start = len(second.events)
        evaluate('(wm-send-string "xyz" 0)')
        second.key(44, 0, start)  # KEY_Z, after x/y/z have drained.
        check(second.modifiers() == (0, 0, 0), "lowercase typing inherited modifiers")
    print("ok - focus cancellation releases Shift, Ctrl and clipboard-fallback keys", flush=True)

    first.focus()
    start = len(first.events)
    host("keydown", "Shift_L")
    first.key(42, 1, start)
    second.roundtrip()
    start = len(second.events)
    evaluate(f'(begin (wm-send-string "A" 0) (rust-call \'wm-focus-window {second.window_id}) #t)')
    second.key(30, 0, start)
    check(second.modifiers()[0] & 1, "cancellation released a physically held Shift")
    host("keyup", "Shift_L")
    second.key(42, 0, start)
    check(second.modifiers() == (0, 0, 0), "physical Shift did not release normally")
    print("ok - synthetic typing borrows an already held physical modifier", flush=True)


def pointer(first, second, client):
    for disconnect in (False, True):
        second.point()
        first.point()
        producer = client("virtual pointer producer")
        virtual = producer.virtual_pointer()
        start = len(first.events)
        producer.send(virtual, 2, uints(0, 272, 1))
        first.button(272, 1, start)
        if disconnect:
            producer.close()
        else:
            producer.send(virtual, 8)
            producer.roundtrip()
        first.button(272, 0, start)
        second.point()
    print("ok - virtual pointer destroy and disconnect release buttons and implicit grabs", flush=True)

    first.point()
    one, two = client("first pointer"), client("second pointer")
    vp1, vp2 = one.virtual_pointer(), two.virtual_pointer()
    start = len(first.events)
    for producer, virtual in [(one, vp1), (two, vp2)]:
        producer.send(virtual, 2, uints(0, 272, 1))
        producer.roundtrip()
    one.send(vp1, 8)
    one.roundtrip()
    first.roundtrip()
    check(first.buttons(start) == [(272, 1)], "one pointer released another pointer's button")
    two.send(vp2, 8)
    two.roundtrip()
    first.button(272, 0, start)
    check(first.buttons(start) == [(272, 1), (272, 0)], "shared button was not balanced")
    second.point()
    print("ok - shared virtual buttons release only when the last owner is gone", flush=True)

    # Observe both legacy axis_discrete and v8 axis_value120 delivery. Sending
    # repeated extremes also exercises the legacy accumulator after each step.
    seat = first.bind("wl_seat", 8)
    modern = first.alloc()
    first.send(seat, 0, uints(modern))
    first.roundtrip()
    first.point()
    producer = client("extreme scroll producer")
    virtual = producer.virtual_pointer()
    limit = (2**31 - 1 - 119) // 120
    for axis in (0, 1):
        for steps in (2**31 - 1, 2**31 - 1, -2**31, -2**31, -2, 1):
            start = len(first.events)
            producer.send(virtual, 7, struct.pack("=IIii", 0, axis, 256, steps))
            producer.send(virtual, 4)  # frame
            bounded = max(-limit, min(limit, steps))
            for obj, opcode, expected in ((first.pointer, 8, bounded),
                                          (modern, 9, bounded * 120)):
                first.wait(lambda e: e[:2] == (obj, opcode)
                           and struct.unpack("=Ii", e[2]) == (axis, expected),
                           f"scroll {steps} on pointer {obj}", start)
    print("ok - extreme scroll stays bounded for legacy and v8 pointers", flush=True)


def lock(first, second, client):
    second.point()
    first.point()
    first.focus()
    producer = client("pointer held across lock")
    virtual = producer.virtual_pointer()
    start = len(first.events)
    producer.send(virtual, 2, uints(0, 272, 1))
    first.button(272, 1, start)
    # Leave work queued for several seconds, then lock before it drains.
    evaluate(r'(begin (wm-send-string (make-string 80 #\A) 200) (wm-click 1 10) #t)')
    owner = client("input lifecycle lock owner")
    lock_object = owner.accepted_lock()
    owner.lock_surface(lock_object)
    owner.wait(lambda e: e[:2] == (owner.keyboard, 1), "lock keyboard enter")
    for target in (first, second):
        target.roundtrip()
    starts = [len(target.events) for target in (first, second)]
    position = ipc("(rust-call 'wm-pointer-position)")
    for command in ["wm-click 1", 'wm-send-string "secret" 0', 'wm-send-key 4 "v"',
                    "wm-paste", "wm-scroll 1 2", "wm-warp-pointer 900 500",
                    "wm-warp-pointer-relative 20 20"]:
        evaluate(f"(rust-call '{command})", "#f")
    check(owner.modifiers() == (0, 0, 0), "lock inherited synthetic modifiers")
    producer.send(virtual, 2, uints(0, 272, 0))
    producer.roundtrip()
    time.sleep(0.5)  # Span queued click timers before checking for stray input.
    check(ipc("(rust-call 'wm-pointer-position)") == position, "pointer moved while locked")
    for target, start in zip((first, second), starts):
        target.roundtrip()
        check(not target.keyboard_events(start), "queued input reached a locked desktop")
        check(not target.buttons(start), "queued click reached a locked desktop")
    protocol.inject_key(owner, first, second)
    owner.send(lock_object, 2)
    owner.roundtrip()
    protocol.locked(False)
    second.point()
    # A different button would remain grabbed if the pre-lock left press leaked.
    start = len(second.events)
    producer.send_batch([(virtual, 2, uints(0, 273, 1)), (virtual, 2, uints(0, 273, 0))])
    second.button(273, 0, start)
    first.point()
    for target in (first, second):
        target.roundtrip()
    starts = [len(target.events) for target in (first, second)]
    time.sleep(1.5)  # All ten old click timers would have elapsed by now.
    for target, start in zip((first, second), starts):
        target.roundtrip()
        check(not target.buttons(start) and not target.keyboard_events(start, (3,)),
              "cancelled input resumed after unlock")
    print("ok - lock rejects new synthetic input, cancels queues and releases held buttons", flush=True)

    # Cancellation must stop the drag's periodic motion as well as its grab.
    first.point()
    result = ipc(f'(wm-drop-text {first.x} 180 "cancelled drag")')
    token = re.fullmatch(r"\(ok (\d+)\)", result)
    check(token, f"drag token: {result}")
    drag_owner = client("lock during drag")
    drag_lock = drag_owner.accepted_lock()
    evaluate(f"(rust-call 'wm-automation-status {token[1]})", "(drop-text cancelled)")
    drag_owner.send(drag_lock, 2)
    drag_owner.roundtrip()
    protocol.locked(False)
    position = ipc("(rust-call 'wm-pointer-position)")
    for _ in range(30):
        time.sleep(0.037)  # Avoid aliasing the 25 ms drag timer.
        check(ipc("(rust-call 'wm-pointer-position)") == position,
              "cancelled drag timer moved pointer after unlock")
    print("ok - cancelled drag timers do not resume after unlock", flush=True)

    dying = client("drag target that disconnects")
    dying.mapped(400)
    result = ipc(f'(wm-drop-text {dying.x} 180 "cancelled target")')
    token = re.fullmatch(r"\(ok (\d+)\)", result)
    check(token, f"drag token: {result}")
    dying.close()
    deadline = time.monotonic() + protocol.TIMEOUT
    while ipc(f"(rust-call 'wm-automation-status {token[1]})") != "(ok (drop-text cancelled))":
        check(time.monotonic() < deadline, "destroyed drag target was not cancelled")
        time.sleep(0.01)
    time.sleep(0.07)  # Let the cancelled drag's next tick clean up its press.
    second.point()
    start = len(second.events)
    producer.send_batch([(virtual, 2, uints(0, 273, 1)), (virtual, 2, uints(0, 273, 0))])
    second.button(273, 0, start)
    first.point()
    print("ok - disappearing drag targets release the automation button", flush=True)


def main():
    clients = []

    def client(name):
        result = Client(name)
        clients.append(result)
        return result

    try:
        protocol.locked(False)
        first, second = client("first input target"), client("second input target")
        first.mapped(100)
        second.mapped(700)
        cases = {"typing": typing, "pointer": pointer, "lock": lock}
        for name in os.environ.get("MINDE_INPUT_LIFECYCLE_CASES", "typing pointer lock").split():
            cases[name](first, second, client)
        print("input-lifecycle: all checks passed", flush=True)
    finally:
        host("keyup", "Shift_L", "Control_L")
        for connection in clients:
            connection.close()


if __name__ == "__main__":
    main()
