// SPDX-License-Identifier: GPL-3.0-or-later

//! Wall-clock probes around the compositor's hot paths: one command applied
//! from Scheme, one key dispatch, one rendered frame. Each probe keeps a
//! count, total and maximum plus a five-bucket histogram, all as atomics so
//! recording costs a few stores. `(wm-timing-stats)` reads the snapshot for
//! the diagnostics report, turning "unverified" costs into numbers on the
//! machine that matters.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Upper bounds (microseconds) of the histogram buckets; the last bucket is
/// open-ended. 16.6 ms is one 60 Hz frame, the budget everything competes
/// for.
pub const BUCKET_LIMITS_US: [u64; 4] = [100, 1_000, 4_000, 16_600];

#[derive(Clone, Copy)]
pub enum Probe {
    /// `MindeState::apply_wm_command`, direct or queued.
    ApplyCommand = 0,
    /// `(wm-handle-key ...)` for one key press, Scheme time included.
    HandleKey = 1,
    /// One `render_surface` (udev) or winit redraw.
    Render = 2,
}

const PROBES: [(&str, Probe); 3] = [
    ("apply-command", Probe::ApplyCommand),
    ("handle-key", Probe::HandleKey),
    ("render", Probe::Render),
];

#[derive(Default)]
struct Stats {
    count: AtomicU64,
    total_us: AtomicU64,
    max_us: AtomicU64,
    buckets: [AtomicU64; 5],
}

static STATS: [Stats; 3] = [
    Stats {
        count: AtomicU64::new(0),
        total_us: AtomicU64::new(0),
        max_us: AtomicU64::new(0),
        buckets: [const { AtomicU64::new(0) }; 5],
    },
    Stats {
        count: AtomicU64::new(0),
        total_us: AtomicU64::new(0),
        max_us: AtomicU64::new(0),
        buckets: [const { AtomicU64::new(0) }; 5],
    },
    Stats {
        count: AtomicU64::new(0),
        total_us: AtomicU64::new(0),
        max_us: AtomicU64::new(0),
        buckets: [const { AtomicU64::new(0) }; 5],
    },
];

/// Records the time elapsed since `start` under `probe`.
pub fn record(probe: Probe, start: Instant) {
    let elapsed = start.elapsed().as_micros().min(u64::MAX as u128) as u64;
    let stats = &STATS[probe as usize];
    stats.count.fetch_add(1, Ordering::Relaxed);
    stats.total_us.fetch_add(elapsed, Ordering::Relaxed);
    stats.max_us.fetch_max(elapsed, Ordering::Relaxed);
    let bucket = BUCKET_LIMITS_US
        .iter()
        .position(|limit| elapsed <= *limit)
        .unwrap_or(BUCKET_LIMITS_US.len());
    stats.buckets[bucket].fetch_add(1, Ordering::Relaxed);
}

/// One probe's snapshot: name, count, total and max microseconds, and the
/// histogram counts in `BUCKET_LIMITS_US` order plus the open-ended tail.
#[derive(Clone, Copy)]
pub struct Snapshot {
    pub name: &'static str,
    pub count: u64,
    pub total_us: u64,
    pub max_us: u64,
    pub buckets: [u64; 5],
}

pub fn snapshot() -> Vec<Snapshot> {
    PROBES
        .iter()
        .map(|(name, probe)| {
            let stats = &STATS[*probe as usize];
            Snapshot {
                name,
                count: stats.count.load(Ordering::Relaxed),
                total_us: stats.total_us.load(Ordering::Relaxed),
                max_us: stats.max_us.load(Ordering::Relaxed),
                buckets: std::array::from_fn(|i| stats.buckets[i].load(Ordering::Relaxed)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_into_the_matching_bucket() {
        let before = snapshot()[Probe::HandleKey as usize].count;
        record(Probe::HandleKey, Instant::now());
        let after = snapshot()[Probe::HandleKey as usize];
        assert_eq!(after.count, before + 1);
        assert!(
            after.buckets[0] >= 1,
            "an immediate sample lands in the first bucket"
        );
    }
}
