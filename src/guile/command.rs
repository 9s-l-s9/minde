// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed commands crossing from Scheme policy to compositor state.

use super::output_config::OutputChangeSpec;

#[derive(Debug, Clone)]
pub enum WmCommand {
    Place {
        id: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    Focus {
        id: u64,
    },
    ClearFocus,
    Close {
        id: u64,
    },
    /// Rectangle selected by Scheme, including when its frame is empty.
    FocusRect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    Message {
        text: String,
        timeout_ms: u64,
    },
    ClearMessage,
    BorderColor {
        rgba: [f32; 4],
    },
    ConfigureAppearance(Vec<super::AppearanceChange>),
    /// One-shot timer whose token indexes the Scheme-side thunk table.
    RunAfter {
        ms: u64,
        token: i64,
    },
    Fullscreen {
        id: u64,
        on: bool,
    },
    Kill {
        id: u64,
    },
    WarpPointer {
        x: i32,
        y: i32,
    },
    Paste,
    SetClipboard {
        text: String,
    },
    SetPrimary {
        text: String,
    },
    /// Float placement does not apply tiled states and raises the surface.
    PlaceFloat {
        id: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    Raise {
        id: u64,
    },
    SetFloating {
        id: u64,
        on: bool,
    },
    SendString {
        text: String,
        delay_ms: u64,
    },
    Click {
        button: u32,
        count: u32,
    },
    /// Hold `button` (1=left 2=middle 3=right) until `ButtonRelease`.
    ButtonPress {
        button: u32,
    },
    ButtonRelease {
        button: u32,
    },
    /// Press at (x1,y1), move in `steps` paced motion events, release at (x2,y2).
    Drag {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        steps: u32,
    },
    SendKey {
        mods: u32,
        keysym: String,
    },
    PasteKey,
    Scroll {
        dx: f64,
        dy: f64,
    },
    /// Deferred screenshot (PNG); completion via the automation-result
    /// registry under `token`, like drop-files.
    Screenshot {
        destination: crate::screenshot::Destination,
        window_id: Option<u64>,
        token: u64,
    },
    Drop {
        x: i32,
        y: i32,
        source: crate::automation_dnd::AutomationDndSource,
    },
    WarpPointerRel {
        dx: i32,
        dy: i32,
    },
    SetKeyRepeat {
        on: bool,
    },
    SetKeyboardRepeat(crate::input::KeyboardRepeat),
    AddOverlay {
        x: i32,
        y: i32,
        text: String,
    },
    ClearOverlays,
    /// Reapply stored libinput rules to devices already present.
    ReapplyInputConfig,
    /// Spawn on the main thread; forking from Guile's REPL thread can wedge
    /// graphics libraries in the parent process.
    Spawn {
        cmd: String,
    },
    /// `(wm-configure-output! name alist)`: apply an output change to the
    /// head called `name` through the same routine as the
    /// wlr-output-management protocol.
    ConfigureOutput {
        name: String,
        spec: OutputChangeSpec,
    },
    /// `(wm-set-keyboard-layout! spec)`: switch the active XKB layout
    /// group of the seat keyboard (see [`KeyboardLayoutSpec`]).
    SetKeyboardLayout {
        spec: KeyboardLayoutSpec,
    },
}

/// Which XKB layout group `wm-set-keyboard-layout!` should activate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardLayoutSpec {
    /// A zero-based index into the keymap's layout list.
    Index(usize),
    /// The group after the active one, wrapping around.
    Next,
    /// The group before the active one, wrapping around.
    Prev,
}

impl WmCommand {
    /// The variant's name without its payload, which can hold user text.
    pub(crate) fn name(&self) -> String {
        struct Name(String);
        impl std::fmt::Write for Name {
            fn write_str(&mut self, part: &str) -> std::fmt::Result {
                let end = part
                    .find(|c: char| !c.is_ascii_alphanumeric())
                    .unwrap_or(part.len());
                self.0.push_str(&part[..end]);
                // Stop formatting at the first character after the name.
                if end == part.len() {
                    Ok(())
                } else {
                    Err(std::fmt::Error)
                }
            }
        }
        let mut name = Name(String::new());
        let _ = std::fmt::write(&mut name, format_args!("{self:?}"));
        name.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_omits_the_payload() {
        assert_eq!(WmCommand::ClearFocus.name(), "ClearFocus");
        let typed = WmCommand::SendString {
            text: "secret".into(),
            delay_ms: 0,
        };
        assert_eq!(typed.name(), "SendString");
        assert_eq!(WmCommand::Focus { id: 7 }.name(), "Focus");
    }
}
