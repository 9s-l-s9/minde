// SPDX-License-Identifier: MIT
// Exercise Smithay's actual wl_touch events and per-slot focus without a GPU.
use super::*;
use smithay::{
    input::{
        SeatState,
        touch::{
            GrabStartData, OrientationEvent, ShapeEvent, TouchGrab, TouchInnerHandle, UpEvent,
        },
    },
    reexports::wayland_server::{Client, Display, DisplayHandle, protocol::wl_surface::WlSurface},
    utils::SERIAL_COUNTER,
    wayland::compositor::{CompositorClientState, CompositorHandler, CompositorState},
};
use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    sync::Arc,
};

struct State {
    compositor: CompositorState,
    seats: SeatState<Self>,
    contacts: TouchContacts,
    grab_ended: bool,
}
smithay::delegate_dispatch2!(State);
impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client
            .get_data::<crate::state::ClientState>()
            .unwrap()
            .compositor_state
    }
    fn commit(&mut self, _: &WlSurface) {}
}
impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seats
    }
}

struct Event {
    object: u32,
    opcode: u16,
    data: Vec<u8>,
}
impl Event {
    fn word(&self, index: usize) -> u32 {
        u32::from_ne_bytes(self.data[index * 4..index * 4 + 4].try_into().unwrap())
    }
}
struct Peer {
    stream: UnixStream,
    surface: WlSurface,
}
fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}
fn send(mut stream: &UnixStream, object: u32, opcode: u16, payload: &[u8]) {
    let mut request = words(&[object, ((payload.len() + 8) as u32) << 16 | opcode as u32]);
    request.extend_from_slice(payload);
    stream.write_all(&request).unwrap();
}
fn events(stream: &mut UnixStream, handle: &DisplayHandle) -> Vec<Event> {
    handle.clone().flush_clients().unwrap();
    let mut bytes = Vec::new();
    match stream.read_to_end(&mut bytes) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
        Err(error) => panic!("reading touch events: {error}"),
    }
    let mut events = Vec::new();
    while !bytes.is_empty() {
        let object = u32::from_ne_bytes(bytes[..4].try_into().unwrap());
        let header = u32::from_ne_bytes(bytes[4..8].try_into().unwrap());
        let size = (header >> 16) as usize;
        assert!(size >= 8 && size <= bytes.len());
        assert_ne!((object, header as u16), (1, 0), "Wayland protocol error");
        events.push(Event {
            object,
            opcode: header as u16,
            data: bytes[8..size].to_vec(),
        });
        bytes.drain(..size);
    }
    events
}
impl Peer {
    fn events(&mut self, handle: &DisplayHandle) -> Vec<Event> {
        events(&mut self.stream, handle)
            .into_iter()
            .filter(|event| event.object == 5)
            .collect()
    }
    fn opcodes(&mut self, handle: &DisplayHandle) -> Vec<u16> {
        self.events(handle)
            .into_iter()
            .map(|event| event.opcode)
            .collect()
    }
}
struct Fixture {
    display: Display<State>,
    state: State,
    touch: TouchHandle<State>,
}
impl Fixture {
    fn new() -> Self {
        let display = Display::<State>::new().unwrap();
        let compositor = CompositorState::new::<State>(&display.handle());
        let mut seats = SeatState::new();
        let mut seat = seats.new_wl_seat(&display.handle(), "touch test");
        let touch = seat.add_touch();
        Self {
            display,
            state: State {
                compositor,
                seats,
                contacts: TouchContacts::default(),
                grab_ended: false,
            },
            touch,
        }
    }
    fn peer(&mut self) -> Peer {
        let (server, mut stream) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let handle = self.display.handle();
        let client = self
            .display
            .handle()
            .insert_client(server, Arc::new(crate::state::ClientState::default()))
            .unwrap();
        send(&stream, 1, 1, &words(&[2])); // get_registry
        self.display.dispatch_clients(&mut self.state).unwrap();
        let globals = events(&mut stream, &handle);
        // Allocate client IDs in order, independent of registry announcement order.
        for (interface, id, version) in [("wl_seat", 3, 7), ("wl_compositor", 4, 4)] {
            let interface = interface.as_bytes();
            let event = globals
                .iter()
                .find(|event| {
                    event.object == 2
                        && event.opcode == 0
                        && &event.data[8..8 + event.word(1) as usize - 1] == interface
                })
                .unwrap();
            let length = interface.len() + 1;
            let mut payload = words(&[event.word(0), length as u32]);
            payload.extend_from_slice(interface);
            payload.push(0);
            payload.resize(payload.len().next_multiple_of(4), 0);
            payload.extend(words(&[version, id]));
            send(&stream, 2, 0, &payload);
        }
        send(&stream, 3, 2, &words(&[5])); // get_touch
        send(&stream, 4, 0, &words(&[6])); // create_surface
        self.display.dispatch_clients(&mut self.state).unwrap();
        let surface = client
            .object_from_protocol_id::<WlSurface>(&handle, 6)
            .unwrap();
        events(&mut stream, &handle);
        Peer { stream, surface }
    }
    fn down(&mut self, peer: &Peer, slot: u32) {
        let event = DownEvent {
            slot: Some(slot).into(),
            location: (20.0, 30.0).into(),
            serial: SERIAL_COUNTER.next_serial(),
            time: 10,
        };
        self.state.contacts.down(&event);
        self.touch.down(
            &mut self.state,
            Some((peer.surface.clone(), (0.0, 0.0).into())),
            &event,
        );
    }
    fn motion(&mut self, slot: u32) {
        let event = MotionEvent {
            slot: Some(slot).into(),
            location: (25.0, 35.0).into(),
            time: 20,
        };
        if self.state.contacts.motion(&event) {
            self.touch.motion(&mut self.state, None, &event);
        }
    }
    fn up(&mut self, slot: u32) {
        if self.state.contacts.up(Some(slot).into()) {
            self.touch.up(
                &mut self.state,
                &UpEvent {
                    slot: Some(slot).into(),
                    serial: SERIAL_COUNTER.next_serial(),
                    time: 30,
                },
            );
        }
    }
    fn frame(&mut self) {
        if self.state.contacts.take_frame() {
            self.touch.frame(&mut self.state);
        }
    }
    fn cancel(&mut self) {
        std::mem::take(&mut self.state.contacts).cancel(&self.touch, &mut self.state);
        assert!(!self.touch.is_grabbed());
    }
    fn assert_old_focus_cleared(&mut self, slots: &[u32], peers: &mut [&mut Peer]) {
        // Bypass our contact ledger to prove Smithay itself forgot old focus.
        for slot in slots {
            self.touch.motion(
                &mut self.state,
                None,
                &MotionEvent {
                    slot: Some(*slot).into(),
                    location: (99.0, 99.0).into(),
                    time: 40,
                },
            );
        }
        self.touch.frame(&mut self.state);
        for peer in peers {
            assert!(peer.opcodes(&self.display.handle()).is_empty());
        }
    }
}

#[test]
fn cancellation_before_or_after_a_frame_reaches_the_client_and_clears_focus() {
    for framed in [false, true] {
        let mut f = Fixture::new();
        let mut peer = f.peer();
        f.down(&peer, 0);
        if framed {
            f.frame();
        }
        assert_eq!(
            peer.opcodes(&f.display.handle()),
            if framed { vec![0, 3] } else { vec![0] }
        );
        f.cancel();
        assert_eq!(
            peer.opcodes(&f.display.handle()),
            if framed { vec![2, 4] } else { vec![3, 2, 4] }
        );
        f.assert_old_focus_cleared(&[0], &mut [&mut peer]);
        f.cancel();
        f.motion(0);
        f.up(0);
        f.frame();
        assert!(peer.opcodes(&f.display.handle()).is_empty());
        let mut next = f.peer();
        f.down(&next, 2);
        f.up(0); // A late old release must not end the new implicit grab.
        assert!(f.touch.is_grabbed());
        f.frame();
        assert_eq!(next.opcodes(&f.display.handle()), [0, 3]);
        assert!(peer.opcodes(&f.display.handle()).is_empty());
    }
}

#[test]
fn cancellation_covers_stationary_slots_and_every_client_once() {
    let mut f = Fixture::new();
    let mut first = f.peer();
    let mut second = f.peer();
    f.down(&first, 0);
    f.down(&first, 1);
    f.frame();
    assert_eq!(first.opcodes(&f.display.handle()), [0, 0, 3]);
    // Model a compositor grab change leaving contacts on two client targets.
    f.touch.unset_grab(&mut f.state);
    f.down(&second, 2);
    f.frame();
    assert_eq!(second.opcodes(&f.display.handle()), [0, 3]);
    f.motion(0);
    f.cancel();
    let first_events = first.events(&f.display.handle());
    assert_eq!(first_events.iter().filter(|e| e.opcode == 4).count(), 1);
    let motions: Vec<_> = first_events.iter().filter(|e| e.opcode == 2).collect();
    assert_eq!(motions.len(), 3); // Real motion, then both held positions.
    assert!(
        motions
            .iter()
            .any(|e| e.word(1) == 1 && e.word(2) == 20 * 256)
    );
    assert_eq!(second.opcodes(&f.display.handle()), [2, 4]);
    f.assert_old_focus_cleared(&[0, 1, 2], &mut [&mut first, &mut second]);
}

#[test]
fn pending_ups_finish_before_cancel_and_do_not_leave_deferred_client_frames() {
    let mut f = Fixture::new();
    let mut peer = f.peer();
    f.down(&peer, 0);
    f.down(&peer, 1);
    f.frame();
    peer.events(&f.display.handle());
    f.up(0);
    f.cancel();
    assert_eq!(peer.opcodes(&f.display.handle()), [1, 3, 2, 4]);
    f.assert_old_focus_cleared(&[0, 1], &mut [&mut peer]);
    f.down(&peer, 2);
    f.frame();
    peer.events(&f.display.handle());
    f.up(2);
    f.cancel();
    assert_eq!(peer.opcodes(&f.display.handle()), [1, 3]);
    f.cancel();
    assert!(peer.opcodes(&f.display.handle()).is_empty());
}

// Like a modal/DnD grab, this grab can swallow cancel and motion. Cleanup must
// run its unset callback, then clear ordinary slot focus through the default grab.
struct SwallowGrab(GrabStartData<State>);
impl TouchGrab<State> for SwallowGrab {
    fn down(
        &mut self,
        _: &mut State,
        _: &mut TouchInnerHandle<'_, State>,
        _: Option<(
            WlSurface,
            smithay::utils::Point<f64, smithay::utils::Logical>,
        )>,
        _: &DownEvent,
    ) {
    }
    fn up(&mut self, _: &mut State, _: &mut TouchInnerHandle<'_, State>, _: &UpEvent) {}
    fn motion(
        &mut self,
        _: &mut State,
        _: &mut TouchInnerHandle<'_, State>,
        _: Option<(
            WlSurface,
            smithay::utils::Point<f64, smithay::utils::Logical>,
        )>,
        _: &MotionEvent,
    ) {
    }
    fn frame(&mut self, _: &mut State, _: &mut TouchInnerHandle<'_, State>) {}
    fn cancel(&mut self, _: &mut State, _: &mut TouchInnerHandle<'_, State>) {}
    fn shape(&mut self, _: &mut State, _: &mut TouchInnerHandle<'_, State>, _: &ShapeEvent) {}
    fn orientation(
        &mut self,
        _: &mut State,
        _: &mut TouchInnerHandle<'_, State>,
        _: &OrientationEvent,
    ) {
    }
    fn start_data(&self) -> &GrabStartData<State> {
        &self.0
    }
    fn unset(&mut self, data: &mut State) {
        data.grab_ended = true;
    }
}

#[test]
fn cancellation_ends_swallowing_grabs_before_clearing_slot_focus() {
    let mut f = Fixture::new();
    let mut peer = f.peer();
    f.down(&peer, 0);
    f.frame();
    peer.events(&f.display.handle());
    let start = f.touch.grab_start_data().unwrap();
    f.touch.set_grab(
        &mut f.state,
        SwallowGrab(start),
        SERIAL_COUNTER.next_serial(),
    );
    f.cancel();
    assert!(f.state.grab_ended);
    assert_eq!(peer.opcodes(&f.display.handle()), [2, 4]);
    f.assert_old_focus_cleared(&[0], &mut [&mut peer]);
}
