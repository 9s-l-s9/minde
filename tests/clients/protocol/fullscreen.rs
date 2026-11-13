// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    input::evaluate,
    lock::inject_key,
    window::{Observer, window_id},
    wire::*,
};

fn request(client: &mut Client, on: bool) {
    client.roundtrip();
    let start = client.events.len();
    client.send(
        client.toplevel_object,
        if on { 11 } else { 12 },
        &if on { words(&[0]) } else { vec![] },
    );
    let surface = client.xdg_surface;
    client.wait(start, |event| event.is(surface, 0));
    client.roundtrip();
}
pub(super) fn selected(group: &str, id: u32) {
    evaluate("(current-group-name)", &format!("{group:?}"));
    evaluate("(focused-window-id)", &id.to_string());
}
pub(super) fn activated(observer: &mut Observer, title: &str, expected: bool) {
    let handle = observer.handle(title);
    let state = observer.c.events.iter().rfind(|e| e.is(handle, 4)).unwrap();
    let active = state.data[4..4 + state.word(0) as usize]
        .chunks_exact(4)
        .any(|word| u32::from_ne_bytes(word.try_into().unwrap()) == 2);
    assert_eq!(active, expected, "foreign activation for {title}");
}
pub fn run() {
    let mut observer = Observer::new();
    let mut anchor = Client::new("fullscreen request anchor");
    anchor.toplevel(true);
    let anchor_id = window_id(&anchor.name);
    evaluate(
        "(begin (create-group! \"fullscreen requests\") (switch-to-group! \"fullscreen requests\") #t)",
        "#t",
    );
    let mut target = Client::new("fullscreen requester");
    target.toplevel(true);
    let target_id = window_id(&target.name);
    evaluate("(begin (switch-to-group! \" I \") #t)", "#t");
    for on in [true, false, true] {
        request(&mut target, on);
        selected(" I ", anchor_id);
        evaluate(&format!("(wm-window-geometry {target_id})"), "#f");
        activated(&mut observer, &anchor.name, true);
        activated(&mut observer, &target.name, false);
        inject_key(&mut anchor, &mut [&mut target]);
    }
    evaluate(
        "(begin (switch-to-group! \"fullscreen requests\") #t)",
        "#t",
    );
    target.roundtrip();
    selected("fullscreen requests", target_id);
    evaluate("(fullscreen-window)", &target_id.to_string());
    evaluate(
        &format!("(wm-window-geometry {target_id})"),
        "(0 0 1280 800)",
    );
    inject_key(&mut target, &mut [&mut anchor]);
    request(&mut target, false);
    evaluate("(fullscreen-window)", "#f");
    println!(
        "ok - background application fullscreen requests preserve group, focus and parked geometry"
    );

    let mut peer = Client::new("fullscreen request focused peer");
    peer.toplevel(true);
    let peer_id = window_id(&peer.name);
    for _ in 0..2 {
        request(&mut target, true);
        selected("fullscreen requests", peer_id);
        evaluate("(fullscreen-window)", &target_id.to_string());
        activated(&mut observer, &peer.name, true);
        activated(&mut observer, &target.name, false);
        evaluate("(begin (sync-frames!) #t)", "#t");
        selected("fullscreen requests", peer_id);
        inject_key(&mut peer, &mut [&mut target, &mut anchor]);
    }
    for callback in ["handle-window-fullscreen!", "handle-foreign-fullscreen!"] {
        for on in ["#t", "#f"] {
            evaluate(&format!("(begin ({callback} 999999 {on}) #t)"), "#t");
            selected("fullscreen requests", peer_id);
            evaluate("(fullscreen-window)", &target_id.to_string());
        }
    }
    request(&mut peer, false); // Cannot clear another window's fullscreen.
    evaluate("(fullscreen-window)", &target_id.to_string());
    let mut newcomer = Client::new("fullscreen request new mapping");
    newcomer.toplevel(true);
    let newcomer_id = window_id(&newcomer.name);
    selected("fullscreen requests", peer_id);
    request(&mut target, false);
    selected("fullscreen requests", peer_id);
    evaluate("(fullscreen-window)", "#f");
    inject_key(&mut peer, &mut [&mut target, &mut newcomer, &mut anchor]);
    drop(newcomer);
    until(
        || ipc(&format!("(wm-window-title {newcomer_id})")) == "(ok #f)",
        "new fullscreen client stayed mapped",
    );
    println!(
        "ok - same-group requests and invalid IDs preserve the selected window and actual keyboard target"
    );

    drop(peer);
    until(
        || ipc(&format!("(wm-window-title {peer_id})")) == "(ok #f)",
        "fullscreen peer stayed mapped",
    );
    evaluate("(begin (switch-to-group! \" I \") #t)", "#t");
    let handle = observer.handle(&target.name);
    observer.c.send(handle, 8, &words(&[0])); // Explicit taskbar fullscreen.
    observer.c.roundtrip();
    target.roundtrip();
    selected("fullscreen requests", target_id);
    evaluate("(fullscreen-window)", &target_id.to_string());
    inject_key(&mut target, &mut [&mut anchor]);
    evaluate("(begin (switch-to-group! \" I \") #t)", "#t");
    request(&mut target, false);
    selected(" I ", anchor_id);
    evaluate(&format!("(wm-window-geometry {target_id})"), "#f");
    evaluate(
        "(begin (switch-to-group! \"fullscreen requests\") #t)",
        "#t",
    );
    target.roundtrip();
    evaluate("(fullscreen-window)", "#f");
    println!(
        "ok - taskbar fullscreen explicitly activates its target; hidden client exit does not"
    );
    drop(target);
    drop(anchor);
    until(
        || ipc("(all-window-ids)") == "(ok ())",
        "fullscreen clients stayed mapped",
    );
    evaluate(
        "(begin (delete-current-group!) (switch-to-group! \" I \") (collapse-to-one-frame!) #t)",
        "#t",
    );
}
