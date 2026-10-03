#!/usr/bin/env python3
"""Real Unix socket tests for mindectl, without a running compositor."""
import json
import os
import pty
from pathlib import Path
import socket
import select
import subprocess
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "scripts/mindectl"


class ClientTests(unittest.TestCase):
    def run_client(self, args, replies, stdin=None, interactive=False):
        with tempfile.TemporaryDirectory(prefix="minde-client-") as runtime:
            listener = socket.socket(socket.AF_UNIX)
            listener.bind(runtime + "/minde-ipc.sock")
            listener.listen(8)
            listener.settimeout(4)
            requests = []
            errors = []

            def serve():
                try:
                    for reply in replies:
                        conn, _ = listener.accept()
                        with conn:
                            chunks = []
                            while True:
                                chunk = conn.recv(8192)
                                if not chunk:
                                    break
                                chunks.append(chunk)
                            requests.append(b"".join(chunks).decode())
                            if isinstance(reply, tuple):
                                payload, delay = reply
                                time.sleep(delay)
                            else:
                                payload = reply
                            try:
                                conn.sendall(payload.encode())
                            except (BrokenPipeError, ConnectionResetError):
                                pass
                except Exception as error:
                    errors.append(error)
                finally:
                    listener.close()

            worker = threading.Thread(target=serve, daemon=True)
            worker.start()
            environment = dict(os.environ, XDG_RUNTIME_DIR=runtime,
                               XDG_STATE_HOME=runtime, GUILE_AUTO_COMPILE="0")
            if interactive:
                master, slave = pty.openpty()
                child = subprocess.Popen([str(CLI), *args], stdin=slave, stdout=slave,
                                         stderr=subprocess.PIPE, env=environment)
                os.close(slave)
                output = bytearray()
                end = time.monotonic() + 5
                while b"minde> " not in output and time.monotonic() < end:
                    if select.select([master], [], [], 0.05)[0]:
                        output.extend(os.read(master, 8192))
                self.assertIn(b"minde> ", output)
                os.write(master, stdin.encode())
                while child.poll() is None and time.monotonic() < end:
                    if select.select([master], [], [], 0.05)[0]:
                        try:
                            output.extend(os.read(master, 8192))
                        except OSError:
                            break
                _, stderr = child.communicate(timeout=2)
                os.close(master)
                process = subprocess.CompletedProcess(child.args, child.returncode,
                                                      output.decode(), stderr.decode())
                history = Path(runtime) / "minde/repl-history"
                self.assertTrue(history.is_file())
                self.assertEqual(history.stat().st_mode & 0o777, 0o600)
                self.assertIn("focus-window!", history.read_text())
            else:
                process = subprocess.run([str(CLI), *args], text=True,
                                         input=stdin, capture_output=True,
                                         env=environment, timeout=8)
            worker.join(5)
            self.assertFalse(worker.is_alive(), "client never reached the expected exchange")
            self.assertFalse(errors, errors)
            return process, requests

    def test_legacy_and_machine_success(self):
        result, requests = self.run_client(["eval", '(list "#< λ" 1)'], ['(ok ("#< λ" 1))'])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, '("#< λ" 1)\n')
        result, _ = self.run_client(["eval", "(+ 1 2)", "--machine"], ["(ok 3)"])
        self.assertEqual(result.stdout, "(ok 3)\n")

    def test_complete_error_and_optional_backtrace(self):
        reply = '(error oops () "failure" "TRACE" ((stage . execution)))'
        result, _ = self.run_client(["eval", "(error)", "--scheme"], [reply])
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, reply + "\n")
        result, _ = self.run_client(["eval", "(error)"], [reply])
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn("failure", result.stderr)
        self.assertNotIn("TRACE", result.stderr)
        result, _ = self.run_client(["eval", "(error)", "--backtrace"], [reply])
        self.assertIn("TRACE", result.stderr)

    def test_schema_json_and_raw_fallback(self):
        snapshot = '(ok ((schema-version . 1) (session . "s") (windows . #()) (groups . #()) (outputs . #()) (layouts . #("wide")) (focused-window . null)))'
        result, _ = self.run_client(["query", "desktop", "--json"], [snapshot])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["windows"], [])
        self.assertIsNone(json.loads(result.stdout)["focused-window"])
        result, _ = self.run_client(["eval", "'((a . 1))", "--json"], ["(ok ((a . 1)))"])
        self.assertEqual(json.loads(result.stdout), {"$scheme": "((a . 1))"})
        result, _ = self.run_client(["help", "focus-window!", "--json"],
                                  ['(ok ((name . focus-window!) (parameters . (((name . direction) (type . (enum horizontal vertical))))) (examples . ((focus-window! 42)))))'])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["parameters"][0]["type"],
                         ["enum", "horizontal", "vertical"])

    def test_allocation_and_quoted_call(self):
        allocation = '(ok ((session . "s") (revision . 7) (request-id . "s:1")))'
        result = '(ok ((schema-version . 1) (session . "s") (revision . 8) (request-id . "s:1") (status . applied) (value . ((focused-window . 42)))))'
        process, requests = self.run_client(["call", "focus-window!", "((window . 42))", "--json"],
                                            [allocation, result])
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(requests[0], "(new-action-request)")
        self.assertIn('(quote ((window . 42)))', requests[1])
        self.assertIn('#:session "s"', requests[1])
        self.assertIn('#:if-revision 7', requests[1])
        self.assertIn('#:request-id "s:1"', requests[1])
        self.assertEqual(json.loads(process.stdout)["status"], "applied")

    def test_explicit_receipt_not_allocated_again(self):
        process, requests = self.run_client(
            ["call", "focus-window!", "((window . 42))", "--session", "s", "--revision", "7",
             "--request-id", "s:1", "--scheme"],
            ['(ok ((status . error) (error . ((code . stale-state)))))'])
        self.assertEqual(process.returncode, 1)
        self.assertEqual(len(requests), 1)
        self.assertIn("perform-action!", requests[0])

    def test_wait_polls_without_repeating_action(self):
        process, requests = self.run_client(
            ["wait", "s:1", "--session", "s", "--json"],
            ['(ok ((status . pending) (value . null)))', '(ok ((status . applied) (value . null)))'])
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(json.loads(process.stdout)["status"], "applied")
        self.assertTrue(all(request == '(action-status "s:1" #:session "s")' for request in requests))

    def test_missing_incomplete_and_multiple_replies(self):
        for reply in ["", "(ok", "(ok 1) (ok 2)", "#.(+ 1 2)", '(error bad () "bad" "" 42)']:
            with self.subTest(reply=reply):
                process, requests = self.run_client(["eval", "(set! x 1)", "--json"], [reply])
                self.assertEqual(process.returncode, 3, process.stderr)
                error = json.loads(process.stdout)["error"]
                self.assertEqual(error["metadata"]["outcome"], "unknown")
                self.assertEqual(len(requests), 1)

    def test_machine_validation_errors_precede_connections(self):
        for args in [["call", "focus-window!", "#.(error)", "--json"],
                     ["eval", "1", "--request-id", "s:1", "--json"]]:
            with self.subTest(args=args):
                process, requests = self.run_client(args, [])
                self.assertEqual(process.returncode, 2, process.stderr)
                self.assertEqual(requests, [])
                error = json.loads(process.stdout)["error"]
                self.assertEqual(error["metadata"]["stage"], "validation")
                self.assertIs(error["metadata"]["execution-started?"], False)

    def test_read_deadline_and_reply_limit(self):
        process, requests = self.run_client(["eval", "1", "--timeout-ms", "50", "--json"],
                                            [("(ok 1)", 0.2)])
        self.assertEqual(process.returncode, 3, process.stderr)
        self.assertEqual(json.loads(process.stdout)["error"]["code"], "timeout")
        self.assertEqual(len(requests), 1)
        process, _ = self.run_client(["eval", "1", "--json"], ["x" * (1024 * 1024 + 1)])
        self.assertEqual(process.returncode, 3, process.stderr)
        self.assertEqual(json.loads(process.stdout)["error"]["code"], "reply-too-large")

    def test_multiline_repl(self):
        process, requests = self.run_client(["repl"],
                                            ['(ok ((actions . ())))', '(ok 3)'],
                                            stdin="\n; comment\n#| block\n comment |#\n(+ 1\n 2)\n,quit\n")
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertEqual(process.stdout, "3\n")
        self.assertIn("(+ 1\n 2)", requests[1])

    def test_interactive_completion_and_private_history(self):
        process, requests = self.run_client(
            ["repl"], ['(ok ((actions . (((name . focus-window!))))))', '(ok 42)'],
            stdin="(focus-wi\t 42)\n,quit\n", interactive=True)
        self.assertEqual(process.returncode, 0, process.stderr)
        self.assertIn("focus-window!", requests[1])

    def test_unknown_outcome_retains_allocated_receipt(self):
        allocation = '(ok ((session . "s") (revision . 7) (request-id . "s:1")))'
        process, requests = self.run_client(
            ["call", "focus-window!", "((window . 42))", "--json"], [allocation, ""])
        self.assertEqual(process.returncode, 3, process.stderr)
        metadata = json.loads(process.stdout)["error"]["metadata"]
        self.assertEqual(metadata["request-id"], "s:1")
        self.assertEqual(metadata["session"], "s")
        self.assertEqual(metadata["outcome"], "unknown")
        self.assertEqual(len(requests), 2)

    def test_watch_snapshot_handoff_and_redacted_watermark(self):
        process, requests = self.run_client(
            ["watch", "--json", "--count", "3"],
            ['(ok ((session . "s") (event-sequence . 10) (windows . #())))',
             '(ok ((session . "s") (sequence . 12) (gap . #f) (events . #())))',
             '(ok ((session . "s") (sequence . 13) (gap . #f) (events . #(((sequence . 13) (name . focus-window) (arguments . #(42)))))))'])
        self.assertEqual(process.returncode, 0, process.stderr)
        lines = [json.loads(line) for line in process.stdout.splitlines()]
        self.assertEqual([line["type"] for line in lines], ["desktop", "events", "events"])
        self.assertEqual(requests[1], '(events-since 10 #:session "s")')
        self.assertEqual(requests[2], '(events-since 12 #:session "s")')

    def test_watch_resnapshots_on_gap_session_change_and_disconnect(self):
        for middle in ['(ok ((session . "s") (sequence . 99) (gap . #t) (events . #())))',
                       '(ok ((session . "new") (sequence . 1) (gap . #f) (events . #())))', ""]:
            with self.subTest(middle=middle):
                process, requests = self.run_client(
                    ["watch", "--json", "--count", "2"],
                    ['(ok ((session . "s") (event-sequence . 10) (windows . #())))', middle,
                     '(ok ((session . "new") (event-sequence . 1) (windows . #())))'])
                self.assertEqual(process.returncode, 0, process.stderr)
                self.assertEqual(requests[-1], "(desktop-snapshot)")
                lines = [json.loads(line) for line in process.stdout.splitlines()]
                self.assertEqual([line["type"] for line in lines], ["desktop", "desktop"])
                self.assertEqual(lines[-1]["data"]["session"], "new")


if __name__ == "__main__":
    unittest.main()
