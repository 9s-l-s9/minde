#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Expose a form and record the submitted values outside the AT-SPI tree."""
import json
from pathlib import Path
import sys

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk

result_path = Path(sys.argv[1])
window = Gtk.Dialog(title=sys.argv[2])
units = Gtk.ComboBoxText()
units.get_accessible().set_name("Model time units")
for unit in ("seconds", "minutes", "hours"):
    units.append_text(unit)
units.set_active(0)
capacity = Gtk.Entry()
capacity.get_accessible().set_name("Capacity")
box = window.get_content_area()
box.add(units)
box.add(capacity)
selections = []
focus_order = []
for section in ("General", "Geräteeinstellungen"):
    panel = Gtk.Box()
    panel.get_accessible().set_name(section)
    button = Gtk.Button(label="Duplicate")
    button.connect("clicked", lambda _button, name: selections.append(name), section)
    panel.add(button)
    box.add(panel)
secret = Gtk.Entry()
secret.set_visibility(False)
secret.set_text("private-fixture-value")
secret.get_accessible().set_name("Password")
box.add(secret)
finish = window.add_button("Finish", Gtk.ResponseType.OK)


def remember_focus(_window, widget):
    if widget in (finish, capacity):
        focus_order.append(widget)


window.connect("set-focus", remember_focus)


def submitted(dialog, response):
    if response == Gtk.ResponseType.OK:
        moved_focus = any(a == finish and b == capacity for a, b in zip(focus_order, focus_order[1:]))
        result_path.write_text(json.dumps([units.get_active_text(), capacity.get_text(), selections, moved_focus]))
    Gtk.main_quit()


window.connect("response", submitted)
window.show_all()
Gtk.main()
