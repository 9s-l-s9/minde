// SPDX-License-Identifier: GPL-3.0-or-later

//! Wall-clock probes around the compositor's hot paths: one command applied
//! from Scheme, one key dispatch, one rendered frame, one layer arrangement,
//! one connector scan. Each probe keeps a count, total and maximum plus a
//! five-bucket histogram, all as atomics so recording costs a few stores.
//! Samples that exceed one frame are also kept with their context, so a slow
//! tail can be traced to its cause. `(wm-timing-stats)` and
//! `(wm-timing-slow)` read the snapshots for the diagnostics report, turning
//! "unverified" costs into numbers on the machine that matters.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Upper bounds (microseconds) of the histogram buckets; the last bucket is
/// open-ended. 16.6 ms is one 60 Hz frame, the budget everything competes
/// for.
pub const BUCKET_LIMITS_US: [u64; 4] = [100, 1_000, 4_000, 16_600];

/// Samples above one frame are rare enough to keep and explain.
const SLOW_US: u64 = BUCKET_LIMITS_US[3];
const SLOW_SAMPLES: usize = 32;

#[derive(Clone, Copy)]
pub enum Probe {
    /// `MindeState::apply_wm_command`, direct or queued.
    ApplyCommand = 0,
    /// `(wm-handle-key ...)` for one key press, Scheme time included.
    HandleKey = 1,
    /// One `render_surface` (udev) or winit redraw.
    Render = 2,
    /// Recompute output usable areas and arrange their layer surfaces.
    ArrangeLayers = 3,
    /// An incremental update of a cached capture image (excludes no damage).
    CaptureRender = 4,
    /// GPU readback of the accumulated changed rectangle into shared pixels.
    CaptureReadback = 5,
    /// One read of a DRM device's connectors, with or without probing.
    ConnectorScan = 6,
}

const PROBES: [(&str, Probe); 7] = [
    ("apply-command", Probe::ApplyCommand),
    ("handle-key", Probe::HandleKey),
    ("render", Probe::Render),
    ("arrange-layers", Probe::ArrangeLayers),
    ("capture-render", Probe::CaptureRender),
    ("capture-readback", Probe::CaptureReadback),
    ("connector-scan", Probe::ConnectorScan),
];

struct Stats {
    count: AtomicU64,
    total_us: AtomicU64,
    max_us: AtomicU64,
    buckets: [AtomicU64; 5],
}

impl Stats {
    const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            total_us: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
            buckets: [const { AtomicU64::new(0) }; 5],
        }
    }
}

static STATS: [Stats; PROBES.len()] = [const { Stats::new() }; PROBES.len()];

struct Slow {
    probe: Probe,
    us: u64,
    at: Instant,
    context: String,
}

/// The most recent samples that exceeded one frame, oldest first.
static SLOW: Mutex<VecDeque<Slow>> = Mutex::new(VecDeque::new());

/// Records a scope even when its event handler returns early.
pub struct Measurement {
    probe: Probe,
    start: Instant,
}

impl Measurement {
    pub fn start(probe: Probe) -> Self {
        Self {
            probe,
            start: Instant::now(),
        }
    }
}

impl Drop for Measurement {
    fn drop(&mut self) {
        record(self.probe, self.start);
    }
}

/// Records the time elapsed since `start` under `probe`.
pub fn record(probe: Probe, start: Instant) {
    record_with(probe, start, String::new);
}

/// Like [`record`]. `context` says what the sample was doing; it is only
/// evaluated for a sample slow enough to be kept.
pub fn record_with(probe: Probe, start: Instant, context: impl FnOnce() -> String) {
    let elapsed = start.elapsed().as_micros().min(u64::MAX as u128) as u64;
    count(probe, elapsed);
    if elapsed > SLOW_US {
        keep_slow(probe, elapsed, context());
    }
}

fn keep_slow(probe: Probe, us: u64, context: String) {
    let mut slow = SLOW.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if slow.len() == SLOW_SAMPLES {
        slow.pop_front();
    }
    slow.push_back(Slow {
        probe,
        us,
        at: Instant::now(),
        context,
    });
}

fn count(probe: Probe, elapsed: u64) {
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

/// One retained slow sample: probe name, duration, age and context.
pub struct SlowSnapshot {
    pub name: &'static str,
    pub us: u64,
    pub age_ms: u64,
    pub context: String,
}

/// The retained samples that exceeded one frame, oldest first.
pub fn slow_snapshot() -> Vec<SlowSnapshot> {
    SLOW.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .map(|slow| SlowSnapshot {
            name: PROBES[slow.probe as usize].0,
            us: slow.us,
            age_ms: slow.at.elapsed().as_millis().min(u64::MAX as u128) as u64,
            context: slow.context.clone(),
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

    #[test]
    fn probes_and_their_names_stay_aligned() {
        for (index, (name, probe)) in PROBES.iter().enumerate() {
            assert_eq!(*probe as usize, index, "{name}");
        }
        assert_eq!(snapshot().len(), PROBES.len());
    }

    #[test]
    fn only_slow_samples_keep_their_context() {
        use std::time::Duration;

        let kept = |context: &str| {
            slow_snapshot()
                .iter()
                .filter(|slow| slow.context == context)
                .count()
        };
        // An ordinary sample neither evaluates nor stores its context.
        record_with(Probe::ConnectorScan, Instant::now(), || {
            panic!("context evaluated for a fast sample")
        });
        let started = Instant::now() - Duration::from_millis(40);
        record_with(Probe::ConnectorScan, started, || "timing test".into());
        assert_eq!(kept("timing test"), 1);
        let sample = slow_snapshot()
            .into_iter()
            .find(|slow| slow.context == "timing test")
            .unwrap();
        assert_eq!(sample.name, "connector-scan");
        assert!(sample.us >= 40_000 && sample.age_ms < 10_000);
        // The ring is bounded and drops its oldest entries.
        for _ in 0..SLOW_SAMPLES {
            record_with(Probe::ConnectorScan, started, || "filler".into());
        }
        assert_eq!(kept("timing test"), 0);
        assert!(slow_snapshot().len() <= SLOW_SAMPLES);
    }
}
