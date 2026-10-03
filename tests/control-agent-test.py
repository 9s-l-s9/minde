#!/usr/bin/env python3
"""Offline classifier contracts; stubs are not model-quality evidence."""
import copy
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
loader = importlib.machinery.SourceFileLoader("minde_agent", str(ROOT / "scripts/minde-agent"))
spec = importlib.util.spec_from_loader(loader.name, loader)
agent = importlib.util.module_from_spec(spec)
loader.exec_module(agent)
CORPUS = agent.read_json(ROOT / "tests/fixtures/control-agent/corpus.json")


class AgentTests(unittest.TestCase):
    def setUp(self):
        self.snapshot = copy.deepcopy(CORPUS["snapshot"])
        self.catalog = copy.deepcopy(CORPUS["catalog"])
        self.candidates = agent.candidates_from(self.snapshot, self.catalog, set(agent.SUPPORTED))

    def assert_code(self, code, function, *args, **kwargs):
        with self.assertRaises(agent.AgentError) as caught:
            function(*args, **kwargs)
        self.assertEqual(code, caught.exception.code)

    def response(self, choice="c0", confidence=1):
        options = ["none", "ambiguous"] + [c["choice-id"] for c in self.candidates]
        return {"model": agent.MODEL, "answers": {"selection": {
            "type": "choice", "choice": choice, "confidence": confidence,
            "probabilities": {key: float(key == choice) for key in options}}}}

    def test_synthetic_baseline(self):
        report = agent.evaluate(CORPUS, "deterministic", None)
        self.assertEqual(10, report["metrics"]["correct"])
        self.assertEqual(0, report["metrics"]["wrong-target"])
        self.assertEqual(5, report["metrics"]["abstained"])
        self.assertFalse(report["provider-contact-attempted"])
        self.assertEqual("disabled", report["execution"])

    def test_discovery_controls_candidates(self):
        only = agent.candidates_from(self.snapshot, self.catalog, {"switch-group!"})
        self.assertEqual(2, len(only))
        self.assertTrue(all(c["action"] == "switch-group!" for c in only))
        self.catalog[0]["parameters"][0]["type"] = "string"
        self.assert_code("unsupported-action-schema", agent.candidates_from,
                         self.snapshot, self.catalog, {"focus-window!"})

    def test_invalid_capability_objects_are_rejected(self):
        client = agent.DesktopClient()
        for response in ([], {"schema-version": True, "actions": []},
                         {"schema-version": 1, "actions": ["bad"]},
                         {"schema-version": 1, "actions": [{"name": 1}]}):
            client.call = lambda *args: response
            self.assert_code("unsupported-catalog", client.catalog, agent.SUPPORTED)
        self.snapshot["schema-version"] = True
        self.assert_code("unsupported-snapshot", agent.check_snapshot, self.snapshot)

    def test_actual_registry_json_contract(self):
        source = """(use-modules (minde commands) (minde command-catalog)
                                  (minde control) (minde control-json))
                    (register-builtin-command-schemas!)
                    (register-control-commands!)
                    (display (control-json (capabilities) 'capabilities)) (newline)
                    (display (control-json (describe-action 'focus-window!) 'description)) (newline)
                    (display (control-json (describe-action 'switch-group!) 'description)) (newline)"""
        result = subprocess.run(["guile", "--no-auto-compile", "-L", str(ROOT / "scheme"), "-c", source],
                                capture_output=True, text=True, check=True)
        capabilities, *catalog = map(json.loads, result.stdout.splitlines())
        self.assertEqual(1, capabilities["schema-version"])
        candidates = agent.candidates_from(self.snapshot, catalog, set(agent.SUPPORTED))
        self.assertEqual(len(self.candidates), len(candidates))

    def test_bounds_duplicates_and_redaction(self):
        self.snapshot["windows"] = [dict(self.snapshot["windows"][0], id=i + 1) for i in range(254)]
        self.assert_code("too-many-candidates", agent.candidates_from,
                         self.snapshot, self.catalog, {"focus-window!"})
        self.snapshot["windows"] = [dict(self.snapshot["windows"][0], id=1)] * 2
        self.assert_code("duplicate-desktop-target", agent.candidates_from,
                         self.snapshot, self.catalog, {"focus-window!"})
        self.snapshot["redacted"] = True
        self.assert_code("desktop-unavailable", agent.candidates_from,
                         self.snapshot, self.catalog, {"focus-window!"})

    def test_untrusted_titles_are_data(self):
        payload = agent.jev_request("focus my browser", self.candidates)
        question = payload["questions"]["selection"]
        self.assertEqual({"user_request": "focus my browser"}, payload["state"])
        self.assertIn("untrusted", question["instructions"])
        self.assertEqual(len(self.candidates) + 2, len(question["criteria"]))
        self.assertNotIn("arguments", json.dumps(question))
        self.assertEqual(42, question["criteria"]["c0"]["untrusted_descriptive_data"]["window-id"])
        selected = agent.deterministic_select("focus my browser", self.candidates)["choice"]
        self.assertEqual(42, next(c for c in self.candidates if c["choice-id"] == selected)["arguments"]["window"])

    def test_valid_jev_response_and_threshold(self):
        selected = agent.parse_jev_response(self.response(), self.candidates)
        result = agent.select("focus browser", self.snapshot, self.catalog, "jev", 0.9,
                              selector=lambda *args: selected)
        self.assertEqual("dry-run", result["status"])
        selected["confidence"] = 0.89
        result = agent.select("focus browser", self.snapshot, self.catalog, "jev", 0.9,
                              selector=lambda *args: selected)
        self.assertEqual("low-confidence", result["reason"])
        self.assertNotIn("arguments", result)

    def test_provider_response_cannot_invent_actions(self):
        for field, value in [("choice", "(wm-quit)"), ("confidence", float("nan")),
                             ("confidence", True), ("probabilities", {"c0": 1})]:
            response = self.response()
            response["answers"]["selection"][field] = value
            self.assert_code("malformed-provider-response", agent.parse_jev_response,
                             response, self.candidates)
        response = self.response()
        response["model"] = "unreviewed-model"
        self.assert_code("malformed-provider-response", agent.parse_jev_response, response, self.candidates)
        self.assert_code("unknown-choice", agent.select, "focus browser", self.snapshot,
                         self.catalog, selector=lambda *args: {"choice": "evil", "model": "stub"})

    def test_ties_and_abstentions(self):
        response = self.response()
        response["answers"]["selection"]["probabilities"].update({"c0": 0.5, "c1": 0.5})
        self.assertEqual("ambiguous", agent.parse_jev_response(response, self.candidates)["choice"])
        for choice in ("none", "ambiguous"):
            result = agent.select("anything", self.snapshot, self.catalog, "jev", 0.8,
                                  selector=lambda *args: {"choice": choice, "confidence": 1, "model": agent.MODEL})
            self.assertEqual(choice, result["reason"])

    def test_http_shape_and_timeout_no_retry(self):
        calls = []

        def opener(request, timeout):
            calls.append((request, timeout))
            return io.BytesIO(json.dumps(self.response()).encode())

        with patch.dict(os.environ, {"TYPESAFE_API_KEY": "synthetic-test-key"}):
            agent.jev_select("focus browser", self.candidates, 0.8, opener=opener)
            self.assertEqual(agent.ENDPOINT, calls[0][0].full_url)
            self.assertEqual(agent.MODEL, json.loads(calls[0][0].data)["model"])
            self.assertEqual(10, calls[0][1])
            self.assert_code("provider-unavailable", agent.jev_select, "focus browser",
                             self.candidates, 0.8,
                             opener=lambda *args, **kwargs: (_ for _ in ()).throw(TimeoutError()))
        with patch.dict(os.environ, {}, clear=True):
            self.assert_code("missing-api-key", agent.jev_select, "focus browser", self.candidates, 0.8)
        self.assert_code("threshold-required", agent.jev_select, "focus browser", self.candidates, None)

    def test_local_transport_no_credentials_or_remote_host(self):
        def opener(request, timeout):
            self.assertIsNone(request.get_header("Authorization"))
            self.assertEqual("http://127.0.0.1:8123/select", request.full_url)
            return io.BytesIO(json.dumps(self.response()).encode())

        answer = agent.local_select("focus browser", self.candidates, 0.8,
                                    "http://127.0.0.1:8123/select", agent.MODEL, opener=opener)
        self.assertEqual("c0", answer["choice"])
        for endpoint in ("https://example.com", "http://evil.test", "http://user:pass@localhost"):
            self.assert_code("invalid-local-endpoint", agent.local_select, "request",
                             self.candidates, 0.8, endpoint, "model", opener=opener)

    def test_json_rejects_duplicates_and_nonfinite_values(self):
        for text in ('{"x":1,"x":2}', '{"x":NaN}', '{"x":1e999}', "[", "x" * (agent.MAX_JSON + 1)):
            with self.assertRaises(agent.AgentError):
                agent.load_json(text)

    def test_execute_requires_allowlist(self):
        self.assert_code("execution-allowlist-required", agent.select, "focus browser",
                         self.snapshot, self.catalog, execute=True)

    def test_execution_uses_receipt_and_snapshot_preconditions(self):
        client = agent.DesktopClient()
        calls = []

        def call(*args):
            calls.append(args)
            if args[0] == "request":
                return {"session": "synthetic-session", "revision": 7, "request-id": "r1"}
            return {"status": "applied", "session": "synthetic-session", "request-id": "r1"}

        client.call = call
        result = agent.select("focus browser", self.snapshot, self.catalog,
                              allowed={"focus-window!"}, execute=True, client=client)
        self.assertEqual("applied", result["result"]["status"])
        self.assertEqual(("call", "focus-window!", "((window . 42))", "--session", "synthetic-session",
                          "--revision", "7", "--request-id", "r1"), calls[1])
        client.call = lambda *args: {"session": "synthetic-session", "revision": 8, "request-id": "r2"}
        self.assert_code("stale-state", client.execute, self.candidates[0], self.snapshot)

    def test_transport_unknown_retains_receipt(self):
        client = agent.DesktopClient()
        calls = []

        def call(*args):
            calls.append(args)
            if args[0] == "request":
                return {"session": "synthetic-session", "revision": 7, "request-id": "r1"}
            raise agent.AgentError("transport-outcome-unknown")

        client.call = call
        result = client.execute(self.candidates[0], self.snapshot)
        self.assertEqual("outcome-unknown", result["status"])
        self.assertEqual("r1", result["request-id"])
        self.assertEqual(2, len(calls))

    def test_disappearing_target_is_reported_from_shared_executor(self):
        client = agent.DesktopClient()
        client.call = lambda *args: ({"session": "synthetic-session", "revision": 7, "request-id": "r1"}
                                    if args[0] == "request" else
                                    {"session": "synthetic-session", "request-id": "r1", "status": "error",
                                     "error": {"code": "unknown-window", "stage": "validation"}})
        self.assertEqual("unknown-window", client.execute(self.candidates[0], self.snapshot)["error"]["code"])

    def test_group_string_is_data_not_code(self):
        text = 'work\"); (wm-quit) ; "λ\n'
        encoded = agent.scheme_string(text)
        result = subprocess.run(["guile", "--no-auto-compile", "-c",
                                 "(let ((v (read))) (write (string? v)))"],
                                input=encoded, text=True, capture_output=True, check=True)
        self.assertEqual("#t", result.stdout)

    def test_cli_evaluation_writes_metric_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "metrics.json"
            reply = subprocess.run([str(ROOT / "scripts/minde-agent"), "evaluate", "--corpus",
                                    str(ROOT / "tests/fixtures/control-agent/corpus.json"),
                                    "--output", str(output)], capture_output=True, check=True)
            self.assertEqual(json.loads(reply.stdout), json.loads(output.read_text()))


if __name__ == "__main__":
    unittest.main()
