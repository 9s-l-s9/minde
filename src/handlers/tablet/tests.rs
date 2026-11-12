// SPDX-License-Identifier: MIT
//! Actual Smithay bindings over socket pairs, including clients with foreign userdata.
use super::*;
use smithay::{
    backend::input::{TabletToolCapabilities, TabletToolDescriptor, TabletToolType},
    input::{SeatHandler, SeatState},
    reexports::wayland_server::{
        Client, DataInit, Dispatch, Display, GlobalDispatch, New, backend::ClientData,
    },
    wayland::{
        Dispatch2, GlobalDispatch2,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        tablet_manager::{
            TabletDescriptor, TabletManagerState, TabletSeatHandle, TabletSeatHandler,
            TabletSeatTrait,
        },
    },
};
use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    sync::Arc,
};

#[derive(Default)]
struct ForeignClientData(CompositorClientState);
impl ClientData for ForeignClientData {}

struct State {
    compositor: CompositorState,
    seats: SeatState<Self>,
    clients: TabletClients,
}
impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ForeignClientData>().unwrap().0
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
impl TabletSeatHandler for State {}
impl<I: Resource + 'static, U: Dispatch2<I, Self>> Dispatch<I, U> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &I,
        request: I::Request,
        data: &U,
        handle: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        state.clients.request(&client.id(), &request);
        data.request(state, client, resource, request, handle, init);
    }
    fn destroyed(state: &mut Self, client: ClientId, resource: &I, data: &U) {
        state.clients.destroyed(&client, resource);
        data.destroyed(state, client, resource);
    }
}
impl<I: Resource + 'static, U: GlobalDispatch2<I, Self>> GlobalDispatch<I, U> for State {
    fn bind(
        state: &mut Self,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<I>,
        data: &U,
        init: &mut DataInit<'_, Self>,
    ) {
        data.bind(state, handle, client, resource, init);
    }
    fn can_view(client: Client, data: &U) -> bool {
        data.can_view(&client)
    }
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|n| n.to_ne_bytes()).collect()
}
fn word(bytes: &[u8], n: usize) -> u32 {
    u32::from_ne_bytes(bytes[n * 4..n * 4 + 4].try_into().unwrap())
}
fn send(mut stream: &UnixStream, object: u32, opcode: u16, payload: &[u8]) {
    let mut bytes = words(&[object, ((payload.len() + 8) as u32) << 16 | opcode as u32]);
    bytes.extend_from_slice(payload);
    stream.write_all(&bytes).unwrap();
}
struct Event {
    object: u32,
    opcode: u16,
    data: Vec<u8>,
}
struct Peer {
    stream: UnixStream,
    client: Client,
    surface: WlSurface,
    next: u32,
    seats: Vec<u32>,
}
impl Peer {
    fn alloc(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }
    fn events(&mut self, handle: &DisplayHandle) -> Vec<Event> {
        handle.clone().flush_clients().unwrap();
        let mut bytes = Vec::new();
        match self.stream.read_to_end(&mut bytes) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("tablet events: {e}"),
        }
        let mut events = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            let object = word(&bytes[offset..], 0);
            let header = word(&bytes[offset..], 1);
            let size = (header >> 16) as usize;
            assert!(size >= 8 && offset + size <= bytes.len());
            assert_ne!((object, header as u16), (1, 0), "Wayland error");
            events.push(Event {
                object,
                opcode: header as u16,
                data: bytes[offset + 8..offset + size].to_vec(),
            });
            offset += size;
        }
        events
    }
    fn tools(&mut self, handle: &DisplayHandle) -> Vec<u32> {
        self.events(handle)
            .into_iter()
            .filter(|e| self.seats.contains(&e.object) && e.opcode == 1)
            .map(|e| word(&e.data, 0))
            .collect()
    }
}
struct Fixture {
    display: Display<State>,
    state: State,
    tablet: TabletSeatHandle,
}
impl Fixture {
    fn new() -> Self {
        let display = Display::new().unwrap();
        let handle = display.handle();
        let compositor = CompositorState::new::<State>(&handle);
        let mut seats = SeatState::new();
        let seat = seats.new_wl_seat(&handle, "tablet test");
        let tablet = seat.tablet_seat();
        let _manager = TabletManagerState::new::<State>(&handle);
        tablet.add_tablet::<State>(
            &handle,
            &TabletDescriptor {
                name: "test".into(),
                usb_id: None,
                syspath: None,
            },
        );
        Self {
            display,
            state: State {
                compositor,
                seats,
                clients: Default::default(),
            },
            tablet,
        }
    }
    fn dispatch(&mut self) {
        self.display.dispatch_clients(&mut self.state).unwrap();
        self.display.flush_clients().unwrap();
    }
    fn peer(&mut self) -> Peer {
        let (server, mut stream) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let handle = self.display.handle();
        let client = handle
            .clone()
            .insert_client(server, Arc::new(ForeignClientData::default()))
            .unwrap();
        send(&stream, 1, 1, &words(&[2]));
        self.dispatch();
        let mut bytes = Vec::new();
        match stream.read_to_end(&mut bytes) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("registry: {e}"),
        }
        // Bind in client-ID order, independent of global announcement order.
        for (interface, id, version) in [
            ("wl_compositor", 3, 4),
            ("wl_seat", 4, 7),
            ("zwp_tablet_manager_v2", 5, 1),
        ] {
            let mut offset = 0;
            let global = loop {
                assert!(offset < bytes.len(), "missing {interface}");
                let size = (word(&bytes[offset..], 1) >> 16) as usize;
                let data = &bytes[offset + 8..offset + size];
                if &data[8..8 + word(data, 1) as usize - 1] == interface.as_bytes() {
                    break word(data, 0);
                }
                offset += size;
            };
            let mut payload = words(&[global, interface.len() as u32 + 1]);
            payload.extend_from_slice(interface.as_bytes());
            payload.push(0);
            payload.resize(payload.len().next_multiple_of(4), 0);
            payload.extend(words(&[version, id]));
            send(&stream, 2, 0, &payload);
        }
        send(&stream, 3, 0, &words(&[6]));
        self.dispatch();
        let surface = client.object_from_protocol_id(&handle, 6).unwrap();
        let mut peer = Peer {
            stream,
            client,
            surface,
            next: 7,
            seats: Vec::new(),
        };
        peer.events(&handle);
        peer
    }
    fn bind_seat(&mut self, peer: &mut Peer) -> Vec<u32> {
        let id = peer.alloc();
        peer.seats.push(id);
        send(&peer.stream, 5, 0, &words(&[id, 4]));
        self.dispatch();
        peer.tools(&self.display.handle())
    }
    fn tool(&mut self, serial: u64) {
        self.tablet.add_tool(
            &mut self.state,
            &self.display.handle(),
            &TabletToolDescriptor {
                tool_type: TabletToolType::Pen,
                hardware_serial: serial,
                hardware_id_wacom: 0,
                capabilities: TabletToolCapabilities::PRESSURE,
            },
        );
        self.state.clients.tool_added();
    }
    fn aware(&mut self, peer: &Peer) -> bool {
        self.state
            .clients
            .aware(&self.display.handle(), &peer.surface)
    }
    fn destroy_tool(&mut self, peer: &Peer, id: u32) {
        send(&peer.stream, id, 1, &[]);
        self.dispatch();
    }
}

#[test]
fn repeated_samples_and_client_switches_do_not_repeat_object_scans() {
    let mut f = Fixture::new();
    let mut aware = f.peer();
    let unaware = f.peer();
    // A browser-like object table must not be walked on each sample.
    for i in 0..1024 {
        let id = aware.alloc();
        send(&aware.stream, 3, 0, &words(&[id]));
        if i % 32 == 0 {
            f.dispatch();
        }
    }
    f.dispatch();
    assert!(!f.aware(&aware)); // Binding the manager alone is insufficient.
    f.tool(1);
    let first = f.bind_seat(&mut aware);
    assert_eq!(first.len(), 1);
    assert!(f.aware(&aware));
    assert!(!f.aware(&unaware));
    let scans = f.state.clients.scans;
    for _ in 0..1000 {
        assert!(f.aware(&aware));
        assert!(!f.aware(&unaware));
    }
    assert_eq!(f.state.clients.scans, scans);
    let second = f.bind_seat(&mut aware);
    assert_eq!(second.len(), 1);
    assert!(f.aware(&aware));
    f.destroy_tool(&aware, first[0]);
    assert!(f.aware(&aware));
    f.destroy_tool(&aware, second[0]);
    assert!(!f.aware(&aware));
    assert_eq!(f.state.clients.scans, scans + 3);
}

#[test]
fn new_tools_surface_destruction_and_disconnect_refresh_the_cache() {
    let mut f = Fixture::new();
    let mut peer = f.peer();
    assert!(f.bind_seat(&mut peer).is_empty());
    assert!(!f.aware(&peer));
    f.tool(1);
    let tools = peer.tools(&f.display.handle());
    assert_eq!(tools.len(), 1);
    assert!(f.aware(&peer));
    f.destroy_tool(&peer, tools[0]);
    assert!(!f.aware(&peer));
    f.tool(2);
    let tools = peer.tools(&f.display.handle());
    assert!(f.aware(&peer));
    // Destroying the seat does not destroy its already-created tool objects.
    send(&peer.stream, peer.seats[0], 0, &[]);
    f.dispatch();
    assert!(f.aware(&peer));
    send(&peer.stream, 6, 0, &[]);
    f.dispatch();
    assert!(f.state.clients.aware.is_empty());
    let surface_id = peer.alloc();
    send(&peer.stream, 3, 0, &words(&[surface_id]));
    f.dispatch();
    peer.surface = peer
        .client
        .object_from_protocol_id(&f.display.handle(), surface_id)
        .unwrap();
    assert!(f.aware(&peer));
    f.destroy_tool(&peer, tools[0]);
    assert!(!f.aware(&peer));
    let old_client = peer.client.id();
    drop(peer);
    f.dispatch();
    assert!(!f.state.clients.aware.contains_key(&old_client));
    let replacement = f.peer();
    assert!(!f.aware(&replacement));
}
