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


class ResizeClient(Client):
    """Keep configure acknowledgements and buffer commits under test control."""
    def prepare(self):
        self.mapped(100)
        self.roundtrip()
        self.seat = self.bind("wl_seat", 5)
        self.virtual = self.virtual_pointer()
        self.configures = []
        self.configure_states = ()

        def toplevel(opcode, data):
            if opcode == 0:
                width, height, length = struct.unpack_from("=iiI", data)
                self.configured_size[:] = [width, height]
                self.configure_states = struct.unpack_from(f"={length // 4}I", data, 12)

        def configure(opcode, data):
            if opcode == 0:
                self.configures.append((struct.unpack("=I", data)[0],
                                        tuple(self.configured_size), self.configure_states))

        self.handlers[self.toplevel_object] = toplevel
        self.handlers[self.xdg_surface] = configure

    def configured(self, resizing, start):
        self.wait(lambda e: e[:2] == (self.xdg_surface, 0)
                  and any((3 in c[2]) == resizing for c in self.configures[start:]),
                  f"configure with resizing={resizing}", len(self.events))
        return next(c for c in self.configures[start:] if (3 in c[2]) == resizing)

    def commit_configure(self, configure, size=None):
        self.send(self.xdg_surface, 4, uints(configure[0]))
        self.paint(self.surface, *(size or configure[1]), 0x0000FFFF)
        self.roundtrip()

    def start_resize(self, edge):
        self.point()
        start = len(self.events)
        self.send(self.virtual, 2, uints(0, 272, 1))
        self.button(272, 1, start)
        event = next(e for e in self.events[start:] if e[:2] == (self.pointer, 3))
        serial = struct.unpack_from("=I", event[2])[0]
        self.send(self.toplevel_object, 6, uints(self.seat, serial, edge))
        self.roundtrip()


def resize(first, second, client):
    def rectangle(edge, size):
        width, height = size
        return (100 + (300 - width if edge & 4 else 0),
                100 + (250 - height if edge & 1 else 0), width, height)

    def check_rectangle(target, rect):
        expected = "(" + " ".join(map(str, rect)) + ")"
        for query in ("wm-window-geometry", "float-geometry"):
            evaluate(f"({query} {target.window_id})", expected)

    for edge in (4, 1, 5, 6, 9, 10):  # left, top, and all four corners
        target = client(f"delayed resize edge {edge}", ResizeClient)
        target.prepare()
        target.start_resize(edge)
        start = len(target.configures)
        evaluate("(wm-warp-pointer 220 190)")
        earlier = target.configured(True, start)
        start = len(target.configures)
        evaluate("(wm-warp-pointer 240 210)")
        latest = target.configured(True, start)
        start = len(target.configures)
        target.send(target.virtual, 2, uints(0, 272, 0))
        final = target.configured(False, start)
        check(final[1] == latest[1], "release lost the last requested size")
        expected = "(" + " ".join(map(str, rectangle(edge, final[1]))) + ")"
        evaluate(f"(float-geometry {target.window_id})", expected)

        # Commits for older configures and content-only commits must not
        # consume the final reposition. Keep the final configure unacked.
        target.commit_configure(earlier)
        target.paint(target.surface, *earlier[1], 0x00FF0000)
        target.roundtrip()
        expected = "(" + " ".join(map(str, rectangle(edge, earlier[1]))) + ")"
        evaluate(f"(wm-window-geometry {target.window_id})", expected)

        # Floating clients may choose another size (e.g. terminal cell
        # increments). The final acknowledged commit is authoritative.
        size = (final[1][0] - 7, final[1][1] + 9) if edge == 9 else final[1]
        target.commit_configure(final, size)
        check_rectangle(target, rectangle(edge, size))
        evaluate("(begin (sync-frames!) #t)")
        target.roundtrip()
        check_rectangle(target, rectangle(edge, size))

        evaluate(f"(begin (set-float-geometry! {target.window_id} '(160 120 220 180))"
                 " (sync-frames!) #t)")
        target.roundtrip()
        target.commit_configure(target.configures[-1])
        check_rectangle(target, (160, 120, 220, 180))
        target.close()
    print("ok - delayed resize commits preserve the anchored edges and final policy geometry", flush=True)
    print("ok - client-chosen resize sizes survive relayout without stale positioning", flush=True)


def resize_lock(first, second, client):
    target = client("resize interrupted by lock", ResizeClient)
    target.prepare()
    target.start_resize(5)
    start = len(target.configures)
    evaluate("(wm-warp-pointer 220 190)")
    target.commit_configure(target.configured(True, start))
    start = len(target.configures)
    owner = client("resize cancellation lock owner")
    lock_object = owner.accepted_lock()
    owner.lock_surface(lock_object)
    final = target.configured(False, start)
    target.commit_configure(final)
    target.send(target.virtual, 2, uints(0, 272, 0))
    target.roundtrip()
    owner.send(lock_object, 2)
    owner.roundtrip()
    protocol.locked(False)
    evaluate("(begin (sync-frames!) #t)")
    target.roundtrip()
    evaluate(f"(wm-window-geometry {target.window_id})", "(120 110 280 240)")
    evaluate(f"(float-geometry {target.window_id})", "(120 110 280 240)")
    check(all(3 not in c[2] for c in target.configures if c[0] >= final[0]),
          "resize state returned after unlocking")
    target.close()
    print("ok - lock cancellation sends a final resize configure and preserves committed geometry", flush=True)


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


def drag(first, second, client):
    producer = client("drag button producer")
    virtual = producer.virtual_pointer()
    payload = Path(protocol.RUNTIME) / "drop-payload.txt"
    payload.write_text("file drop regression\n")

    def reject_drops(label):
        position = ipc("(wm-pointer-position)")
        focus = ipc("(focused-window-id)")
        first.roundtrip()
        start = len(first.events)
        for operation, argument in (("drop-text", '"must not arrive"'),
                                    ("drop-files", f'(list "{payload}")')):
            result = ipc(f'(wm-{operation} {second.x} 180 {argument})')
            token = re.fullmatch(r"\(ok (\d+)\)", result)
            check(token, f"{label}: missing request token: {result}")
            evaluate(f"(wm-automation-status {token[1]})", f"({operation} cancelled)")
            check(ipc("(wm-pointer-position)") == position, f"{label}: rejected drop moved pointer")
            check(ipc("(focused-window-id)") == focus, f"{label}: rejected drop changed focus")
        first.roundtrip()
        check(not first.buttons(start), f"{label}: rejected drop injected a button event")

    for button in (272, 273):
        first.point()
        start = len(first.events)
        producer.send(virtual, 2, uints(0, button, 1))
        first.button(button, 1, start)
        reject_drops("held click")
        producer.send(virtual, 2, uints(0, button, 0))
        first.button(button, 0, start)
        check(first.buttons(start) == [(button, 1), (button, 0)], "held click became unbalanced")
        second.point()
    print("ok - automation drops preserve held left and right clicks", flush=True)

    def geometry():
        result = ipc(f"(wm-window-geometry {first.window_id})")
        match = re.fullmatch(r"\(ok \((-?\d+) (-?\d+) (\d+) (\d+)\)\)", result)
        check(match, f"window geometry: {result}")
        return tuple(map(int, match.groups()))

    for button in (272, 273):
        first.point()
        first.focus()
        before = geometry()
        start = len(first.events)
        host("keydown", "Super_L")
        first.key(125, 1, start)
        producer.send(virtual, 2, uints(0, button, 1))
        producer.roundtrip()
        host("keyup", "Super_L")
        first.key(125, 0, start)
        reject_drops("move/resize grab")
        evaluate(f"(wm-warp-pointer {first.x + 20} 200)")
        expected = (before[0] + 20, before[1] + 20, *before[2:]) if button == 272 else \
            (*before[:2], before[2] + 20, before[3] + 20)
        deadline = time.monotonic() + protocol.TIMEOUT
        while geometry() != expected:
            check(time.monotonic() < deadline, f"move/resize grab stopped working: {geometry()}")
            first.roundtrip()  # Ack resize configures and commit the new size.
        producer.send(virtual, 2, uints(0, button, 0))
        producer.roundtrip()
        second.point()
    print("ok - automation drops preserve working compositor move/resize grabs", flush=True)

    devices = []
    for target in (first, second):
        manager = target.bind("wl_data_device_manager", 3)
        seat, device = target.bind("wl_seat", 5), target.alloc()
        target.send(manager, 1, uints(device, seat))
        target.roundtrip()
        devices.append((manager, device))
    manager, origin_device = devices[0]
    target_device = devices[1][1]
    first.point()
    start = len(first.events)
    producer.send(virtual, 2, uints(0, 272, 1))
    event = first.wait(lambda e: e[:2] == (first.pointer, 3)
                       and struct.unpack("=IIII", e[2])[2:] == (272, 1), "drag press", start)
    serial = struct.unpack_from("=I", event[2])[0]
    source = first.alloc()
    first.send(manager, 0, uints(source))
    first.send(source, 0, protocol.string("text/plain;charset=utf-8"))
    first.send(source, 2, uints(1))  # Offer the copy action.
    first.send(origin_device, 0, uints(source, first.surface, 0, serial))
    first.roundtrip()
    evaluate(f"(wm-warp-pointer {first.x + 1} 180)")
    first.wait(lambda e: e[:2] == (origin_device, 1), "client drag enter", start)
    reject_drops("client drag")
    first.roundtrip()
    check(not any(e[:2] == (source, 2) for e in first.events), "client drag was cancelled")

    start = len(second.events)
    evaluate(f"(wm-warp-pointer {second.x} 180)")
    event = second.wait(lambda e: e[:2] == (target_device, 1), "client drag target enter", start)
    serial, surface, _, _, offer = struct.unpack("=IIiiI", event[2])
    check(surface == second.surface and offer, "client drag entered wrong target")
    second.send(offer, 0, uints(serial) + protocol.string("text/plain;charset=utf-8"))
    second.send(offer, 4, uints(1, 1))
    second.roundtrip()
    producer.send(virtual, 2, uints(0, 272, 0))
    second.wait(lambda e: e[:2] == (target_device, 4), "client drop", start)
    first.wait(lambda e: e[:2] == (source, 3), "client drop performed")
    second.send(offer, 3)  # finish
    first.wait(lambda e: e[:2] == (source, 4), "client drag finished")
    check(not any(e[:2] == (source, 2) for e in first.events), "finished client drag was cancelled")
    print("ok - rejected automation leaves client drag negotiation and completion intact", flush=True)

    second.point()
    start = len(second.events)
    result = ipc(f'(wm-drop-text {second.x} 180 "idle drop")')
    token = re.fullmatch(r"\(ok (\d+)\)", result)
    check(token, f"idle drag token: {result}")
    event = second.wait(lambda e: e[:2] == (target_device, 1), "automation enter", start)
    serial, _, _, _, offer = struct.unpack("=IIiiI", event[2])
    second.send(offer, 0, uints(serial) + protocol.string("text/plain;charset=utf-8"))
    second.send(offer, 4, uints(1, 1))
    second.wait(lambda e: e[:2] == (target_device, 4), "automation drop", start)
    second.send(offer, 3)
    second.roundtrip()
    evaluate(f"(wm-automation-status {token[1]})", "(drop-text accepted)")
    print("ok - automation drops succeed once the pointer is idle", flush=True)


def main():
    clients = []

    def client(name, kind=Client):
        result = kind(name)
        clients.append(result)
        return result

    try:
        protocol.locked(False)
        first, second = client("first input target"), client("second input target")
        first.mapped(100)
        second.mapped(700)
        cases = {"typing": typing, "pointer": pointer, "lock": lock, "drag": drag,
                 "resize": resize, "resize-lock": resize_lock}
        for name in os.environ.get("MINDE_INPUT_LIFECYCLE_CASES",
                                   "typing pointer lock drag resize resize-lock").split():
            cases[name](first, second, client)
        print("input-lifecycle: all checks passed", flush=True)
    finally:
        host("keyup", "Shift_L", "Control_L", "Super_L")
        for connection in clients:
            connection.close()


if __name__ == "__main__":
    main()
