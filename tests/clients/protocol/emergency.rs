// SPDX-License-Identifier: GPL-3.0-or-later
use super::wire::*;
use std::{env, fs, io, time::Instant};

fn forwarded(client: &mut Client, chord: &str, code: u32) {
    let start = client.events.len();
    let keyboard = client.keyboard;
    command("xdotool", &["key", chord]);
    for state in [1, 0] {
        client.wait(start, |e| {
            e.is(keyboard, 3) && e.word(2) == code && e.word(3) == state
        });
    }
}

fn write_config(fields: &str) {
    fs::write(
        env::var("MINDE_EMERGENCY_CONFIG").unwrap(),
        format!("(minde-config (version 1) (prefix (ctrl) \"t\") {fields})\n"),
    )
    .unwrap();
}

pub fn run(mode: &str) {
    let mut desktop = Client::new("emergency exit configuration");
    desktop.toplevel(true);
    desktop.event(desktop.keyboard, 1);
    // The fixture disables the native exit before backend startup. The XKB
    // terminate option is enabled, so this also exercises Terminate_Server.
    forwarded(&mut desktop, "super+q", 16);
    forwarded(&mut desktop, "ctrl+alt+BackSpace", 14);
    println!("ok - Super+Q and the disabled default chord reach the application");

    write_config("(emergency-exit (ctrl alt shift) \"Escape\") (bindings)");
    assert_eq!(ipc("(reload-configuration!)"), "(ok #t)");
    forwarded(&mut desktop, "ctrl+alt+BackSpace", 14);
    forwarded(&mut desktop, "ctrl+alt+Escape", 1);
    // All parsing must finish before either the keymap or native exit changes.
    write_config("(emergency-exit #f) (bindings (\"x\" missing-emergency-command!))");
    assert_eq!(ipc("(reload-configuration!)"), "(ok #f)");
    assert_eq!(
        ipc("(and (= %prefix-mods 4) (string=? %prefix-key \"t\"))"),
        "(ok #t)"
    );
    assert_eq!(
        ipc("(begin
          (for-each
           (lambda (args)
             (unless (eq? #f (apply wm-set-emergency-exit args))
               (error \"invalid emergency exit accepted\" args)))
           (list (list 12.0 \"Escape\") (list -1 \"Escape\")
                 (list (expt 2 40) \"Escape\") (list 2 \"Escape\")
                 (list #f \"Escape\") (list 12 #f) (list 12 'Escape)
                 (list 12 \"NoSymbol\") (list 12 \"escape\")
                 (list 12 (string-append \"Escape\" (string #\\nul)))))
          #t)"),
        "(ok #t)"
    );
    // The actual chosen chord below must still exit after both kinds of error.
    assert_eq!(
        ipc("(begin (define (wm-handle-key . args) (error \"broken key policy\")) #t)"),
        "(ok #t)"
    );
    let mut target = match mode {
        "locked" => {
            let mut owner = Client::new("configured emergency exit while locked");
            let lock = owner.lock(true);
            owner.lock_surface(lock, true);
            owner.event(owner.keyboard, 1);
            locked(true);
            owner
        }
        "inhibited" => {
            let manager = desktop.bind("zwp_keyboard_shortcuts_inhibit_manager_v1", 1);
            let seat = desktop.bind("wl_seat", 1);
            let inhibitor = desktop.alloc();
            desktop.send(manager, 1, &words(&[inhibitor, desktop.surface, seat]));
            desktop.event(inhibitor, 0); // active, not merely requested
            // A normally reserved prefix must now reach the client.
            forwarded(&mut desktop, "ctrl+t", 20);
            desktop
        }
        _ => panic!("expected locked or inhibited"),
    };
    forwarded(&mut target, "ctrl+alt+BackSpace", 14);
    forwarded(&mut target, "ctrl+alt+Escape", 1);
    command("xdotool", &["key", "ctrl+alt+shift+Escape"]);
    let end = Instant::now() + TIMEOUT;
    loop {
        match target.read(end) {
            Ok(_) => {}
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
                ) =>
            {
                break;
            }
            error => panic!("configured emergency exit failed: {error:?}"),
        }
    }
    println!(
        "ok - configured emergency exit survives rejected updates and broken Scheme while {mode}"
    );
}
