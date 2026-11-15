// SPDX-License-Identifier: GPL-3.0-or-later

//! Reconcile controller assignments from complete connector snapshots.

use smithay::reexports::drm::control::{Device, connector, crtc};
use smithay_drm_extras::drm_scanner::CrtcMapper;
use std::{collections::HashMap, hash::Hash};

struct Routes<C, R> {
    connector: C,
    current: Option<R>,
    possible: Vec<R>,
}

fn assign<C: Copy + Eq + Hash, R: Copy + Eq>(
    previous: &HashMap<C, R>,
    routes: &[Routes<C, R>],
) -> HashMap<C, R> {
    // Keep connected heads stable, including deliberately disabled heads.
    // Entries absent from the snapshot must release their controller too.
    let mut assigned: HashMap<_, _> = routes
        .iter()
        .filter_map(|r| previous.get(&r.connector).map(|crtc| (r.connector, *crtc)))
        .collect();
    for route in routes {
        if !assigned.contains_key(&route.connector)
            && let Some(crtc) = route.current
            && !assigned.values().any(|used| *used == crtc)
        {
            assigned.insert(route.connector, crtc);
        }
    }
    for route in routes {
        if !assigned.contains_key(&route.connector)
            && let Some(crtc) = route
                .possible
                .iter()
                .find(|crtc| !assigned.values().any(|used| used == *crtc))
        {
            assigned.insert(route.connector, *crtc);
        }
    }
    assigned
}

#[derive(Default)]
struct ConnectorMapper {
    assigned: HashMap<connector::Handle, crtc::Handle>,
}

impl CrtcMapper for ConnectorMapper {
    fn map<'a>(
        &mut self,
        drm: &impl Device,
        connectors: impl Iterator<Item = &'a connector::Info> + Clone,
    ) {
        let Ok(resources) = drm.resource_handles() else {
            return;
        };
        let mut connectors: Vec<_> = connectors
            .filter(|info| info.state() == connector::State::Connected)
            .collect();
        connectors.sort_by_key(|info| u32::from(info.handle()));
        let routes: Vec<_> = connectors
            .into_iter()
            .map(|info| Routes {
                connector: info.handle(),
                current: info
                    .current_encoder()
                    .and_then(|e| drm.get_encoder(e).ok())
                    .and_then(|e| e.crtc()),
                possible: info
                    .encoders()
                    .iter()
                    .filter_map(|e| drm.get_encoder(*e).ok())
                    .flat_map(|e| resources.filter_crtcs(e.possible_crtcs()))
                    .collect(),
            })
            .collect();
        self.assigned = assign(&self.assigned, &routes);
    }

    fn crtc_for_connector(&self, connector: &connector::Handle) -> Option<crtc::Handle> {
        self.assigned.get(connector).copied()
    }
}

/// Whether a scan asks the driver to detect every display again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Probe {
    /// What the kernel last saw. Returns at once.
    Cached,
    /// Re-detect displays and their modes. Disconnected DP and HDMI ports
    /// can take the driver hundreds of milliseconds each.
    Forced,
}

/// The connectors of one device as last read, with their controllers.
#[derive(Default)]
pub(super) struct ConnectorScan {
    connectors: HashMap<connector::Handle, connector::Info>,
    mapper: ConnectorMapper,
}

impl ConnectorScan {
    pub(super) fn scan(&mut self, drm: &impl Device, probe: Probe) -> std::io::Result<()> {
        let resources = drm.resource_handles()?;
        self.connectors = resources
            .connectors()
            .iter()
            .filter_map(|handle| drm.get_connector(*handle, probe == Probe::Forced).ok())
            .map(|info| (info.handle(), info))
            .collect();
        self.mapper.map(drm, self.connectors.values());
        Ok(())
    }

    pub(super) fn connectors(&self) -> impl Iterator<Item = &connector::Info> {
        self.connectors.values()
    }

    pub(super) fn crtc_for_connector(&self, connector: &connector::Handle) -> Option<crtc::Handle> {
        self.mapper.crtc_for_connector(connector)
    }
}

/// A controller must be retired before its new connector is brought up.
pub(super) fn retired<C: Copy + Eq, R: Copy + Eq>(
    heads: &[(C, R)],
    connected: &[(C, R)],
) -> Vec<R> {
    heads
        .iter()
        .filter(|pair| !connected.contains(pair))
        .map(|(_, crtc)| *crtc)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routes(ids: &[u8]) -> Vec<Routes<u8, u8>> {
        ids.iter()
            .map(|id| Routes {
                connector: *id,
                current: None,
                possible: vec![10, 11],
            })
            .collect()
    }

    #[test]
    fn waiting_connector_gets_a_freed_controller_without_a_new_connect_event() {
        let first = assign(&HashMap::new(), &routes(&[1, 2]));
        let full = assign(&first, &routes(&[1, 2, 3]));
        assert_eq!(first, full);
        assert!(!full.contains_key(&3));
        // Connector 1 disappears from the resource snapshot altogether.
        let next = assign(&full, &routes(&[2, 3]));
        assert_eq!(next[&2], first[&2]);
        assert_eq!(next[&3], first[&1]);
        let old: Vec<_> = first.into_iter().collect();
        let current: Vec<_> = next.into_iter().collect();
        assert_eq!(retired(&old, &current), vec![10]);
    }

    #[test]
    fn connected_heads_keep_assignments_without_active_encoders() {
        let previous = HashMap::from([(2, 10), (3, 11)]);
        // A new connector comes first, but cannot steal a disabled head's CRTC.
        assert_eq!(assign(&previous, &routes(&[1, 2, 3])), previous);
        assert!(retired(&[(2, 10), (3, 11)], &[(2, 10), (3, 11)]).is_empty());
    }

    #[test]
    fn kernel_assignments_and_encoder_compatibility_are_respected() {
        let mut input = routes(&[1, 2, 3]);
        input[0].current = Some(11);
        input[1].possible = vec![11];
        input[2].possible = vec![10];
        assert_eq!(
            assign(&HashMap::new(), &input),
            HashMap::from([(1, 11), (3, 10)])
        );
    }
}
