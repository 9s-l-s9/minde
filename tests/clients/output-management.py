#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Output-manager serial isolation and configuration protocol errors."""
from pathlib import Path
import runpy
import struct
import sys

api = runpy.run_path(str(Path(__file__).with_name("session-lock.py")))
Client, ProtocolError, check, uints = (api[name] for name in
                                    ("Client", "ProtocolError", "check", "uints"))


class Outputs(Client):
    def __init__(self, name):
        super().__init__(name)
        self.manager = self.bind("zwlr_output_manager_v1", 4)
        self.roundtrip()
        self.head = struct.unpack("=I", self.wait(
            lambda e: e[:2] == (self.manager, 0), "output head")[2])[0]
        self.mode = struct.unpack("=I", self.wait(
            lambda e: e[:2] == (self.head, 3), "output mode")[2])[0]

    def serial(self):
        return [e[2] for e in self.events if e[:2] == (self.manager, 1)][-1]

    def configuration(self, enabled=True, serial=None):
        config = self.alloc()
        self.send(self.manager, 0, uints(config) + (serial or self.serial()))
        head = 0
        if enabled is True:
            head = self.alloc()
            self.send(config, 0, uints(head, self.head))
        elif enabled is False:
            self.send(config, 1, uints(self.head))
        return config, head

    def result(self, config, opcode=3, expected=0):
        self.send(config, opcode)
        event = self.wait(lambda e: e[0] == config and e[1] in (0, 1, 2),
                          "configuration result")
        check(event[1] == expected, f"expected result {expected}, received {event}")
        self.send(config, 4)
        self.roundtrip()

    def error(self, obj, code):
        try:
            # A protocol error may already have closed the server's receive
            # side. Drain its error event without sending another request.
            self.wait(lambda _: False, "protocol error")
        except ProtocolError as error:
            check(f"object {obj}, code {code}:" in str(error), f"wrong protocol error: {error}")
        else:
            raise AssertionError(f"expected protocol error {code} on object {obj}; request was accepted")


def serials():
    a = Outputs("long-lived output client")
    b = None
    try:
        serial = a.serial()
        config, _ = a.configuration()
        b = Outputs("new output client")
        a.roundtrip()
        check(a.serial() == serial == b.serial(), "binding a manager changed the layout serial")
        a.result(config)
        # A complete no-op apply must not invalidate other clients either.
        b.result(b.configuration()[0], opcode=2)
        a.result(a.configuration()[0])
        old, _ = a.configuration()
        config, head = b.configuration()
        b.send(head, 4, struct.pack("=i", 320))  # scale 1.25
        b.result(config, opcode=2)
        a.roundtrip()
        check(a.serial() == b.serial() != serial, "real changes did not reach both managers")
        a.result(old, expected=2)
        a.result(a.configuration()[0])
        config, head = b.configuration()
        b.send(head, 4, struct.pack("=i", 256))
        b.result(config, opcode=2)
        print("ok - binds preserve configurations; real changes notify all clients and cancel stale ones", flush=True)
    finally:
        a.close()
        if b:
            b.close()


def invalid(name, requests, expected, enabled=True, on_config=False):
    c = Outputs(name)
    try:
        config, head = c.configuration(enabled)
        c.send_batch(requests(c, config, head))
        c.error(config if on_config else head, expected)
    finally:
        c.close()
    # A client error must not poison compositor state or another connection.
    probe = Outputs("healthy client after " + name)
    try:
        probe.result(probe.configuration()[0])
    finally:
        probe.close()
    print("ok - " + name, flush=True)


def wrong_mode(c, config, head):
    # Two advertisements of the same physical output still have distinct head
    # resources. Equal mode values cannot establish ownership of a mode object.
    manager = c.bind("zwlr_output_manager_v1", 4)
    c.roundtrip()
    other_head = struct.unpack("=I", c.wait(lambda e: e[:2] == (manager, 0), "second head")[2])[0]
    other_mode = struct.unpack("=I", c.wait(lambda e: e[:2] == (other_head, 3), "second mode")[2])[0]
    return [(head, 0, uints(other_mode))]


def errors(selected):
    cases = {
        "scale-zero": (lambda c, f, h: [(h, 4, struct.pack("=i", 0))], 5),
        "scale-negative": (lambda c, f, h: [(h, 4, struct.pack("=i", -256))], 5),
        "transform": (lambda c, f, h: [(h, 3, uints(99))], 4),
        "custom-width": (lambda c, f, h: [(h, 1, uints(0, 600, 60000))], 3),
        "custom-height": (lambda c, f, h: [(h, 1, struct.pack("=iii", 800, -1, 60000))], 3),
        "custom-refresh": (lambda c, f, h: [(h, 1, struct.pack("=iii", 800, 600, -1))], 3),
        "adaptive-sync": (lambda c, f, h: [(h, 5, uints(99))], 6),
        "foreign-mode": (wrong_mode, 2),
        "mode-then-custom": (lambda c, f, h: [(h, 0, uints(c.mode)), (h, 1, uints(800, 600, 0))], 1),
        "custom-then-mode": (lambda c, f, h: [(h, 1, uints(800, 600, 0)), (h, 0, uints(c.mode))], 1),
    }
    setters = (
        ("mode", 0, lambda c: uints(c.mode)),
        ("custom-mode", 1, lambda c: uints(800, 600, 0)),
        ("position", 2, lambda c: uints(0, 0)),
        ("transform", 3, lambda c: uints(0)),
        ("scale", 4, lambda c: struct.pack("=i", 256)),
        ("adaptive-sync", 5, lambda c: uints(0)),
    )
    for name, opcode, payload in setters:
        cases["duplicate-" + name] = (
            lambda c, f, h, op=opcode, data=payload: [(h, op, data(c))] * 2, 1)
    for name, (requests, code) in cases.items():
        if not selected or name in selected:
            invalid(name, requests, code)
    for action, opcode in (("test", 3), ("apply", 2)):
        name = "omitted-" + action
        if not selected or name in selected:
            invalid(name, lambda c, f, h, op=opcode: [(f, op, b"")], 2,
                    enabled=None, on_config=True)
        requests = {
            "enable": lambda c, f, h: (f, 0, uints(c.alloc(), c.head)),
            "disable": lambda c, f, h: (f, 1, uints(c.head)),
            "test": lambda c, f, h: (f, 3, b""),
            "apply": lambda c, f, h: (f, 2, b""),
        }
        for name, setter, payload in setters:
            requests[name] = lambda c, f, h, op=setter, data=payload: (h, op, data(c))
        for suffix, request in requests.items():
            name = f"after-{action}-{suffix}"
            if not selected or name in selected:
                invalid(name, lambda c, f, h, op=opcode, req=request:
                        [(f, op, b""), req(c, f, h)], 3, on_config=True)
    for name, request in (
        ("duplicate-enable", lambda c, f, h: (f, 0, uints(c.alloc(), c.head))),
        ("duplicate-disable", lambda c, f, h: (f, 1, uints(c.head))),
    ):
        if not selected or name in selected:
            invalid(name, lambda c, f, h, req=request: [req(c, f, h)], 1, on_config=True)


def valid_and_destroy():
    c = Outputs("valid configuration lifecycle")
    try:
        for opcode in (3, 2):
            config, head = c.configuration()
            c.send_batch([(head, 0, uints(c.mode)),
                          (head, 2, uints(0, 0)),
                          (head, 3, uints(0)),
                          (head, 4, struct.pack("=i", 256)),
                          (head, 5, uints(0))])
            c.result(config, opcode=opcode)
            c.wait(lambda e: e[:2] == (1, 1) and e[2] == uints(head),
                   "configuration-head destruction")
        # Valid protocol requests may still be refused by backend capabilities.
        config, head = c.configuration()
        c.send(head, 5, uints(1))
        c.result(config, expected=1)
        print("ok - valid properties succeed, backend refusals stay nonfatal, and destroy retires children", flush=True)
    finally:
        c.close()


if __name__ == "__main__":
    selected = set(sys.argv[1:])
    if not selected or "serials" in selected:
        serials()
    if selected != {"serials"}:
        errors(selected)
    if not selected or "lifecycle" in selected:
        valid_and_destroy()
