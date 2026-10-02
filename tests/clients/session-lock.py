#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Session-lock security regression client; Python standard library only.

Use only through session-lock-e2e.sh, which owns a private Xvfb display.
Deliberately invalid requests must kill only the attacking client. Every
Wayland wait has a deadline; input checks wait for delivery to its intended
recipient before roundtripping the clients that must not receive it.
"""
import array
import ctypes
import ctypes.util
import os
import select
import socket
import struct
import subprocess
import time

TIMEOUT = 5
RUNTIME = os.environ["XDG_RUNTIME_DIR"]
WAYLAND = os.path.join(RUNTIME, os.environ["NESTED_WAYLAND_DISPLAY"])


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def uints(*values):
    return struct.pack("=" + "I" * len(values), *values)


def string(value):
    data = value.encode() + b"\0"
    return uints(len(data)) + data + b"\0" * (-len(data) % 4)


def ipc(expression):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.settimeout(TIMEOUT)
        sock.connect(os.path.join(RUNTIME, "minde-ipc.sock"))
        sock.sendall(expression.encode())
        sock.shutdown(socket.SHUT_WR)
        chunks = []
        while chunk := sock.recv(65536):
            chunks.append(chunk)
        return b"".join(chunks).decode().strip()


def locked(expected=True):
    result = ipc("(wm-session-locked?)")
    check(result == "(ok #t)" if expected else result == "(ok #f)",
          f"session locked state: expected {expected}, received {result}")


class ProtocolError(RuntimeError):
    pass


class Client:
    def __init__(self, name):
        self.name = name
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(TIMEOUT)
        self.sock.connect(WAYLAND)
        self.buffer = b""
        self.next_id = 2
        self.globals = {}
        self.events = []
        self.handlers = {}
        self.keyboard = None
        self.registry = self.alloc()
        self.send(1, 1, uints(self.registry))
        self.roundtrip()

    def close(self):
        self.sock.close()

    def alloc(self):
        obj = self.next_id
        self.next_id += 1
        return obj

    def send(self, obj, opcode, payload=b"", fd=None):
        message = uints(obj, ((8 + len(payload)) << 16) | opcode) + payload
        if fd is None:
            self.sock.sendall(message)
        else:
            sent = self.sock.sendmsg([message], [(socket.SOL_SOCKET, socket.SCM_RIGHTS,
                                                 array.array("i", [fd]))])
            if sent < len(message):
                self.sock.sendall(message[sent:])

    def send_batch(self, requests):
        self.sock.sendall(b"".join(uints(obj, ((8 + len(payload)) << 16) | opcode) + payload
                                  for obj, opcode, payload in requests))

    def read(self, deadline):
        while True:
            if len(self.buffer) >= 8:
                obj, header = struct.unpack_from("=II", self.buffer)
                size, opcode = header >> 16, header & 65535
                check(size >= 8 and size % 4 == 0, "malformed Wayland message")
                if len(self.buffer) >= size:
                    data, self.buffer = self.buffer[8:size], self.buffer[size:]
                    break
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.sock], [], [], remaining)[0]:
                raise TimeoutError(f"{self.name}: Wayland event deadline exceeded")
            data, ancillary, flags, _ = self.sock.recvmsg(65536, socket.CMSG_SPACE(4096))
            check(not flags & socket.MSG_CTRUNC, "truncated Wayland ancillary data")
            for level, kind, content in ancillary:
                if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                    fds = array.array("i")
                    fds.frombytes(content[:len(content) // fds.itemsize * fds.itemsize])
                    for fd in fds:
                        os.close(fd)
            if not data:
                raise ConnectionError(f"{self.name}: Wayland connection closed")
            self.buffer += data
        if obj == 1 and opcode == 0:
            object_id, code, length = struct.unpack_from("=III", data)
            message = data[12:12 + length - 1].decode(errors="replace")
            raise ProtocolError(f"{self.name}: object {object_id}, code {code}: {message}")
        if obj == self.registry and opcode == 0:
            name, length = struct.unpack_from("=II", data)
            interface = data[8:8 + length - 1].decode()
            version = struct.unpack_from("=I", data, 8 + (length + 3) // 4 * 4)[0]
            self.globals[interface] = (name, version)
        event = (obj, opcode, data)
        self.events.append(event)
        if obj in self.handlers:
            self.handlers[obj](opcode, data)
        return event

    def wait(self, predicate, message, start=0):
        deadline = time.monotonic() + TIMEOUT
        while True:
            for event in self.events[start:]:
                if predicate(event):
                    return event
            start = len(self.events)
            try:
                self.read(deadline)
            except TimeoutError as error:
                raise TimeoutError(f"{self.name}: waiting for {message}") from error

    def roundtrip(self):
        callback = self.alloc()
        self.send(1, 0, uints(callback))
        self.wait(lambda e: e[:2] == (callback, 0), "sync callback")

    def bind(self, interface, version=1):
        check(interface in self.globals, f"missing {interface} global")
        name, available = self.globals[interface]
        obj = self.alloc()
        self.send(self.registry, 0, uints(name) + string(interface)
                  + uints(min(version, available), obj))
        return obj

    def get_keyboard(self):
        if self.keyboard is None:
            seat = self.bind("wl_seat")
            self.keyboard = self.alloc()
            self.send(seat, 1, uints(self.keyboard))
        return self.keyboard

    def new_surface(self):
        compositor = self.bind("wl_compositor")
        surface = self.alloc()
        self.send(compositor, 0, uints(surface))
        return surface

    def buffer_for(self, width, height, color):
        shm = self.bind("wl_shm")
        pool, buffer = self.alloc(), self.alloc()
        fd = os.memfd_create("minde-session-lock-e2e", os.MFD_CLOEXEC)
        try:
            pixels = struct.pack("=I", color) * (width * height)
            os.ftruncate(fd, len(pixels))
            view = memoryview(pixels)
            while view:
                count = os.write(fd, view)
                view = view[count:]
            self.send(shm, 0, uints(pool, len(pixels)), fd=fd)
            # wl_shm.format.xrgb8888, offset zero and tightly packed rows.
            self.send(pool, 0, uints(buffer, 0, width, height, width * 4, 1))
            self.send(pool, 1)
        finally:
            os.close(fd)
        return buffer

    def paint(self, surface, width, height, color):
        buffer = self.buffer_for(width, height, color)
        self.send(surface, 1, uints(buffer, 0, 0))
        self.send(surface, 2, uints(0, 0, width, height))
        self.send(surface, 6)

    def toplevel(self):
        self.get_keyboard()
        self.surface = self.new_surface()
        shell = self.bind("xdg_wm_base")
        self.handlers[shell] = lambda op, data: self.send(shell, 3, data) if op == 0 else None
        xdg_surface, toplevel = self.alloc(), self.alloc()
        self.send(shell, 2, uints(xdg_surface, self.surface))
        self.send(xdg_surface, 1, uints(toplevel))
        self.send(toplevel, 2, string(self.name))
        size = [640, 480]

        def toplevel_event(opcode, data):
            if opcode == 0:
                width, height = struct.unpack_from("=ii", data)
                size[:] = [width or size[0], height or size[1]]

        def configure(opcode, data):
            if opcode == 0:
                self.send(xdg_surface, 4, data)
                self.paint(self.surface, *size, 0x00FF00FF)

        self.handlers[toplevel] = toplevel_event
        self.handlers[xdg_surface] = configure
        self.send(self.surface, 6)
        self.wait(lambda e: e[:2] == (xdg_surface, 0), "xdg configure")
        self.roundtrip()

    def request_lock(self):
        manager = self.bind("ext_session_lock_manager_v1")
        lock = self.alloc()
        self.send(manager, 1, uints(lock))
        return lock

    def accepted_lock(self):
        lock = self.request_lock()
        event = self.wait(lambda e: e[0] == lock and e[1] in (0, 1), "lock result")
        check(event[1] == 0, f"{self.name}: legitimate lock denied")
        return lock

    def denied_lock(self):
        lock = self.request_lock()
        event = self.wait(lambda e: e[0] == lock and e[1] in (0, 1), "denied lock result")
        check(event[1] == 1, f"{self.name}: concurrent lock replaced live owner")
        self.roundtrip()
        check(not any(e[:2] == (lock, 0) for e in self.events), "denied lock received locked")
        return lock

    def lock_surface(self, lock, commit=True):
        self.get_keyboard()
        self.surface = self.new_surface()
        output = self.bind("wl_output")
        lock_surface = self.alloc()
        self.send(lock, 1, uints(lock_surface, self.surface, output))
        if commit:
            _, _, data = self.wait(lambda e: e[:2] == (lock_surface, 0), "lock configure")
            serial, width, height = struct.unpack_from("=III", data)
            check(width > 0 and height > 0, "invalid lock surface dimensions")
            self.send(lock_surface, 1, uints(serial))
            self.paint(self.surface, width, height, 0x00004080)
            self.roundtrip()
        return lock_surface

    def keyboard_events(self, start=0, opcodes=(1, 3)):
        return [e for e in self.events[start:] if e[0] == self.keyboard and e[1] in opcodes]

    def expect_rejected_request(self):
        # Both a protocol error and an explicit disconnect reject the request.
        # A successful sync would show the malicious request was allowed through.
        try:
            self.roundtrip()
        except (ProtocolError, ConnectionError, BrokenPipeError):
            return
        raise AssertionError(f"{self.name}: malicious request was not rejected")


def keyboard_details(events):
    return [{"opcode": opcode, "words": struct.unpack("=" + "I" * (len(data) // 4), data)}
            for _, opcode, data in events]


def inject_key(recipient, *excluded):
    # Drain events queued by earlier unlock/relock transitions before setting
    # this injection's baseline. Then verify no excluded client retains focus.
    recipient.roundtrip()
    for client in excluded:
        client.roundtrip()
        focus_events = client.keyboard_events(0, (1, 2))
        check(not focus_events or focus_events[-1][1] == 2,
              f"{client.name}: ordinary client retains keyboard focus while locked: "
              f"{keyboard_details(focus_events[-2:])}")
    start = len(recipient.events)
    excluded_starts = [len(client.events) for client in excluded]
    subprocess.run(["xdotool", "key", "--clearmodifiers", "a"], check=True, timeout=TIMEOUT)
    # XKB evdev keycode KEY_A is 30. Wait for both press and release.
    recipient.wait(lambda e: e[0] == recipient.keyboard and e[1] == 3
                   and struct.unpack_from("=IIII", e[2])[2:] == (30, 0),
                   "key release on intended surface", start)
    keys = recipient.keyboard_events(start, (3,))
    states = [struct.unpack_from("=IIII", e[2])[2:] for e in keys]
    check((30, 1) in states and (30, 0) in states,
          f"{recipient.name}: incomplete test key: {states}")
    for client, excluded_start in zip(excluded, excluded_starts):
        client.roundtrip()
        unexpected = client.keyboard_events(excluded_start)
        check(not unexpected,
              f"{client.name}: ordinary client received keyboard events while locked: "
              f"{keyboard_details(unexpected)}")


def restored_focus(clients, starts):
    for client, start in zip(clients, starts):
        client.roundtrip()
        focus_events = client.keyboard_events(start, (1, 2))
        if focus_events and focus_events[-1][1] == 1:
            return client
    raise AssertionError("owner unlock failed to restore ordinary keyboard focus")


class HostPixels:
    """Read the private nested window through Xlib without image dependencies."""
    def __init__(self):
        self.lib = ctypes.CDLL(ctypes.util.find_library("X11") or "libX11.so.6")
        self.lib.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.lib.XOpenDisplay.restype = ctypes.c_void_p
        self.lib.XGetGeometry.argtypes = [ctypes.c_void_p, ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int),
            ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_uint),
            ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint),
            ctypes.POINTER(ctypes.c_uint)]
        self.lib.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong,
            ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint,
            ctypes.c_ulong, ctypes.c_int]
        self.lib.XGetImage.restype = ctypes.c_void_p
        self.lib.XGetPixel.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int]
        self.lib.XGetPixel.restype = ctypes.c_ulong
        self.lib.XDestroyImage.argtypes = [ctypes.c_void_p]
        self.lib.XCloseDisplay.argtypes = [ctypes.c_void_p]
        self.display = self.lib.XOpenDisplay(os.environ["DISPLAY"].encode())
        check(self.display, "could not connect to private Xvfb")
        self.window = int(os.environ["MINDE_SESSION_LOCK_TEST_WINDOW"])

    def samples(self):
        root = ctypes.c_ulong()
        x, y = ctypes.c_int(), ctypes.c_int()
        width, height, border, depth = (ctypes.c_uint() for _ in range(4))
        check(self.lib.XGetGeometry(self.display, self.window, ctypes.byref(root),
              ctypes.byref(x), ctypes.byref(y), ctypes.byref(width), ctypes.byref(height),
              ctypes.byref(border), ctypes.byref(depth)), "nested window geometry unavailable")
        image = self.lib.XGetImage(self.display, self.window, 0, 0,
                                  width.value, height.value, ctypes.c_ulong(-1).value, 2)
        check(image, "nested framebuffer unavailable")
        try:
            return [self.lib.XGetPixel(image, width.value * i // 4, height.value * j // 4)
                    & 0xFFFFFF for i in (1, 2, 3) for j in (1, 2, 3)]
        finally:
            self.lib.XDestroyImage(image)

    def wait_for_black(self):
        deadline = time.monotonic() + TIMEOUT
        while True:
            if all(pixel == 0 for pixel in self.samples()):
                return
            check(time.monotonic() < deadline, "destroyed lock surface did not produce a black frame")
            time.sleep(0.01)

    def wait_for_desktop(self):
        deadline = time.monotonic() + TIMEOUT
        while True:
            if 0xFF00FF in self.samples():
                return
            check(time.monotonic() < deadline, "painted desktop never reached nested framebuffer")
            # Poll a real observable state rather than assuming frame timing.
            time.sleep(0.01)

    def close(self):
        self.lib.XCloseDisplay(self.display)


def main():
    clients = []
    pixels = HostPixels()

    def client(name):
        result = Client(name)
        clients.append(result)
        return result

    try:
        locked(False)
        desktop = client("desktop before lock")
        desktop.toplevel()
        desktop.wait(lambda e: e[0] == desktop.keyboard and e[1] == 1, "initial keyboard enter")
        inject_key(desktop)
        pixels.wait_for_desktop()

        owner = client("active lock owner")
        lock = owner.accepted_lock()
        # Check the framebuffer as soon as locked is received, before making
        # another compositor roundtrip or creating the owner's lock surface.
        check(all(pixel == 0 for pixel in pixels.samples()),
              "locked confirmation arrived while desktop pixels remained visible")
        locked()
        desktop.roundtrip()
        print("ok - lock confirmation observes a cleared nested framebuffer", flush=True)

        surface_attacker = client("denied lock surface attacker")
        denied = surface_attacker.denied_lock()
        denied_surface = surface_attacker.lock_surface(denied, commit=False)
        # GetLockSurface may legally be pipelined before finished arrives.
        # An inert protocol object keeps that path safe without disconnecting
        # the client; it must not configure, reserve the output, or get focus.
        surface_attacker.roundtrip()
        check(not any(e[:2] == (denied_surface, 0) for e in surface_attacker.events),
              "denied lock surface received configure")
        check(not surface_attacker.keyboard_events(), "denied lock surface received keyboard focus")
        locked()
        # A denied surface must not reserve the output or steal active focus.
        owner.lock_surface(lock)
        owner.wait(lambda e: e[0] == owner.keyboard and e[1] == 1, "lock keyboard enter")
        inject_key(owner, desktop, surface_attacker)
        print("ok - live owner cannot be replaced; denied surface cannot steal input", flush=True)

        # Ownership belongs to the lock object, not merely its Wayland
        # connection. A second lock object on the owner's own connection is
        # still denied and must not create a competing lock surface.
        active_surface = owner.surface
        same_client_denied = owner.denied_lock()
        keyboard_start = len(owner.events)
        same_client_surface = owner.lock_surface(same_client_denied, commit=False)
        owner.roundtrip()
        check(not any(e[:2] == (same_client_surface, 0) for e in owner.events),
              "same-client denied lock surface received configure")
        check(not owner.keyboard_events(keyboard_start, (1,)),
              "same-client denied lock surface stole keyboard focus")
        owner.surface = active_surface
        # Destroying the denied object is legal and must leave the genuine
        # owner's lock and its output registration intact.
        owner.send(same_client_denied, 0)
        owner.roundtrip()
        locked()
        inject_key(owner, desktop, surface_attacker)
        print("ok - second lock object on owner's connection has no ownership", flush=True)

        ordinary = client("ordinary toplevel created while locked")
        ordinary.toplevel()
        check(not ordinary.keyboard_events(), "new ordinary toplevel received keyboard enter while locked")
        inject_key(owner, desktop, ordinary, surface_attacker)
        print("ok - mapped ordinary toplevel receives no lock-screen keyboard input", flush=True)

        ime = client("input method attempting keyboard grab while locked")
        ime_manager = ime.bind("zwp_input_method_manager_v2")
        ime_seat = ime.bind("wl_seat")
        input_method = ime.alloc()
        ime.send(ime_manager, 0, uints(ime_seat, input_method))
        ime.roundtrip()
        check(not any(e[:2] == (input_method, 6) for e in ime.events),
              "isolated input-method instance unexpectedly unavailable")
        ime_grab = ime.alloc()
        ime.send(input_method, 5, uints(ime_grab))  # grab_keyboard
        ime.roundtrip()
        check(any(e[:2] == (ime_grab, 0) for e in ime.events),
              "input-method grab was not initialized with a keymap")
        ime_start = len(ime.events)
        inject_key(owner, desktop, ordinary, surface_attacker)
        ime.roundtrip()
        ime_keys = [e for e in ime.events[ime_start:] if e[:2] == (ime_grab, 1)]
        check(not ime_keys,
              "input-method grab captured lock-screen keys: "
              f"{keyboard_details(ime_keys)}")
        locked()
        ime.send(ime_grab, 0)  # release
        ime.send(input_method, 6)  # destroy
        ime.roundtrip()
        inject_key(owner, desktop, ordinary, surface_attacker)
        print("ok - IME keyboard grab cannot capture lock-screen input", flush=True)

        unlock_attacker = client("denied lock unlock attacker")
        denied = unlock_attacker.denied_lock()
        unlock_attacker.send(denied, 2)
        unlock_attacker.expect_rejected_request()
        locked()
        owner.roundtrip()
        inject_key(owner, desktop, ordinary, surface_attacker)
        print("ok - denied unlock cannot unlock another client's session", flush=True)

        restore_starts = [len(c.events) for c in (desktop, ordinary)]
        owner.send(lock, 2)
        owner.roundtrip()
        locked(False)
        normal = restored_focus((desktop, ordinary), restore_starts)
        inject_key(normal)
        print("ok - owner unlock restores ordinary keyboard input", flush=True)

        surface_owner = client("owner destroying focused wl_surface")
        surface_lock = surface_owner.accepted_lock()
        surface_owner.lock_surface(surface_lock)
        inject_key(surface_owner, desktop, ordinary, surface_attacker)
        surface_owner.send(surface_owner.surface, 0)  # wl_surface.destroy
        surface_owner.roundtrip()  # Connection and owning lock remain alive.
        locked()
        pixels.wait_for_black()
        destruction_start = len(surface_owner.events)
        ordinary_starts = [len(c.events) for c in (desktop, ordinary)]
        subprocess.run(["xdotool", "key", "--clearmodifiers", "a"],
                       check=True, timeout=TIMEOUT)
        surface_owner.roundtrip()
        locked()
        for ordinary_client, start in zip((desktop, ordinary), ordinary_starts):
            ordinary_client.roundtrip()
            check(not ordinary_client.keyboard_events(start),
                  "destroyed focused lock surface exposed ordinary input: "
                  f"{keyboard_details(ordinary_client.keyboard_events(start))}")
        check(not surface_owner.keyboard_events(destruction_start),
              "destroyed lock surface received stale keyboard events: "
              f"{keyboard_details(surface_owner.keyboard_events(destruction_start))}")
        restore_starts = [len(c.events) for c in (desktop, ordinary)]
        surface_owner.send(surface_lock, 2)
        surface_owner.roundtrip()
        locked(False)
        normal = restored_focus((desktop, ordinary), restore_starts)
        inject_key(normal)
        print("ok - focused wl_surface destruction keeps live owner locked and able to unlock", flush=True)

        abandoned_owner = client("owner that disconnects")
        abandoned_lock = abandoned_owner.accepted_lock()
        abandoned_owner.lock_surface(abandoned_lock)
        for ordinary_client in (desktop, ordinary):
            ordinary_client.roundtrip()
        abandoned_starts = [len(c.events) for c in (desktop, ordinary)]
        abandoned_owner.close()
        # A separate connection's sync plus IPC services the close without
        # assuming a fixed amount of time has elapsed.
        ordinary.roundtrip()
        locked()
        ordinary.roundtrip()
        takeover = client("abandoned lock takeover")
        takeover_lock = takeover.accepted_lock()
        check(all(pixel == 0 for pixel in pixels.samples()),
              "abandoned lock takeover confirmed before a cleared frame")
        locked()
        for ordinary_client, start in zip((desktop, ordinary), abandoned_starts):
            ordinary_client.roundtrip()
            check(not ordinary_client.keyboard_events(start),
                  "abandoned lock restored ordinary focus")
        takeover.lock_surface(takeover_lock)
        inject_key(takeover, desktop, ordinary)
        restore_starts = [len(c.events) for c in (desktop, ordinary)]
        takeover.send(takeover_lock, 2)
        takeover.roundtrip()
        locked(False)
        normal = restored_focus((desktop, ordinary), restore_starts)
        inject_key(normal)
        print("ok - owner death remains locked; replacement owner can unlock", flush=True)

        duplicate_owner = client("duplicate output lock owner")
        duplicate_lock = duplicate_owner.accepted_lock()
        duplicate_owner.lock_surface(duplicate_lock)
        # Bind another wl_output object for the *same physical output*.
        # Comparing WlOutput resources alone misses this protocol violation.
        duplicate_owner.lock_surface(duplicate_lock, commit=False)
        duplicate_owner.expect_rejected_request()
        locked()
        replacement = client("replacement after duplicate output violation")
        replacement_lock = replacement.accepted_lock()
        replacement.lock_surface(replacement_lock)
        inject_key(replacement, desktop, ordinary, surface_attacker)
        replacement.send(replacement_lock, 2)
        replacement.roundtrip()
        locked(False)
        print("ok - duplicate physical output through a second wl_output bind is rejected", flush=True)

        same_client_owner = client("same-client unauthorized unlock")
        same_client_lock = same_client_owner.accepted_lock()
        same_client_owner.lock_surface(same_client_lock)
        same_client_denied = same_client_owner.denied_lock()
        # Protocol errors terminate the whole connection, including the
        # genuine owner; the session must remain locked through that death.
        same_client_owner.send(same_client_denied, 2)
        same_client_owner.expect_rejected_request()
        locked()
        same_client_recovery = client("recovery after same-client unlock attack")
        recovery_lock = same_client_recovery.accepted_lock()
        same_client_recovery.lock_surface(recovery_lock)
        inject_key(same_client_recovery, desktop, ordinary, surface_attacker)
        same_client_recovery.send(recovery_lock, 2)
        same_client_recovery.roundtrip()
        locked(False)
        print("ok - denied lock on owner's connection cannot unlock session", flush=True)
        pending_owner = client("owner aborting unconfirmed lock")
        compositor = pending_owner.bind("wl_compositor")
        output = pending_owner.bind("wl_output")
        manager = pending_owner.bind("ext_session_lock_manager_v1")
        pending_owner.roundtrip()
        pending_surface = pending_owner.alloc()
        pending_lock = pending_owner.alloc()
        pending_role = pending_owner.alloc()
        # One write lets the server consume the entire sequence before its
        # deferred frame-confirmation path can announce locked. Destroy is
        # therefore valid for this still-unconfirmed owning lock object.
        pending_owner.send_batch([
            (compositor, 0, uints(pending_surface)),
            (manager, 1, uints(pending_lock)),
            (pending_lock, 1, uints(pending_role, pending_surface, output)),
            (pending_role, 0, b""),
            (pending_surface, 0, b""),
            (pending_lock, 0, b""),
        ])
        pending_owner.roundtrip()
        check(not any(e[:2] == (pending_lock, 0) for e in pending_owner.events),
              "batched aborted lock was confirmed before surface cleanup completed")
        locked()
        retry = pending_owner.denied_lock()
        pending_owner.send(retry, 0)
        pending_owner.roundtrip()
        locked()
        pending_recovery = client("fresh connection after pending-lock abort")
        pending_recovery_lock = pending_recovery.accepted_lock()
        pending_recovery.lock_surface(pending_recovery_lock)
        inject_key(pending_recovery, desktop, ordinary, surface_attacker)
        pending_recovery.send(pending_recovery_lock, 2)
        pending_recovery.roundtrip()
        locked(False)
        print("ok - pending lock abort requires reconnection and fresh owner can recover", flush=True)
        print("session-lock-e2e: all checks passed", flush=True)
    finally:
        for connection in clients:
            connection.close()
        pixels.close()


if __name__ == "__main__":
    main()
