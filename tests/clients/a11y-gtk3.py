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
for _ in range(2):
    box.add(Gtk.Button(label="Duplicate"))
window.add_button("Finish", Gtk.ResponseType.OK)


def submitted(dialog, response):
    if response == Gtk.ResponseType.OK:
        result_path.write_text(json.dumps([units.get_active_text(), capacity.get_text()]))
    Gtk.main_quit()


window.connect("response", submitted)
window.show_all()
Gtk.main()
