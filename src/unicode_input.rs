// SPDX-License-Identifier: GPL-3.0-or-later

//! Unicode input through a temporary client keymap, without changing the
//! seat's XKB state, physical presses, layout groups or selections.

use smithay::{
    input::keyboard::{KeymapFile, SerializedMods, xkb},
    reexports::wayland_server::{
        Resource,
        protocol::wl_keyboard::{KeyState, KeymapFormat, WlKeyboard},
    },
    utils::SERIAL_COUNTER,
};
use std::{
    io,
    os::fd::{AsFd, OwnedFd},
};

/// Reserved code used only to reach KeyboardFocusTarget through Smithay's
/// forwarding path. The client receives a different, unpressed keycode.
pub(crate) const KEYCODE: u32 = 8;

type MapFd = (OwnedFd, u32);

#[derive(Debug)]
struct PreparedKeyboard {
    keyboard: WlKeyboard,
    temporary: MapFd,
    original: MapFd,
}

fn prepare_fd(map: &KeymapFile, sealed: bool) -> io::Result<MapFd> {
    let mut result = None;
    map.with_fd(sealed, |fd, size| {
        result = Some(fd.try_clone_to_owned().map(|fd| (fd, size as u32)));
    })?;
    result.expect("with_fd invoked its callback")
}

#[derive(Debug)]
pub(crate) struct UnicodeStroke {
    keyboards: Vec<PreparedKeyboard>,
    modifiers: SerializedMods,
    keycode: u32,
    deferred: bool,
}

impl UnicodeStroke {
    pub(crate) fn new(
        ch: char,
        keycode: u32,
        original: &xkb::Keymap,
        modifiers: SerializedMods,
        keyboards: Vec<WlKeyboard>,
        deferred: bool,
    ) -> Option<Self> {
        let name = original.key_get_name(xkb::Keycode::new(keycode))?;
        let mut source = original.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
        // libxkbcommon's canonical serialization ends with xkb_symbols and
        // then xkb_keymap. Add an override inside that final symbols section.
        // Keeping the full map also keeps X11's required compatibility table.
        let end = source.rfind("\n};\n\n};")?;
        source.insert_str(
            end,
            &format!(
                "\nreplace key <{name}> {{ type = \"ONE_LEVEL\", repeat = No,\n\
             symbols[Group1] = [ 0x{:x} ], actions[Group1] = [ NoAction() ] }};",
                xkb::utf32_to_keysym(ch as u32).raw()
            ),
        );
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let temporary = xkb::Keymap::new_from_string(
            &context,
            source,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )?;
        let temporary = KeymapFile::new(&temporary);
        let original = KeymapFile::new(original);
        // Prepare restoration descriptors before sending any event. Older
        // keyboards need writable copies; restoration must not depend on a
        // later successful temporary-file allocation.
        let keyboards = keyboards
            .into_iter()
            .map(|keyboard| {
                let sealed = keyboard.version() >= 7;
                Ok(PreparedKeyboard {
                    temporary: prepare_fd(&temporary, sealed)?,
                    original: prepare_fd(&original, sealed)?,
                    keyboard,
                })
            })
            .collect::<io::Result<Vec<_>>>()
            .ok()?;
        Some(Self {
            keyboards,
            modifiers,
            keycode,
            deferred,
        })
    }

    pub(crate) fn send(&self, time: u32) {
        let press = SERIAL_COUNTER.next_serial().into();
        let release = SERIAL_COUNTER.next_serial().into();
        for prepared in &self.keyboards {
            let keyboard = &prepared.keyboard;
            keyboard.keymap(
                KeymapFormat::XkbV1,
                prepared.temporary.0.as_fd(),
                prepared.temporary.1,
            );
            keyboard.modifiers(press, 0, 0, 0, 0);
            keyboard.key(press, time, self.keycode - 8, KeyState::Pressed);
            keyboard.key(release, time, self.keycode - 8, KeyState::Released);
            // Xwayland drains its queued key events on modifiers.
            keyboard.modifiers(release, 0, 0, 0, 0);
        }
        if !self.deferred {
            self.restore();
        }
    }

    pub(crate) fn restore(&self) {
        let serial = SERIAL_COUNTER.next_serial().into();
        let mods = self.modifiers;
        for prepared in &self.keyboards {
            let keyboard = &prepared.keyboard;
            keyboard.keymap(
                KeymapFormat::XkbV1,
                prepared.original.0.as_fd(),
                prepared.original.1,
            );
            keyboard.modifiers(
                serial,
                mods.depressed,
                mods.latched,
                mods.locked,
                mods.layout_effective,
            );
        }
    }
}
