# Application compatibility and soak testing

The test layout separates the small always-run compatibility gate from heavyweight
toolkit/browser checks. Tests never require every application in one Guix
environment.

Minde is a Wayland compositor. These tests host its nested backend under
Xvfb, a virtual X server, so they can run without using the live desktop.
Native Wayland clients connect to the nested compositor's Wayland socket;
X11 clients connect to its separate Xwayland server. `MINDE_NO_XWAYLAND=1`
disables that compatibility server for native-only scenarios, but Xvfb is
still required by the harness. Suites that explicitly test Xwayland need it
enabled.

Raw Wayland protocol regressions share the Rust client in
`tests/clients/protocol/`, built with `cargo build --example protocol-client`.
The nested test scripts build it automatically and select the required
scenario. Control-client and selector tests use Scheme and run through
`make check-control`; `make check-control-e2e` exercises them against the
compositor.

## Resource policy

- One application scenario runs at a time.
- Every client runs in a dedicated process group with a 20-second timeout and
  forced cleanup.
- Each run receives an isolated temporary home, cache, config, state and
  Wayland runtime directory. The directory is removed when the compositor
  stops, including on script failure or interruption. Repeated cleanup is safe.
- Xvfb must acknowledge readiness on the requested display through its own
  descriptor. An occupied display fails startup instead of attaching the test
  to another server.
- Client lifetime bounds log growth. The fixed scenario inventory creates at
  most one log and one screenshot per application, overwriting the same paths
  on the next run.
- `make check-all` runs only foot, wl-clipboard and xterm. Browsers, Electron,
  Firefox, Emacs, toolkit demos and layer-shell programs are separate targets.
- The soak runner records compositor RSS on every iteration and fails after
  256 MiB of growth by default (`SOAK_MAX_RSS_GROWTH_KIB` overrides it).

The Guix store downloads a package only when its specific batch is requested.
Do not assemble a monolithic application manifest.

## Scenario inventory

| Scenario | Surface | Default gate | Verification |
|---|---|---:|---|
| foot | native Wayland | required | managed toplevel appears |
| wl-clipboard | data device | required | copy/paste MIME round trip |
| xterm | Xwayland | required | managed X11 toplevel appears |
| GTK 3/4 | native Wayland | optional | toolkit demo maps |
| Qt 5/6 | native Wayland | optional | matching-major QML window maps |
| Electron, Chromium | native Wayland | optional | Ozone/Wayland window maps |
| Firefox | native Wayland | optional | isolated-profile window maps |
| Emacs PGTK | native Wayland | optional | clean PGTK frame maps |
| SDL2/LÖVE | native Wayland | optional | SDL Wayland window maps |
| swaybg | layer shell | optional | background namespace maps |
| fuzzel | layer shell | optional | launcher namespace maps |
| swaylock | session lock | manual | locked presentation is confirmed; runs last because locker death preserves the lock |
| eww | layer shell | optional | bar namespace maps |

Normal runs record unavailable optional scenarios as `skip`. Set
`MINDE_APPS_STRICT=1` to turn a selected missing scenario into a failure.
Swaylock may be unavailable because of an identified PAM/account startup
failure; a running locker that never confirms, protocol errors and unexplained
exits fail the scenario. The broad `tests/e2e.sh` suite also verifies that killing
the actual foreground locker leaves the session locked.
Machine-readable results are written to
`/tmp/minde-applications/results.json`; JSONL, per-client logs and bounded
screenshots remain beside it after failures.

`tests/nested-harness-test.sh` runs headless checks in the default static gate:
colored log fields, repeated teardown, filesystem failures, signal exits,
background client PIDs and locker error classification. Its `display` and
`scenario` modes run in `make check-e2e`, checking a real display collision and
cleanup after an ordinary command failure or TERM in the main end-to-end script.
Use `MINDE_HARNESS_DISPLAY` for these checks, `E2E_DISPLAY` for the broad suite,
and `MINDE_APPS_DISPLAY` for the application matrix when selecting private
display numbers.

## Small local batches

Core release gate:

```sh
make check-apps-core
```

Run toolkits separately so Qt and GTK closures are not combined:

```sh
MINDE_APPS_FILTER=gtk3 MINDE_APPS_STRICT=1 \
  guix shell -m manifest.scm xorg-server imagemagick jq util-linux \
  gtk+:bin -- sh tests/applications.sh

MINDE_APPS_FILTER=qt6 MINDE_APPS_STRICT=1 \
  guix shell -m manifest.scm xorg-server imagemagick jq util-linux \
  qtdeclarative qtwayland -- sh tests/applications.sh
```

Qt 5 uses the same command with `qt5`, `qtdeclarative@5` and `qtwayland@5`.
Other useful single-package filters are:

- `sdl2` with `love`
- `electron` with `electron`
- `chromium` with `ungoogled-chromium`
- `firefox` with `firefox`
- `emacs-pgtk` with `emacs-pgtk`

The layer-shell batch remains relatively small:

```sh
guix shell -m manifest.scm xorg-server imagemagick jq util-linux \
  swaybg fuzzel swaylock eww -- make check-apps-layer
```

## Virtual input

`sh tests/virtual-input-e2e.sh` checks virtual keyboard typing and pointer
protocol behavior in a private Xvfb/winit session. It types Unicode into a
terminal and verifies clipboard preservation. Raw keyboard clients decode
temporary keymaps and check restoration, held modifiers, layout groups and
focus and lock cancellation with both legacy and sealed keymap descriptors.
When Xwayland is enabled, an xterm check verifies Unicode, subsequent ordinary
typing and clipboard preservation through the compatibility server too.
Rust clients exercise v1
and v2 pointer creation, output-resource release, zero and oversized absolute
extents, output position/scale/rotation changes, scroll frame boundaries and
the exact protocol errors for invalid axes and sources. Rust unit tests map
two Smithay outputs to check the requested-output association, shared edges,
disabled-output fallback and restoration.

`sh tests/input-config-e2e.sh` checks the Scheme input-rule API, including
output overrides, automatic mapping and invalid values. Rust mapping tests
use multiple Smithay outputs to cover device associations, ordered rules,
internal-panel fallback, reconnects, all eight output transforms, fractional
scale and exclusive output edges. Nested host input keeps its window-relative
coordinates despite the framebuffer's flip. Physical libinput association and
touch/stylus delivery are covered by the hardware checklist.

## Idle time and inhibition

`sh tests/idle-e2e.sh` checks protocol advertisement and a real swayidle
timeout. Scheme initialization samples idle time before backend creation or
input, and a native Rust client checks elapsed time, queries that leave
activity unchanged, pointer activity reset and the public `idle-ms` wrapper.
The clock uses the same monotonic origin as runtime uptime. Physical session
resume remains a hardware check.

Rust clients also create multiple inhibitors on one surface. Both destruction
orders must preserve inhibition until the last inhibitor is gone. Surface
destruction, disconnect and SIGKILL must release
inhibition without input, while other clients' inhibitors remain effective.
Input-only notifications prove that time passes while ordinary notifications
are suppressed. Exact idle/resume histories also check that destroying stale
inhibitor resources cannot cancel a replacement surface's inhibition.

## Fullscreen and pointer constraints

`sh tests/fullscreen-pointer-e2e.sh` runs Rust and C protocol clients in a hidden
Xvfb/winit session. It checks native and X11 fullscreen (including a pre-map
EWMH hint), client self-resize against tiling, fullscreen panel/border pixels
and input routing, null and explicit pointer confinement, input regions,
subtracted holes, Xwayland pointer grabs, relative motion during a lock and
cursor-position hints on unlock. Rust clients check focus through implicit,
move/resize and drag-and-drop grabs; committed hints inside current input
regions; inactive hints; independent clients; and retired one-shot resources
whose later requests must not affect a replacement lock. It also checks
consecutive absolute-input lock deltas, warp resets, keyboard focus across
fullscreen group switches and fullscreen resizing when output scaling changes.
Application requests from X11 clients in hidden groups or behind a focused
peer must preserve group selection, foreign activation and real keyboard
delivery. Explicit taskbar fullscreen must still activate its target.
It is part of `make check-e2e` and uses the normal development manifest; all
client lifetimes are bounded. This does not
replace testing an actual game on the DRM/libinput backend.

## Nested rendering failures

`sh tests/winit-errors-e2e.sh` loads a test EGL proxy only into a private
nested compositor. It injects temporary context-activation failures and
surface loss during swaps, then checks recovery through actual window pixels
and Wayland frame callbacks. Repeated failures must retry without a new
client commit and must not acknowledge an unpresented frame. The gate covers
the desktop, lock surfaces, lock confirmation, output disable and DPMS off,
and verifies clean shutdown after context loss during rendering or swapping.
It runs in `make check-e2e`; no fault-injection code is built into Minde.

## DRM repaint scheduling

The Rust unit gate checks unexpected display completions against real calloop
idle callbacks and timers: pending repaint work must survive, while an expected
completion cancels only its own watchdog. It also checks bounded render-failure
retries, recovery, diagnostic throttling and timer cancellation. These checks
exercise scheduling without a DRM device; they do not emulate KMS or libseat.
Physical presentation and VT switching still require the
[hardware validation checklist](hardware-validation.md).

GPU-free layout tests exercise the production window selector across output
power loss, disable, re-enable and placement changes. Parked windows transfer
to the first output that can render, while windows confined to a powered-off
output receive no callbacks. The tests check eligibility; physical DRM frame
delivery remains a hardware check.

`sh tests/frame-callbacks-e2e.sh` runs a native frame-paced raster client and
records callback counts plus client/compositor CPU usage. It checks low-rate
hidden updates on an idle desktop and during visible redraws, immediate
recovery on reveal, lock and DPMS suspension, child-only requests and unmap.
Its surface-commit scenario measures render and layer-arrangement counts
with a hidden client repainting at 20 Hz and a bar repainting at 1 Hz.
It also checks exclusive-zone changes, configure acknowledgements,
keyboard-only layer changes and synchronized child commits.
Both backends use the same callback selection and hidden timer; the nested
suite does not measure a physical GPU or DRM presentation.

## Output configuration protocol

`sh tests/output-management-e2e.sh` combines normal `wlr-randr` requests with
raw protocol clients. Binding a second manager must preserve the first
client's serial and pending configuration; a real layout change must notify
both clients and cancel configurations using the old serial.

Long-lived power controllers must receive the reset to `on` when a powered-off
head is disabled. Configuration tests and unchanged power states must not
notify, and controllers must remain usable after re-enable and agree with a
fresh binding. The nested backend exercises this lifecycle; actual DRM power
switching remains a hardware check.

Malformed requests are checked for the exact error object and code: invalid
scale, transform, custom mode and adaptive-sync values; repeated properties;
mixed advertised/custom modes; a mode advertised for a different head
resource; omitted heads; duplicate enable/disable; and every setter after
apply or test. A healthy client must still work after each rejection. Valid
properties and nonfatal backend refusals are checked separately, along with
destruction of the configuration's child objects. Scheme's single-head API
continues to accept partial changes; completeness is a protocol requirement.

## Screen capture lifecycle

`sh tests/screencapture-e2e.sh` checks both capture protocols and file captures
in a private nested compositor. Raw clients batch capture requests with lock,
disable and power-off requests to exercise transitions before the next render.
Each affected frame must fail once; its buffer must remain untouched, and it
must never receive a later `ready`. File captures report `screenshot failed`
without creating the destination file. New requests succeed after recovery.

Disabling or removing an output stops its ext capture sessions. Lock and DPMS
off preserve sessions but fail their frames. Mode, scale and transform changes
refresh ext buffer constraints and fail queued frames prepared against the old
geometry; wlr frames awaiting `copy` also fail if their source changed.
Both protocols capture the upright logical desktop at the output scale.
The suite checks dimensions, damage bounds and buffer guards after rotation,
fractional scaling and host-window resize, plus a new session on a rotated
output. Physical DRM hotplug uses the same cleanup but is not exercised here.

Pixel regressions compare live layer rendering with both capture protocols
and pointer hit-testing. They cover layer levels, mapping order, repainting an
older surface and fullscreen policy. Centered-message region captures must
equal the corresponding crop of a full frame at scales 1 and 1.5; the selected
region must also contain message pixels, so an omitted overlay cannot pass.
The capture gate generates a small Xcursor theme with odd, asymmetric hotspots
and checks cursor bounds at scales 1, 1.25, 1.5, 1.75, 2 and 2.5, including
returning to cached images. Cursor-free captures must omit the fixture. This
exercises Minde's shared themed-cursor renderer; the nested backend delegates
its live cursor to the host, so live DRM cursor placement remains a hardware
check. The generated binary theme stays in the test output directory.

## Gamma control lifecycle

`cargo test handlers::gamma_control` uses real Wayland resource identities and
events with a mock CRTC device. It covers rejected duplicates within one client
and across clients with identical wire object numbers, stale controls
after output removal, deferred writes and restores, retained original ramps
across owner replacement, replay errors, and invalid descriptor handling.
These checks run in the normal Rust gate and do not alter a display's gamma.
They verify compositor state and hardware-call sequencing; driver-specific
gamma persistence is outside their hardware-independent coverage.

## Portable and historical keymaps

`make check-e2e` runs two nested sessions. The broad behavior suite retains
the historical test map so it can reach old commands; a second reduced-map
session exercises contextual `?`, direct `w 0`/`f 0` numbering, and terminal
launch through the actual input path. Neither session reads the host's
personal keymap or startup configuration.

## Soak runner

A quick deterministic smoke run:

```sh
make check-soak SOAK_ITERATIONS=3
```

The release-duration run is:

```sh
make check-soak SOAK_MINUTES=60
```

Each iteration maps/unmaps a client, reloads configuration, creates/deletes a
group, changes the clipboard, simulates a two-output hotplug and restores the
single output, then validates schema-v1 state. Results and RSS measurements
are retained in `/tmp/minde-soak/results.json`.

Touch cancellation has headless Rust coverage using real Wayland surfaces and
`wl_touch` resources: completed and pending frames, stationary contacts,
multiple clients, pending releases and fresh gestures after cancellation.
The fullscreen/pointer gate also checks click focus against actual keyboard
events, toplevel activation, Scheme frame/float selection, control snapshots
and commands. It covers disabling the window-focus hook, empty space, and
noninteractive, on-demand and exclusive layer surfaces. Destroying only a
focused layer role, with no desktop window, must send inhibitor inactive and
restore compositor shortcuts while the client and its resources stay alive.
With an input method bound, text-input enter/leave must form one balanced pair;
without one, neither event is sent. A Rust X11 client verifies initial key
delivery and its restoration after an exclusive layer closes. Touch and stylus use the
same target-selection helper; physical tap delivery remains a hardware check.
It also checks that X11 surface association and session unlock never grant
keyboard focus to the desktop underneath an exclusive layer. The output
management gate closes layers on disable, rejects new roles on disabled or
missing outputs, and accepts fresh layers after re-enable. Test-only and
rejected configurations preserve layer ownership; physical unplug remains
part of the hardware checklist.
Popup regressions inspect real configure geometry for toplevel and layer-shell
parents on an offset output, including nested menus, flip/slide adjustments,
reposition requests and clients forbidding adjustments. Rust tests cover output
selection for spanning windows, negative origins, fractional scale and rotation.
The fractional-scale gate inspects preferred-scale events on toplevels, layers,
nested popups and their subsurfaces while changing output scale in both
directions. Late child bindings and unchanged-scale event counts are checked.
Rust tests cover mixed-scale output selection after movement, resizing,
disable and re-enable without relying on cached output membership.
Tablet-awareness tests use real Wayland tool bindings over socket pairs,
including multiple bindings, newly registered tools, surface destruction and
client disconnects. Repeated queries across clients with large object tables
assert that cached positive and negative results avoid further object scans.
Clients with foreign userdata exercise the same cache path used by Xwayland.
Physical touch/tablet events, Java, Wine, games, IME, drag-and-drop and multi-GPU retain
manual/optional application coverage. Session-lock ownership and input
isolation are exercised by `tests/session-lock-e2e.sh` using
`ext-session-lock-v1`, including inhibitor inactivation when the focused lock
surface is destroyed. It checks one owner-loss notification on disconnection
or an unconfirmed lock's destruction, and launches a real replacement locker
through the default Scheme recovery policy. The replacement takes ownership
and unlocks without exposing desktop input. Scheme tests drive retry timers,
backoff, failed starts, cancellation and stale callbacks.
Its keyboard filter scenario checks press/release pairs
through prompts, menus, focus changes, shortcut inhibition and locking, plus
held modifiers, keypad Enter and compositor repeat. VT suspension balances held
physical keys and cancels repeat; actual VT switching still needs a hardware
session. Its input-lifecycle scenario sends bursts of 500 move motions and
500 resize commits, comparing bounded geometry traffic on the event socket
and journal. It checks immediate inspection, pause/release delivery, the
client's final resize size, lock cancellation and queued-event removal on
unmap. The fullscreen/pointer gate also unmaps X11 windows
during move/resize grabs and terminates only its private Xwayland child.
Its Rust X11 client changes legacy and UTF-8 titles and WM_CLASS without
repainting, checking Scheme hooks and foreign-toplevel property/done events.
It covers repeated values, title precedence, deleted properties and remapping.
It verifies window and menu cleanup, foreign handles, failed runtime status,
child `DISPLAY`, and continued native input. Automatic Xwayland restart
remains unimplemented.

`tests/clipboard-e2e.sh` checks large clipboard, primary-selection and drag
payloads, bounded receive floods and stalled-transfer cleanup. Its raw drag
target accepts text and file offers, finishes or destroys accepted offers,
rejects an offer, and locks during a drag. The result registry must match
exactly one terminal event for each token. Source unit tests also cover late
cleanup after history eviction. Session-lock tests verify that cancelled drag
timers cannot resume motion after unlock and that disappearing targets release
their automation button.

`tests/foreign-toplevel-e2e.sh` checks actual output enter/leave and done
events after floating moves, edge overlap, fullscreen and output disable/enable.
Delayed buffer commits distinguish configured sizes from committed geometry;
unchanged associations must stay silent. The fullscreen/pointer gate covers
the corresponding X11 placement and fullscreen paths. Unit tests cover
overlap across two outputs with different scales, spanning-window resize,
fullscreen bounds, rotation, negative origins and removal. Physical
multi-monitor dragging remains a hardware check.

The foreign-toplevel gate also sends native xdg fullscreen requests from
hidden groups and unfocused windows. It checks configure replies for deferred,
repeated and cancelled requests, parked geometry, keyboard delivery, invalid
IDs and explicit taskbar activation. Scheme tests cover stale float mirrors,
fullscreen-owner replacement and removal of the separately focused window.
Its lifecycle client checks that floating flags reject unknown, future and
retired IDs, follow real map/unmap lifetimes, and leave other windows intact.
Another client creates its decoration object after the initial xdg configure,
checks the decoration mode arrives before the next xdg configure, then attaches
its first buffer and maps. Client-side preferences retain server-side policy.
