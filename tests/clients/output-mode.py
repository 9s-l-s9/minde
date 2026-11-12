#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Malformed custom refresh rates must disconnect only the requesting client."""
import runpy
from pathlib import Path
import struct

protocol = runpy.run_path(str(Path(__file__).with_name("session-lock.py")))
Client, ProtocolError = protocol["Client"], protocol["ProtocolError"]
check, uints, ipc = protocol["check"], protocol["uints"], protocol["ipc"]

before = ipc("(output-heads)")
for refresh in (-2**31, -1, 0, 2**31 - 1, 60000):
    client = Client(f"custom refresh {refresh}")
    try:
        manager = client.bind("zwlr_output_manager_v1", 4)
        client.roundtrip()
        head = struct.unpack("=I", client.wait(lambda e: e[:2] == (manager, 0), "head")[2])[0]
        serial = struct.unpack("=I", client.wait(lambda e: e[:2] == (manager, 1), "done")[2])[0]
        config, config_head = client.alloc(), client.alloc()
        client.send(manager, 0, uints(config, serial))
        client.send(config, 0, uints(config_head, head))
        # Pipeline the request and apply so an immediate protocol-error close
        # cannot race the second write before we read the expected error.
        client.send_batch([(config_head, 1, struct.pack("=iii", 1280, 800, refresh)),
                           (config, 2, b"")])
        try:
            result = client.wait(lambda e: e[0] == config and e[1] in (0, 1, 2), "apply result")
        except ProtocolError as error:
            check(refresh < 0 and f"object {config_head}, code 3:" in str(error), str(error))
        else:
            check(refresh >= 0, f"negative refresh accepted: {refresh}")
            # winit accepts its current mode; other refreshes fail at the backend.
            check(result[1] == (0 if refresh == 60000 else 1), f"unexpected apply result: {result}")
        check(ipc("(output-heads)") == before, "invalid mode changed the output state")
    finally:
        client.close()
print("ok - custom mode protocol rejects negative refresh and survives integer limits", flush=True)
