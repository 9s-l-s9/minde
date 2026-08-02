// SPDX-License-Identifier: GPL-3.0-or-later

//! Typed commands crossing from Scheme policy to compositor state.

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
    },
    Click {
        button: u32,
    },
    SendKey {
        mods: u32,
        keysym: String,
    },
    WarpPointerRel {
        dx: i32,
        dy: i32,
    },
    SetKeyRepeat {
        on: bool,
    },
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
}
