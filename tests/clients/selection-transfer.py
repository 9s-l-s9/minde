#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Stalled selection owners/readers must not retain unbounded compositor resources."""
from collections import deque
import fcntl
import os
from pathlib import Path
import runpy
import struct
import time

shared = runpy.run_path(str(Path(__file__).with_name("input-lifecycle.py")))
protocol = shared["protocol"]
check, ipc, uints, string = protocol.check, protocol.ipc, protocol.uints, protocol.string
evaluate = shared["evaluate"]
MIME = "text/plain;charset=utf-8"
LIMIT = 32
PID = int(os.environ["MINDE_TRANSFER_TEST_PID"])


def resources():
    status = Path(f"/proc/{PID}/status").read_text()
    threads = int(next(line.split()[1] for line in status.splitlines() if line.startswith("Threads:")))
    return len(list(Path(f"/proc/{PID}/fd").iterdir())), threads


def wait_for(predicate, message, timeout=7):
    deadline = time.monotonic() + timeout
    while not predicate():
        check(time.monotonic() < deadline, message)
        time.sleep(0.01)


class Client(shared["Client"]):
    def __init__(self):
        self.keep_fds = False
        self.incoming = deque()
        self.writers = []
        super().__init__("selection transfer target")
        self.mapped(100)
        self.control = self.bind("zwlr_data_control_manager_v1", 2)
        seat = self.bind("wl_seat")
        self.device = self.alloc()
        self.send(self.control, 1, uints(self.device, seat))
        manager = self.bind("wl_data_device_manager", 3)
        self.drag_device = self.alloc()
        self.send(manager, 1, uints(self.drag_device, seat))
        self.roundtrip()

    def received_fd(self, fd):
        if self.keep_fds:
            self.incoming.append(fd)
        else:
            os.close(fd)

    def receive(self, offer, opcode):
        reader, writer = os.pipe2(os.O_CLOEXEC)
        os.set_blocking(reader, False)
        try:
            fcntl.fcntl(reader, fcntl.F_SETPIPE_SZ, 4096)
            self.send(offer, opcode, string(MIME), fd=writer)
        finally:
            os.close(writer)
        return reader

    def contents(self, reader):
        data = bytearray()
        deadline = time.monotonic() + 7
        while True:
            try:
                chunk = os.read(reader, 16384)
                if not chunk:
                    return bytes(data)
                data.extend(chunk)
            except BlockingIOError:
                check(time.monotonic() < deadline, "selection reader never reached EOF")
                self.roundtrip()
                time.sleep(0.001)

    def flood(self, offer, opcode, finish, label):
        baseline, threads = resources()
        readers = []
        try:
            for _ in range(96):
                readers.append(self.receive(offer, opcode))
            self.roundtrip()
            evaluate("#t")  # compositor remains responsive under the stalled load
            retained, current_threads = resources()
            check(current_threads <= threads + 2,
                  f"{label}: reader flood grew threads from {threads} to {current_threads}")
            check(retained <= baseline + LIMIT + 2,
                  f"{label}: reader flood retained {retained - baseline} fds")
            if finish:
                finish()
            self.roundtrip()
            wait_for(lambda: resources()[0] <= baseline, f"{label}: stalled writes retained fds", 1 if finish else 7)
            for reader in readers:
                self.contents(reader)
        finally:
            for reader in readers:
                os.close(reader)
        print(f"ok - {label} receive flood is bounded and stalled writes close", flush=True)

    def clipboard(self):
        for setter, opcode in (("wm-set-clipboard", 1), ("wm-set-primary", 3)):
            start = len(self.events)
            evaluate(f"({setter} (make-string 262144 #\\A))")
            offer = struct.unpack("=I", self.wait(
                lambda e: e[:2] == (self.device, opcode), "owned selection", start)[2])[0]
            check(offer, "missing compositor selection")
            reader = self.receive(offer, 0)
            try:
                check(self.contents(reader) == b"A" * 262144, "partial selection write corrupted bytes")
            finally:
                os.close(reader)
            # receive also permits a regular-file fd, which epoll cannot
            # monitor. Keep this path working without blocking the event loop.
            fd = os.memfd_create("minde-selection-file", os.MFD_CLOEXEC)
            try:
                self.send(offer, 0, string(MIME), fd=fd)
                self.roundtrip()
                wait_for(lambda: os.fstat(fd).st_size == 262144, "regular-file selection write failed")
                check(os.pread(fd, 262144, 0) == b"A" * 262144, "regular-file payload corrupted")
            finally:
                os.close(fd)
            finish = (lambda: evaluate(f'({setter} "replacement")')) if opcode == 1 else (
                lambda: self.send(self.device, 2, uints(0)))  # external primary clear
            self.flood(offer, 0, finish, setter)
        start = len(self.events)
        evaluate("(wm-set-clipboard (make-string 262144 #\\T))")
        offer = struct.unpack("=I", self.wait(
            lambda e: e[:2] == (self.device, 1), "timed selection", start)[2])[0]
        self.flood(offer, 0, None, "clipboard deadline")
        print("ok - large clipboard and primary text survive partial writes", flush=True)

    def dnd(self):
        self.point()
        start = len(self.events)
        result = ipc(f"(wm-drop-text {self.x} 180 (make-string 262144 #\\D))")
        check(result.startswith("(ok "), f"drag failed: {result}")
        event = self.wait(lambda e: e[:2] == (self.drag_device, 1), "drag enter", start)
        serial, surface, x, y, offer = struct.unpack("=IIiiI", event[2])
        check(surface == self.surface and offer, "drag entered wrong surface")
        self.send(offer, 0, uints(serial) + string(MIME))  # accept
        self.send(offer, 4, uints(1, 1))  # copy supported/preferred
        self.wait(lambda e: e[:2] == (self.drag_device, 4), "drop", start)
        reader = self.receive(offer, 1)
        try:
            check(self.contents(reader) == b"D" * 262144, "drag payload corrupted")
        finally:
            os.close(reader)
        self.flood(offer, 1, lambda: self.send(offer, 3), "drag payload")  # finish
        print("ok - large drag payload survives partial writes", flush=True)

    def paste(self):
        self.keep_fds = True
        source = self.alloc()
        self.send(self.control, 0, uints(source))
        self.send(source, 0, string(MIME))

        def source_event(opcode, data):
            if opcode == 0:  # send
                check(self.incoming, "selection send arrived without a pipe")
                self.writers.append(self.incoming.popleft())

        self.handlers[source] = source_event
        self.send(self.device, 0, uints(source))
        self.roundtrip()
        evaluate("(begin (define transfer-pastes '())"
                 " (define (handle-paste! text) (set! transfer-pastes (cons text transfer-pastes))) #t)")
        baseline, threads = resources()
        evaluate("(begin (do ((i 0 (+ i 1))) ((= i 40)) (wm-request-paste)) #t)")
        self.roundtrip()
        check(len(self.writers) == 40, "owner did not receive every paste request")
        os.write(self.writers[0], b"partial")  # keep ALL write ends open
        check(resources()[0] <= baseline + LIMIT, "paste requests retained too many fds")
        check(resources()[1] <= threads + 2, "paste requests created worker threads")
        wait_for(lambda: ipc("(length transfer-pastes)") == "(ok 40)",
                 "stalled pastes never completed")
        evaluate('(and (= 1 (count (lambda (s) (equal? s "partial")) transfer-pastes))'
                 ' (= 39 (count string-null? transfer-pastes)))')
        wait_for(lambda: resources()[0] <= baseline, "paste deadlines retained read fds", 1)
        for fd in self.writers:
            try:
                os.write(fd, b"late")
            except BrokenPipeError:
                pass
            else:
                raise AssertionError("timed-out paste still has a reader")
            os.close(fd)
        self.writers.clear()
        self.roundtrip()
        evaluate("(= 40 (length transfer-pastes))")
        # A normal EOF still completes promptly after the timeout/cap paths.
        evaluate("(begin (wm-request-paste) #t)")
        self.roundtrip()
        check(len(self.writers) == 1, "normal paste was not requested")
        os.write(self.writers[0], "normal ✓".encode())
        os.close(self.writers.pop())
        try:
            wait_for(lambda: ipc("(length transfer-pastes)") == '(ok 41)', "normal paste did not finish", 1)
            evaluate('(equal? (car transfer-pastes) "normal ✓")')
        except AssertionError:
            print("paste result:", ipc("(list (length transfer-pastes) (car transfer-pastes))"), flush=True)
            raise
        self.keep_fds = False
        print("ok - stalled paste owners release capped fds and deliver partial text exactly once", flush=True)

    def close(self):
        for fd in list(self.incoming) + self.writers:
            os.close(fd)
        super().close()


client = Client()
try:
    for case in os.environ.get("MINDE_TRANSFER_CASES", "clipboard dnd paste").split():
        getattr(client, case)()
    print("selection-transfer: all checks passed", flush=True)
finally:
    client.close()
