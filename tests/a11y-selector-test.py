#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Ambiguous target resolution must fail before any AT-SPI mutation."""
import runpy
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

bridge = runpy.run_path(str(Path(__file__).resolve().parents[1] / "scripts/minde-a11y"))


def accessible(name, role, *children):
    node = Mock()
    node.get_name.return_value = name
    node.get_role_name.return_value = role
    node.get_child_count.return_value = len(children)
    node.get_child_at_index.side_effect = children.__getitem__
    return node


class TargetTest(unittest.TestCase):
    def check_refusal(self, windows, code, within_name=None):
        api = Mock()
        api.init.return_value = 0
        app = accessible("app", "application", *windows)
        app.get_process_id.return_value = 23
        api.get_desktop.return_value = accessible("desktop", "desktop", app)
        args = SimpleNamespace(operation="click", pid=23, title="Properties",
                               role="push-button", name="OK", within_name=within_name, within_role=None)
        with self.assertRaises(bridge["Failure"]) as caught:
            bridge["request"](api, args)
        self.assertEqual(caught.exception.code, code)
        api.Action.do_action.assert_not_called()

    def test_duplicate_window(self):
        self.check_refusal([accessible("Properties", "frame"),
                            accessible("Properties", "dialog")], "ambiguous")

    def test_duplicate_button(self):
        self.check_refusal([accessible("Properties", "dialog",
                                       accessible("OK", "push button"),
                                       accessible("OK", "push button"))], "ambiguous")

    def test_duplicate_parent(self):
        self.check_refusal([accessible("Properties", "dialog",
                                       accessible("Section", "panel", accessible("OK", "push button")),
                                       accessible("Section", "panel"))], "ambiguous", "Section")

    def test_other_window_button(self):
        self.check_refusal([accessible("Unrelated", "dialog", accessible("OK", "push button")),
                            accessible("Properties", "dialog")], "not-found")


if __name__ == "__main__":
    unittest.main()
