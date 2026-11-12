#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Map, hide and remap live xdg toplevels against the private nested compositor."""
import importlib.util
from pathlib import Path
import re
import struct

spec = importlib.util.spec_from_file_location("protocol", Path(__file__).with_name("session-lock.py"))
protocol = importlib.util.module_from_spec(spec)
spec.loader.exec_module(protocol)
check, ipc, uints, string = protocol.check, protocol.ipc, protocol.uints, protocol.string


def evaluate(expression, expected="#t"):
    result = ipc(expression)
    check(result == f"(ok {expected})", f"{expression}: {result}")


def window_id(name):
    result = ipc(f'(find (lambda (id) (equal? (window-title id) "{name}")) (all-window-ids))')
    match = re.fullmatch(r"\(ok (\d+)\)", result)
    check(match, f"window {name}: {result}")
    return int(match[1])


def grabbed_popup(client, seat, serial):
    shell = client.bind("xdg_wm_base")
    client.handlers[shell] = lambda op, data: client.send(shell, 3, data) if op == 0 else None
    positioner = client.alloc()
    client.send(shell, 1, uints(positioner))
    client.send(positioner, 1, uints(100, 80))
    client.send(positioner, 2, uints(10, 10, 20, 20))
    surface = client.new_surface()
    xdg, popup = client.alloc(), client.alloc()
    client.send(shell, 2, uints(xdg, surface))
    client.send(xdg, 2, uints(popup, client.xdg_surface, positioner))
    client.send(popup, 1, uints(seat, serial))

    def configure(opcode, data):
        if opcode == 0:
            client.send(xdg, 4, data)
            client.paint(surface, 100, 80, 0x0000FF00)

    client.handlers[xdg] = configure
    client.send(surface, 6)
    client.wait(lambda e: e[:2] == (xdg, 0), "popup configure")
    client.roundtrip()
    return surface, xdg, popup


class Observer(protocol.Client):
    def __init__(self):
        super().__init__("window lifecycle observer")
        self.manager = self.bind("zwlr_foreign_toplevel_manager_v1", 3)
        self.roundtrip()

    def live_handles(self):
        self.roundtrip()
        created = [struct.unpack("=I", data)[0] for obj, op, data in self.events
                   if (obj, op) == (self.manager, 0)]
        closed = {obj for obj, op, _ in self.events if op == 6}
        return [obj for obj in created if obj not in closed]

    def handle(self, title):
        live = self.live_handles()
        for obj, op, data in reversed(self.events):
            if obj in live and op == 0 and data[4:-1].rstrip(b"\0").decode() == title:
                return obj
        raise AssertionError(f"no foreign handle for {title}")


def main():
    observer, anchor, target = Observer(), protocol.Client("anchor"), protocol.Client("pending")
    try:
        evaluate('(begin (define lifecycle-maps (quote ())) (define lifecycle-unmaps (quote ()))'
                 ' (add-event-hook! (quote new-window)'
                 '  (lambda (id title app) (set! lifecycle-maps (cons id lifecycle-maps))))'
                 ' (add-event-hook! (quote destroy-window)'
                 '  (lambda (id) (set! lifecycle-unmaps (cons id lifecycle-unmaps)))) #t)')
        anchor.toplevel()
        anchor_id = window_id("anchor")
        target.toplevel(map_on_configure=False)
        seat = target.bind("wl_seat", 5)
        pointer = target.alloc()
        target.send(seat, 0, uints(pointer))
        manager = target.bind("zwlr_virtual_pointer_manager_v1")
        virtual = target.alloc()
        target.send(manager, 0, uints(0, virtual))
        anchor_seat = anchor.bind("wl_seat", 5)
        anchor_pointer = anchor.alloc()
        anchor.send(anchor_seat, 0, uints(anchor_pointer))
        target.roundtrip()
        anchor.roundtrip()
        evaluate('(length (all-window-ids))', "1")
        evaluate('(length lifecycle-maps)', "1")
        check(len(observer.live_handles()) == 1, "bufferless role announced a foreign handle")
        print("ok - configuring a bufferless toplevel creates no managed window", flush=True)

        old_id = None
        for cycle in range(6):
            name = f"lifecycle-{cycle}"
            target.send(target.toplevel_object, 2, string(name))
            target.send(target.toplevel_object, 3, string("org.minde.lifecycle"))
            if cycle == 1:
                target.send(target.toplevel_object, 11, uints(0))  # fullscreen before mapping
                target.roundtrip()
                check(target.configured_size == [1280, 800], "pre-map fullscreen has no output size")
            target.map_on_configure = True
            target.paint(target.surface, *target.configured_size, 0x00FF00FF)
            target.roundtrip()
            current = window_id(name)
            check(old_id is None or current > old_id, "remap reused a stale managed ID")
            evaluate('(length (all-window-ids))', "2")
            evaluate('(focused-window-id)', str(current))
            evaluate('(length lifecycle-maps)', str(cycle + 2))
            evaluate(f'(window-app-id {current})', '"org.minde.lifecycle"')
            if cycle == 1:
                evaluate('(fullscreen-window)', str(current))
            handle = observer.handle(name)
            check(len(observer.live_handles()) == 2, "mapped window was not announced exactly once")
            # Ordinary content and state-only commits are not new maps.
            target.paint(target.surface, *target.configured_size, 0x0000FFFF)
            target.send(target.surface, 6)
            target.roundtrip()
            evaluate('(length lifecycle-maps)', str(cycle + 2))

            if cycle == 5:
                # A cancelled top-left resize must not reposition the next
                # mapping when it commits its newly requested dimensions.
                evaluate(f'(begin (float-window! {current} (list 160 120 220 180))'
                         ' (sync-frames!) #t)')
                target.roundtrip()
                target.roundtrip()
                evaluate(f'(wm-window-geometry {current})', "(160 120 220 180)")

            popup_objects = None
            if 2 <= cycle <= 4:
                evaluate(f'(begin (float-window! {current} (list 100 100 300 250))'
                         ' (sync-frames!) (wm-warp-pointer 200 180))')
                target.roundtrip()
                start = len(target.events)
                target.send(virtual, 2, uints(0, 272, 1))
                event = target.wait(lambda e: e[:2] == (pointer, 3)
                                    and struct.unpack("=IIII", e[2])[2:] == (272, 1),
                                    "move/resize press", start)
                serial = struct.unpack_from("=I", event[2])[0]
                if cycle == 2:
                    popup_objects = grabbed_popup(target, seat, serial)
                elif cycle == 3:
                    target.send(target.toplevel_object, 5, uints(seat, serial))
                else:
                    target.send(target.toplevel_object, 6, uints(seat, serial, 5))  # top-left
                target.roundtrip()

            anchor.roundtrip()
            target.roundtrip()
            anchor_start, target_start = len(anchor.events), len(target.events)
            target.map_on_configure = False
            target.send(target.surface, 1, uints(0, 0, 0))
            target.send(target.surface, 6)
            target.roundtrip()
            evaluate('(length (all-window-ids))', "1")
            evaluate('(focused-window-id)', str(anchor_id))
            evaluate('(fullscreen-window)', "#f")
            evaluate(f'(wm-window-geometry {current})', "#f")
            evaluate(f'(wm-window-title {current})', "#f")
            evaluate('(length lifecycle-unmaps)', str(cycle + 1))
            anchor.wait(lambda e: e[:2] == (anchor.keyboard, 1), "focus after unmap", anchor_start)
            target.wait(lambda e: e[:2] == (target.keyboard, 2), "hidden keyboard leave", target_start)
            observer.wait(lambda e: e[:2] == (handle, 6), "foreign handle closed")
            check(len(observer.live_handles()) == 1, "hidden window remained in foreign registry")
            if popup_objects:
                surface, xdg, popup = popup_objects
                target.wait(lambda e: e[:2] == (popup, 1), "hidden popup dismissed", target_start)
                target.send(popup, 0)
                target.send(xdg, 0)
                target.send(surface, 0)
                target.roundtrip()
            if 2 <= cycle <= 4:
                evaluate('(wm-warp-pointer 50 50)')
                anchor.wait(lambda e: e[:2] == (anchor_pointer, 0),
                            "pointer after hidden move/resize", anchor_start)
                target.send(virtual, 2, uints(0, 272, 0))
                target.roundtrip()
            target.send(target.surface, 6)
            target.roundtrip()
            evaluate('(length lifecycle-unmaps)', str(cycle + 1))
            old_id = current
        print("ok - repeated null-buffer unmaps clear layout, focus, fullscreen and foreign handles", flush=True)
        print("ok - remaps get fresh IDs and honor pre-map metadata and fullscreen", flush=True)
        print("ok - hiding a window dismisses popups and ends popup/move/resize grabs", flush=True)
        print("ok - remapping after a cancelled resize uses the new geometry", flush=True)

        # Destroying a still-unmapped role must not fire a second unmap.
        target.send(target.toplevel_object, 0)
        target.send(target.xdg_surface, 0)
        target.send(target.surface, 0)
        target.roundtrip()
        evaluate('(length lifecycle-unmaps)', "6")
        anchor.map_on_configure = False
        anchor.send(anchor.toplevel_object, 0)
        anchor.send(anchor.xdg_surface, 0)
        anchor.send(anchor.surface, 0)
        anchor.roundtrip()
        evaluate('(all-window-ids)', "()")
        evaluate('(focused-window-id)', "#f")
        evaluate('(length lifecycle-unmaps)', "7")
        check(not observer.live_handles(), "destroyed window retained a foreign handle")
        print("ok - mapped and unmapped role destruction emit unmap exactly once", flush=True)
    finally:
        for connection in (target, anchor, observer):
            connection.close()


if __name__ == "__main__":
    main()
