# Performance measurements

## Nested redraw scheduling — 2026-09-07

The nested backend requested another redraw after every render, including
frames with no damage and powered-off outputs. With no buffer swap to pace
unchanged frames, it repeatedly rebuilt the scene while idle.

Scene changes now wake a coalescing calloop source. Requests within the
nested output's advertised 60 Hz interval share a pending redraw; a one-shot
timer supplies pacing when necessary. An idle output has no recurring redraw
timer. Surface commits, captures, window raising, output configuration, power
changes and lock transitions wake rendering explicitly.

Measured with an isolated Xvfb display, Mesa software rendering, the optimized
dev profile, no application windows, and a five-second sampling interval:

| State | Before | After |
| --- | ---: | ---: |
| Idle | 81.39% CPU | Below 0.2% CPU |
| DPMS off | 113.21% CPU | Below 0.2% CPU |

CPU percentages are relative to one core; renderer worker threads can push a
process above 100%. Both after samples consumed zero measured scheduler ticks;
with 100 ticks/second, this five-second test resolves increments of 0.2%, not
literal zero CPU usage. Startup is excluded. The before binary includes the
layer-list allocation cleanup, so that cleanup is not credited with these gains.
These measurements do not establish DRM input-to-display latency or hardware
GPU performance.

Reproduce from the repository root:

```sh
guix shell -m manifest.scm -- sh tests/bench-nested.sh
```

`MINDE_BENCH_OUT` selects the artifact directory (default
`/tmp/minde-bench-nested`); `MINDE_BENCH_DISPLAY` selects the isolated X display
(default `:97`). The benchmark builds the current checkout, reports process CPU
from `/proc`, and checks that IPC still responds after powering the output on.
It does not restart or alter the user's desktop session.

Validation covers virtual keyboard text delivery to foot, virtual pointer
motion, ext-image-copy capture with grim, wlr-screencopy video recording, and
DPMS off/on. The DPMS test checks host pixels as well as protocol state to catch
missed redraws.

## Scheme focus baseline

Before further policy changes, `tests/bench-sync-frames.scm` with 10,000 focus
changes reported 116.9 microseconds per call with 12 windows and 373.5 with 100.
Each focus change sent two placements. These are synthetic measurements with
Rust primitives stubbed; status publication, rendering and client response time
are excluded. They do not explain extreme desktop latency by themselves.

Remaining investigation should measure the real event-loop and client paths,
and include status publication when profiling policy work. The historical
checkboxes in `TODO.md` include explicitly deferred work; they are not evidence
that all performance concerns have been resolved.

## DRM presentation membership — 2026-09-07

The presentation-feedback pass now uses `Space::elements_for_output` instead
of calling `outputs_for_element` for every window. The old path repeatedly
searched Space's element list and allocated a temporary output Vec per window;
the iterator walks the same cached membership directly in stacking order.

A GPU-free benchmark using Smithay's real Space implementation and synthetic
window identities/geometries measured 5,000 passes over two outputs:

| Windows | Previous walk | Direct iterator |
| --- | ---: | ---: |
| 12 | 1.518 µs | 0.443 µs |
| 100 | 26.224 µs | 3.706 µs |
| 1,000 (stress case) | 1,680.702 µs | 38.672 µs |

These figures cover membership selection only, excluding GPU rendering and
surface traversal. At ordinary window counts this is a small saving; it does
not by itself explain severe input lag. A regression test checks membership
and stacking order with overlapping outputs, parked windows, movement before
and after refresh, raising, window removal and output removal. All 72 regular
Rust tests pass; the measurement test is ignored during normal test runs.

Run the benchmark in the project Guix shell:

```sh
LD_LIBRARY_PATH="$GUIX_ENVIRONMENT/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
  cargo test --locked space_bench::bench_output_membership -- --ignored --nocapture
```

## Installed policy versus checkout

Read-only inspection of the active DRM session found an older installed build:
`wm-timing-stats` was unbound, its status publisher still wrote both files
synchronously, and its frame policy had no placement cache. Editing or pulling
this checkout does not update an already running compositor.

Running the same synthetic focus benchmark against the installed Scheme source
and the checkout (10,000 iterations, automatic compilation disabled, isolated
empty cache) gave:

| Windows | Installed policy | Checkout policy | Placements per focus, installed → checkout |
| --- | ---: | ---: | ---: |
| 12 | 216.7 µs | 118.3 µs | 24 → 2 |
| 100 | 723.8 µs | 379.0 µs | 200 → 2 |

These differences come from fixes already present in the checkout, not just
this optimization pass. Rust work and status publication are excluded from this
table. To include status generation and file writes in future policy comparisons,
set `MINDE_BENCH_STATUS=1` when running `tests/bench-sync-frames.scm`. That mode
uses a temporary runtime directory and drains deferred writes after each focus
change, so it measures the complete per-event work rather than allowing thousands
of changes to coalesce into one final write.

## Loading the main policy bytecode — 2026-09-08

A successful package build was not sufficient to prove that its bytecode was
used. The package installed `site-ccache/init.go`, but the Rust startup path
called `(load "/absolute/path/init.scm")`. Guile's absolute loader searched for
a differently named compiled file and then checked its autocompile cache.
With automatic compilation disabled and an empty cache, the main policy ran in
the evaluator. A populated developer cache could hide the problem.

Bundled initialization now uses `load-from-path`, which finds `init.go` through
the compiled load path and checks freshness. Explicit custom init files keep
absolute-path loading, including files also named `init.scm`. The default init
path now follows the resolved Scheme directory instead of a build-time checkout
path that does not exist after installation.

The isolated keyboard benchmark inspects `program-sources` to assert the intended
execution mode, then measures 1,000 ordinary unbound key dispatches through the
real `wm-handle-key`. One fresh-cache run measured 2.31 µs per interpreted call
versus 1.43–1.48 µs compiled; a repeat measured 2.47 versus 1.61–1.64 µs. These
numbers include the same benchmark-loop overhead and exclude IPC startup and
client rendering. They are modest savings for this short path, not a claim to
have measured end-to-end desktop latency.

```sh
guix shell -m manifest.scm -- sh tests/init-bytecode-e2e.sh
guix shell -m manifest.scm -- sh tests/bench-key-dispatch.sh
```

The nested fixture gives the compositor a fresh cache. Set
`MINDE_NESTED_PACKAGE=/gnu/store/...-minde-...` to test an installed package's
binary, Scheme source, compiled cache and runtime library paths instead of the
development build. `MINDE_NESTED_INIT` selects a custom init for regression tests.

Actual package builds also exposed a missing SRFI-1 builder import in bytecode
installation and missing X11 runtime library paths for the advertised nested
backend. Both are fixed in `guix.scm`.

## Interactive resize dispatch

Xdg resize grabs now defer their latest requested size to one idle callback per
event-loop dispatch. A synthetic regression feeds 1,000 distinct sizes before
flushing: the queue produces one callback and the final size, instead of 1,000
immediate configure attempts. Returning to the previously sent size produces no
configure. Release still sends its final size immediately and cancels the queued
callback; grab replacement also cancels pending work.

This is a deterministic work-count check, not a real pointer latency benchmark.
Events arriving in separate dispatches still send separate configures. There is
no fixed frame delay and the X11 resize path is unchanged.

`tests/resize-e2e.sh` exercises Super+right-drag with a real Foot client in an
isolated nested compositor. It checks the final xdg configure and saved floating
rectangle after a burst/release, then checks that a held drag delivers a configure
before release. The client disables terminal-cell snapping to make the pixel
assertions deterministic. Both scenarios pass; the gate is in `make check-e2e`.

In the same four-motion burst plus release, the pre-batching package
(`51l6p34f75c41jp8lk8fqdrxzw0lnwf5-minde-1.0.0-rc1`) sent five xdg toplevel
configures: 650×410, 660×420, 670×430, 700×500 while resizing, then 700×500 on
release. The updated development build sent one, the final 700×500 release.
The separate held-drag scenario sent an in-progress and a final configure in
both builds. These are observed protocol counts for this controlled run;
event scheduling can change batching, so this is not a universal 80% reduction
or a latency measurement. Logs: `/tmp/minde-resize-before/foot.log` and
`/tmp/minde-resize-e2e/foot.log`.

## DRM frame callback selection

The parked-window check now runs only on the first output, and only for windows
that do not overlap that output. Previously every window queried output geometry
on every output pass, even when its callback eligibility was already known.
Hidden windows still receive callbacks through the first output.

The GPU-free `bench_frame_callback_selection` benchmark uses real Smithay Space
geometry and checks identical selected window IDs before timing 1,000 passes:

| Windows, two outputs | Before (µs/pass) | After (µs/pass) |
| --- | ---: | ---: |
| 12 | 5.132 | 2.313 |
| 100 | 46.150 | 21.726 |
| 1,000 | 1182.632 | 923.064 |

The fixture includes windows on each output, spanning both, and parked offscreen.
Timing includes collecting selected IDs, but excludes client surface traversal
and GPU work. The remaining `element_geometry` lookup is linear per window.
Cached output membership cannot simply replace this check: Smithay refreshes it
from bounding boxes, whereas this callback path uses current window geometry.

```sh
guix shell -m manifest.scm -- cargo test --locked bench_frame_callback_selection -- --ignored --nocapture
```

## Status JSON string writes

Ordinary JSON strings (keys, titles, application IDs) now use one bulk `display`
instead of one port operation per character. Strings containing quotes,
backslashes or control characters retain the escaping writer. Differential tests
cover empty strings, Unicode and all 32 control characters.

A fixed real status object, serialized 20,000 times in separately compiled
processes, measured 141.88 µs/object before and 114.87 µs/object after; a repeated
after run measured 110.75 µs/object. These exclude filesystem I/O and state
construction. An attempted in-process binding-swap comparison was rejected:
a guard demonstrated that Guile had inlined the private writer.

The complete 12-window focus benchmark including real per-event status writes
measured 864.2 → 837.5 µs/event; at 100 windows, 1118.0 → 1089.2 µs/event. These
small end-to-end differences may include filesystem noise. The isolated JSON
measurement establishes the serializer improvement, not desktop latency.

```sh
guix shell -m manifest.scm -- sh -c 'make compile-scheme && GUILE_AUTO_COMPILE=0 GUILE_LOAD_COMPILED_PATH="$PWD/build/ccache" guile -L scheme tests/bench-status-json.scm'
```

Run each source revision separately; replacing a private module binding at
runtime does not reliably replace its compiled callers.

## Integrated package validation

The combined changes build successfully as
`/gnu/store/fs8cal1hqbzxg4xr8vvsg63nb917zs2y-minde-1.0.0-rc1`, with revision label
`1ce1abe-performance-integrated-20260908`. Package contents, compiled main policy
and custom init loading, real-client resize, virtual input, both capture
protocols, and output-power protocol/pixel checks all pass against this artifact.

The packaged nested compositor again recorded no process CPU ticks during each
five-second idle and powered-off sample: below the 0.2% single-core measurement
resolution. This does not measure the active DRM desktop or input-to-display
latency. Logs are in `/tmp/minde-integrated-validation.log`; CPU results are in
`/tmp/minde-integrated-cpu/measurements.txt`.

The full source-tree `./check` gate also passes after regenerating API and package
documentation. The API inventory and signatures were checked unchanged; the API
reference hash was refreshed for current source links and the already-implemented
`focus-window-by-id!` return-value description. The package has not been installed
into or used to restart the active desktop.

## Distinct window counts in status

`group-status-summaries` now counts unique window IDs with a hash table instead
of building a deduplicated list. Mirrored/repeated IDs retain their original
counting semantics. Empty lists and large exact integer IDs are covered by tests.

The compiled `tests/bench-window-count.scm` benchmark includes 25% duplicate IDs:

| Distinct IDs | List (µs) | Hash (µs) |
| --- | ---: | ---: |
| 12 | 15.71 | 10.52 |
| 100 | 584.42 | 90.33 |
| 1,000 | 60792.98 | 1088.26 |

A sequential, separately compiled before/after run of 2,000 focus changes with
100 windows and actual status-file writes measured 4113.3 → 3795.0 µs/event
(about 8%). Absolute timings were higher than earlier runs, so those earlier
numbers are not used as this change's baseline. Maximum event time did not
improve (18.96 → 23.08 ms); this is an average-work reduction, not evidence of
improved tail latency. Detailed results: `/tmp/minde-count-ab.log`.

The nested render probe now also records actual redraw scopes. Its idle
benchmark observed one startup render before/after the idle sample, and two
renders before/after the powered-off sample: zero recurring redraws in either
five-second interval. The startup render took 269.5 ms in the software-rendered
fixture; this is initialization, not a steady-state frame timing.

These counting and measurement changes postdate the integrated package above.

## Profile-guided JSON escaping scan

A 1 kHz Guile CPU sample of the 100-window, status-enabled focus workload
attributed 6.39% self time to the per-character JSON escape predicate. The
bulk-string fast path now scans with a precomputed character set containing
exactly U+0000–U+001F, quote and backslash. It avoids the Scheme predicate call
for each character while preserving the existing escaping writer.

A fresh pair of separately compiled 20,000-object serialization runs measured
419.67 → 239.06 µs/object (43%). These absolute times differ from earlier runs;
only this contemporaneous pair is used for the comparison. All status tests,
including Unicode and each control character, pass. Profile and paired timing
results are in `/tmp/minde-status-profile.log` and
`/tmp/minde-json-charset-{before,after}.log`. No input-to-display latency claim
is made from this serializer benchmark.

Status publication also reuses the output snapshot taken for its no-output
check when constructing the JSON body. A counting regression verifies one
`wm-outputs` query per publication instead of two. This avoids a redundant FFI
conversion; no separate latency improvement is claimed for this small change.

## Active DRM session after reconfiguration (2026-09-09)

The running executable and both Home/system profile links now point to
`/gnu/store/gbd7ys8rky9s1lnqwjqrnx5j3m7wj524-minde-1.0.0-rc1/bin/minde`.
The runtime identifies the backend as udev. The package version labels itself
`local-checkout`; the store path, rather than that label, identifies this build.

A read-only 20.03-second sample subtracted timing counters at its boundaries
and checked that the process identity and executable did not change:

- CPU: 2.1% of one core during the observed desktop activity.
- Render: 397 calls, mean 649.76 µs. Of those, 378 were at most 1 ms,
  nine were 1–4 ms, ten were 4–16.6 ms, and none exceeded 16.6 ms.
- Applied commands: 11 calls, mean 29.27 µs, none above 1 ms.
- Key dispatches: zero; this interval cannot establish input latency.

This is an active desktop observation, not a controlled idle benchmark or a
before/after comparison. Render duration measures compositor work, not the
complete input-to-display path. Raw aggregate results are in
`/tmp/minde-live-drm-sample.json`; no window titles or input contents were saved.

## Avoid allocating unchanged placement rectangles (2026-09-09)

`apply-placements!` now compares cached rectangle coordinates directly, avoiding
four temporary cons cells for each checked tiled placement. Changed placements
still populate the same rectangle cache, and floating/failed placements retain
their invalidation behavior. A primitive-boundary regression covers each
coordinate, repeated floats, and retries after an unknown-window result.

A separately compiled before/after focus benchmark with installed Guile 3.0.9,
a fresh temporary bytecode cache, and 1,000 focus changes measured:

| Windows | Before (µs/change) | After (µs/change) |
| --- | ---: | ---: |
| 12 | 24.2 | 22.9 |
| 100 | 81.1 | 77.2 |
| 1,000 | 653.1 | 532.3 |

Both variants sent exactly two placements per focus change. This isolates policy
synchronization with stubbed Rust primitives; status writes and client rendering
are excluded. The small-window differences are modest, while the large-window
case improves by about 18%. Maximum timing did not improve, so no reduction in
tail latency is established. Results: `/tmp/minde-placement-ab.log`.


### StumpWM-style gaps (2026-09-09)

Gap policy runs during layout synchronization, not rendering. With gaps
disabled, transient metadata is not queried. With StumpWM gaps enabled, one
batch supplies transient IDs per sync; placements retain the existing cache.

A compiled Guile comparison against `0043976`, 3,000 focus changes per case,
with status writes disabled:

| Windows | Before | Gaps disabled | Gaps enabled (5 inner / 10 outer / 20 head) |
| --- | ---: | ---: | ---: |
| 12 | 22.1 µs | 20.7 µs | 24.8 µs |
| 100 | 67.8 µs | 65.8 µs | 67.8 µs |

Every case sent two placements per focus change. These short sequential
samples are a regression check, not evidence of a speedup. The Scheme
benchmark stubs the Rust metadata lookup; it does not measure its window scan
or input-to-display latency. Set `MINDE_BENCH_GAPS=1` with
`tests/bench-sync-frames.scm` to exercise the enabled policy.

The isolated nested idle benchmark, both disabled and enabled, recorded no
CPU ticks in each five-second idle and powered-off sample (below 0.2% of one
core at this sampling resolution). Render counts were unchanged across each
sample. Use `MINDE_BENCH_GAPS=1 sh tests/bench-nested.sh` for the enabled case.
The gaps e2e test verifies actual background pixels, border alignment,
client configure sizes, toggling, and fractional output scaling at 1.5.
