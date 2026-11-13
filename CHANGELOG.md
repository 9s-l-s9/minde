# Changelog

All notable user-visible changes will be recorded here. This project has not
yet made a public release; breaking changes remain possible before 1.0.

## Unreleased

Target version: `1.0.0-rc1`.

### Changed

- `wm-send-string` and `wm-type` type characters outside the active layout
  through temporary client keymaps. They preserve the clipboard and no longer
  depend on a client's Ctrl+V binding (#144).
- Portable bindings, including nested maps and global shortcuts, now come
  from validated configuration data. Typed command arguments and `#f` unbinding
  are supported. Return opens the run prompt; terminal, brightness and audio
  launchers belong in personal configuration. The historical map loads only
  from its optional file, and its utility commands remain discoverable (#163).
- The desktop selector and control tests now use Scheme, and protocol test
  clients share a Rust harness. CI metadata is generated in Scheme, with
  build manifests and package inputs updated accordingly.

### Fixed

- Screenshots stay below `MINDE_SCREENSHOT_DIR` (by default the runtime
  directory's `minde/screenshots` child), reject symlinks and existing files,
  and publish complete PNGs without overwriting concurrent captures. Typed
  group actions accept unpadded names and return the stored group name (#161).
- Surface commits redraw their affected outputs; hidden windows keep their
  throttled frame callbacks without waking the renderer. Surface ownership
  is cached across commits and cleared on unmap. Content-only layer commits
  no longer recompute usable areas; `wm-timing-stats` includes layer
  arrangement counts (#153).
- Receipt allocation and status checks update revisions without rebuilding
  desktop snapshots. Snapshot facts are cached with detached replies and
  indexed frame locations. `mindectl wait` and `watch` use event wakeups,
  bounded safety checks and polling backoff when events are unavailable (#164).
- Interactive move/resize geometry notifications are combined at 50 ms
  intervals, with immediate updates on release and cancellation. A settled
  event reports completion, waiting for native clients' final resize commit.
  Inspection stays current, and unmapping discards queued notifications (#157).
- Documentation links to the current session-locking commands, personal
  configuration load order and upstream provenance. The README and testing
  guide distinguish Wayland sessions, Xwayland compatibility and Xvfb test
  hosting (#151).
- IPC helpers, `mindectl` and status publication agree on the private
  `${TMPDIR:-/tmp}/minde-UID` fallback, treating empty runtime and temporary
  directory values as unset. Fallback compatibility status also stays in
  that private directory, and an empty status-path override is ignored (#94).
- Vendored release archives must resolve the complete locked Cargo graph
  offline using their extracted sources and an empty cache. Missing or
  incompatible crates and outdated lockfiles fail validation; CI vendoring
  also uses `--locked` (#93).
- Generated-document checks compare the testing reference in a temporary
  directory instead of overwriting `doc/testing.md` before comparison.
  Stale text and changed command inventories now fail validation while
  preserving the source files (#92).
- Tearing control rejects duplicate objects on a surface with the required
  manager error, including requests through another manager binding.
  Destroying a control permits replacement; destroying its surface leaves
  the control inert. Nested protocol tests cover these lifetimes (#90).
- Placement rules separate exact app IDs from title substrings and combine
  explicit predicates with AND. Remember refuses windows without an app ID;
  forget no longer removes app rules because an unrelated title contains
  their text. Version-1 rules migrate to version 2 with the deprecated
  single-string semantics, and `check-config` reports shadowed rules (#159).
- Reconnecting an output restores each group's frame layout, window stacks
  and selections. Windows remain available on a surviving head while it is
  absent; closed, floated or deliberately moved windows stay out of the
  restored assignment. Connector IDs survive Output replacement, and dynamic
  groups retile after borrowed windows return (#160).
- DRM connector scans retry outputs when modes or a display controller become
  available, including hotplug while the session was paused. Connected heads
  refresh their modes and monitor identity while retaining layout, explicit
  disable and power state. Removed dock connectors release their controller
  assignments (#69, #70, #71).
- DRM device setup failures and removal release the libseat lease, event
  registration and GPU resources, including cached renderers. Explicit sync
  belongs to the successfully initialized primary GPU and is withdrawn when
  that device disappears; duplicate device notifications preserve live heads
  (#58).
- The security model now states the existing trust requirement for every
  connected Wayland client, lists privileged interfaces and their limits,
  and explains why socket access does not isolate sandboxed apps (#133).
- Documentation checks reject missing ripgrep and failed discovery/extraction
  instead of reporting success. Demo checks reject nonnumeric or nonpositive
  video durations and failed manifest extraction (#129, #130).
- Development Scheme caches rebuild together when inputs change, so callers
  cannot retain inlined values from older dependencies. Rebuilds ignore
  inherited and user bytecode caches, discard removed outputs, and detect
  content edits even when source timestamps are preserved (#125).
- Verification targets propagate archive build/comparison and hardware-report
  failures. Static checks require ShellCheck and include `scripts/run-nested`
  and `scripts/minde-agent`, so omitted tooling or launchers cannot silently
  pass (#123, #124).
- Local releases reject missing, empty, or duplicate Unreleased changelog
  sections before builds or output cleanup. Release notes retain the
  validated changes and previous artifacts survive validation failures (#122).
- Pages CI pins actions and Guix inputs, uses the verified bootstrap and
  filtered TLS roots, and grants Pages/OIDC write permissions only to the
  deployment job. Checkout credentials are not retained (#121).
- Source release CI requires a version-matching tag on `origin/main`, tied
  to the checkout and triggering event, before building or publishing.
  Release candidates are published as prereleases (#120).
- Demo manifests reject empty fields and embedded record separators before
  capture starts. Setup, action, and cleanup cannot shift columns during
  shell parsing; `#t` remains the explicit no-op expression (#119).
- Malformed provider HTTP replies produce structured transport errors.
  Evaluation retains failed cases and continues writing the full report,
  including successful cases before and after a failure (#118).
- Long window titles, app IDs, and group names no longer prevent desktop
  selection. Provider descriptions are shortened while local matching and
  action arguments retain their full values (#117).
- `urgent-window` is discoverable and remains available through both live
  subscriptions and journal replay while locked. Both paths share one
  redaction policy, including suppression of unknown events and redaction
  of partial title payloads (#112).
- Runtime and generated API catalogs resolve exported bindings to their
  values, restoring callable signatures and procedure documentation,
  including re-exports. Unbound exports remain inspectable (#111).
- Failed status-file writes remain pending and retry on the next publication,
  even without a desktop change. Retries preserve the sequence number, and
  newer state replaces the pending state before a deferred write (#110).
- Control snapshots retain fullscreen state for hidden groups. Fullscreen
  setters affect the target's own group, preserve unrelated fullscreen
  owners, and leave focus unchanged when disabling or repeating a satisfied
  setting (#107).
- Control receipts accept only the exact server-issued request ID. Altered
  spellings cannot execute actions or consume their receipts, and oversized
  numeric suffixes are rejected before parsing or retention (#108).
- Control errors return flat association lists for their details, matching
  argument-validation errors and preserving the same shape in stored and
  replayed receipts (#106).
- `minde-msg` validates timeout syntax and bounds before IPC, rejects missing
  arguments with usage status 2, and accepts `--` before literal message text.
  Quoted text and trailing newlines are preserved (#114).
- Diagnostic state queries and report requests use the bounded IPC client,
  returning transport status 3 without an uncaught backtrace. A failed initial
  state request no longer leaves an empty report directory, and concurrent
  report creation cannot overwrite another report (#115, #116).
- IPC clients handle pending connections and transient socket errors within
  their original deadline. Interrupted waits resume, and transport failures
  only report possible effects after request bytes have been sent (#109).
- Nested tests clean up owned processes and fresh runtime directories on
  failures and signals, reject occupied Xvfb displays, and match colored
  log fields consistently. Repeated teardown does not signal retired PIDs
  (#126, #127, #131).
- Locker tests wait for actual lock confirmation and kill the foreground
  swaylock process when checking abandonment. The application matrix runs
  its locker last and distinguishes PAM/account startup failures from
  protocol failures (#128, #168).
- Suspend setup errors and early exits remove the pending confirmation hook,
  allowing a fresh attempt and preventing an unrelated later lock from
  suspending the machine. Failed timeout registration also cancels setup;
  stale timeouts cannot cancel a newer attempt (#104).
- JSON logs quote non-finite floating-point fields as `"NaN"`, `"inf"` or
  `"-inf"`, preserving their diagnostic meaning without breaking parsers
  (#145).
- Scheme idle time uses a monotonic clock, advances before the first input,
  and resets when the DRM session resumes. Wall-clock adjustments no longer
  create or hide idle intervals (#146).
- Idle inhibition lasts until every inhibitor on a surface is released.
  Surface destruction and client disconnect, including SIGKILL, release
  inhibition without waiting for user input (#85, #86).
- Hidden windows receive pending frame callbacks at most once per second,
  with one shared timer that also works on an idle desktop. Visible windows
  retain normal pacing, and revealing a window bypasses its hidden delay.
  Lock, output power and unmap suppress pending hidden callbacks (#139).
- Parked windows receive DRM frame callbacks from the first rendering output
  when an earlier output is powered off. Windows on the powered-off output
  itself remain paused; output disable and re-enable use the current layout
  when choosing callback recipients (#74).
- Disabling a powered-off output reports the power reset to existing
  controllers on both DRM and nested backends. Controllers remain usable
  after re-enable; unchanged power states do not send extra notifications
  (#76).
- DRM display completions preserve pending repaint timers and idle callbacks.
  Paused sessions defer forced renders, and failed renders retry with a bounded
  delay and rate-limited diagnostics instead of running at refresh rate
  (#72, #73).
- Decoration objects created after the initial xdg configure receive their
  server-side mode immediately, allowing clients waiting for that reply to
  attach their first buffer and map (#68).
- Enabling floating state rejects unknown or retired window IDs instead of
  leaving ghost entries or premarking future windows. Disabling remains a
  harmless cleanup operation for stale IDs (#59).
- Automation drops publish one terminal result per token. Finishing or
  destroying an accepted offer cleans up transfers without overwriting the
  accepted handoff result or resurrecting evicted history entries (#51).
- Application fullscreen requests affect their own Wayland or X11 window
  without changing the active group or keyboard focus. Hidden groups defer
  fullscreen until visited, invalid IDs do nothing, and taskbar requests
  retain explicit activation. Repeated and deferred xdg requests still
  receive configure replies (#41).
- Foreign-toplevel output associations follow floating placement, fullscreen,
  drag movement and committed resize geometry for Wayland and X11 windows.
  Disabling the last output clears its associations; unchanged membership
  does not emit duplicate enter/leave events (#43).
- X11 title and class changes update foreign-toplevel handles and Scheme
  metadata hooks without requiring a repaint. Repeated values are ignored,
  and remapped windows begin with their current metadata (#42).
- Themed cursor hotspots follow fractional output scale, retaining subpixel
  precision through the image-to-logical conversion before rounding physical
  placement. Cached cursor images stay aligned across scale changes (#35).
- Layer rendering matches input order: Overlay above Top, windows above
  Bottom and Background, and newer mappings above older ones within a layer.
  Fullscreen still hides Top surfaces while preserving Overlay (#34).
- Centered messages retain their scene position in region captures, including
  fractional output scales. Centered and positioned overlays share one render
  helper (#48).
- Preferred fractional scale follows window placement and committed size
  changes immediately. Xdg popups, nested menus and subsurfaces inherit their
  owner's scale at creation and on output changes, without repeated events
  for unchanged scales (#63, #64).
- Popups use their parent window's output by largest overlap instead of the
  first output. Layer-shell menus apply positioner constraints after their
  parent is assigned, including nested popups and reposition requests (#37, #38).
- Layer surfaces on disabled or removed outputs receive `closed`, release
  keyboard focus and cannot reappear when the output returns. Output tests
  and failed configurations preserve their layers (#66).
- New layer surfaces targeting a disabled output, or created with no enabled
  output, receive `closed` instead of waiting indefinitely for configure (#67).
- Exclusive layers retain keyboard ownership when an X11 window finishes
  surface association and regain it before desktop windows on unlock (#57).
- X11 keyboard focus uses Smithay's X11 target, honoring the client's input
  model and X11 focus notifications. Closing a focused layer restores real
  keyboard delivery to the selected X11 window (#65).
- Clearing keyboard focus synchronizes text input, shortcut inhibition and
  selection access. Destroying a focused layer role without a desktop window
  restores compositor shortcuts; destroying a lock surface also inactivates
  its retained inhibitor (#91).
- Text-input focus preserves Smithay's notifications instead of duplicating
  them or sending an unmatched enter when no input method is bound.
- Click and tap focus use an overridable Scheme hook, keeping frame and
  floating-window selection, activation, borders and control state aligned
  with keyboard focus (#137).
- Pointer and touch focus honor the actual layer surface under the input:
  noninteractive layers preserve focus, on-demand layers accept it, and
  exclusive Top/Overlay layers retain it across clicks and layout sync (#57).
- Tablet samples reuse cached client awareness instead of walking every
  Wayland object on each event. Binding and destruction refresh the cache;
  pointer emulation reuses the tablet target when constraints allow (#140).
- Physical touchscreens, tablets and absolute pointers use their libinput
  output association, with internal-panel fallback, output rotation and
  logical scaling. `wm-configure-input!` accepts `#:map-to-output` overrides
  and `'auto`; mappings follow output reconfiguration and hotplug (#138).
- Virtual pointers honor their requested output and clamp absolute extents
  inside its current logical geometry. Disabled or removed targets fall back
  to an enabled output; zero extents leave the pointer unchanged (#87).
- Invalid virtual-pointer scroll axes and sources produce the protocol's
  specified errors, including stop and discrete-scroll requests (#88).
- Pointer locks and confinement activate only for the surface holding pointer
  focus, including during implicit, move/resize and drag-and-drop grabs (#80).
- Unlock hints belong to individual constraint resources and must be committed
  while active and lie inside the surface's current input region. Inactive or
  retired locks cannot reuse hints or remove a replacement constraint (#83).
- Touch cancellation clears every held contact and client focus, including
  stationary fingers from completed frames. Locking, window unmapping and
  VT suspension use the same cleanup; late motion/up events cannot revive
  the cancelled gesture (#78).
- Compositor-consumed keys also consume their matching releases, including
  across prompt, focus, shortcut-inhibitor and lock transitions. VT suspension
  releases forwarded physical keys and cancels compositor repeat (#77).
- Prompts and menus forward pure modifiers, so applications retain the correct
  held-Shift state after submission. Menus also accept keypad Enter (#79, #113).
- Development builds refresh the compiled revision after Git changes and
  tracked-file edits, including linked worktrees. Version detection leaves
  the Git index untouched and preserves incremental builds (#36).
- Scheme's `version!` displays the running binary's version and revision
  through `wm-version`, sharing the CLI string instead of a stale literal
  (#103).
- Expose and frame selection clean up on unbound keys, failed bindings,
  group/head changes and session locking. Expose snapshots belong to their
  group and head and survive repeated entry. Digits, letters and multi-key
  labels make every window or frame selectable (#162).
- Frame-overlay timers clear only their own generation of labels, preserving
  newer frame-selection and expose overlays.
- Super+Q no longer quits the compositor. The native Ctrl+Alt+BackSpace
  emergency exit can be rebound or disabled in declarative configuration;
  invalid reloads preserve it. The keymap reference and tutorial warn that
  this chord ends the entire session without a save prompt (#158).
- Nested bindings for the prefix keysym no longer re-arm the top-level map,
  and the literal-prefix escape fully leaves command mode (#95, #96).
- Frame-flash timers restore only the border color, preserving prompt/menu
  key repeat and pending which-key help (#97).
- Rebinding keys removes stale descriptions and submap metadata. Declarative
  overrides and their removal preserve the personal layer's matching help
  and nested-map relationships (#98).
- Configuration validation checks case-sensitive XKB names in bindings and
  prefix keys, normalizes modifier order and keysym aliases, and rejects
  duplicates after normalization. Offline checks and live reload use the
  same rules; invalid keys leave the active configuration unchanged (#148).
- Offline configuration checks recognize bundled control commands and reject
  bindings that require arguments. `mindectl check-config FILE --live` checks
  personal commands against the running session without applying changes;
  configuration readers disable read-time evaluation (#24).
- Configuration documentation describes the actual default-file lookup and
  shows the shipped `C-t` prefix (#105, #132).
- Output mirror diagnostics compare raw usable rectangles, so head gaps
  and span layouts do not produce false drift reports; real geometry
  disagreements remain visible (#8).
- Floating windows stranded in gaps between surviving outputs move onto
  the nearest output after geometry changes. Recovery also covers hidden
  groups and span layouts while preserving visible floats across adjacent
  monitors (#10).
- Group transitions retain the selected group record, so legacy duplicate
  names cannot redirect focus, window moves or deletion. New blank and
  duplicate names are rejected before mutation, and automatic names choose
  the first unused numeral (#15).
- Group deletion documentation now states its actual next-group fallback,
  wrapping from the last group to the first (#18).
- Title and app-ID changes compare previously reported client metadata,
  so real retitles trigger placement rules and events even under a user
  rename; repeated metadata does not retrigger float rules (#19).
- Late placement rules synchronize when following the current group.
  Rule moves and floats immediately rebuild affected dynamic layouts,
  including other heads and hidden groups after a window closes
  (#20, #21, #22).
- Placement rules respect floating target groups on map, late app-ID
  arrival and explicit placement, parking hidden floats until shown (#23).
- Fullscreen keeps keyboard focus and close commands on the displayed
  window while newly mapped windows are laid out behind it (#12).
- Failed sibling selection preserves floating focus. Focus hooks, urgency
  and `other-window!` history follow the effective focused window, including
  floats (#9, #13).
- Reverse window cycling from a cleared frame selects the last window
  (#14), and dynamic retiling preserves the selected surviving tiled window
  across group switches, layout changes and unrelated unmaps (#16).
- Layout replacement and `only` keep floating windows and other heads'
  windows out of the current head's frame tree (#4).
- Pulling a resident window no longer duplicates it. Pulling from a cleared
  frame moves its first hidden window while keeping the remaining windows
  hidden; invalid window IDs are rejected before insertion (#7, #11).
- Exchanging a focused float with the dynamic master leaves the tree,
  window order, placements and focus unchanged (#17).
- Host window resizes replace the nested output's mode list and preferred
  mode, retiring obsolete modes for output-management clients (#30).
- Custom modes with an omitted or zero refresh select the preferred mode
  at that size, otherwise the highest advertised refresh. Scheme and
  output-management requests use the same selection rules (#53).
- Output settings that would be discarded on a disabled head now fail
  with an instruction to include `#:enabled #t`; disabling alone remains
  valid (#56).
- Output-manager binds preserve other clients' configuration serials. Invalid
  configuration requests now raise the specified protocol errors, and
  destroying a configuration also destroys its head objects (#28, #29).
- Screen captures fail promptly on session lock, DPMS off and output removal
  instead of lingering until the desktop returns. Removed outputs stop ext
  capture sessions; rotation, scale and mode changes refresh constraints and
  invalidate frames prepared against the old geometry (#44, #45, #46).
- Gamma controls enforce resource ownership: rejected or retired controls
  cannot change or remove a newer owner's ramps. Night-light tables survive
  DPMS, modesets and VT changes; off-screen updates and restoration are
  deferred, replay failures are reported, and non-file payloads are rejected
  before reading (#54, #156).
- The nested backend retries temporary render and swap failures with a full
  redraw, and stops cleanly on graphics-context loss. Failed frames do not
  acknowledge a lock or send frame callbacks (#33).
- Missing runtime directories or exhausted Wayland socket names now report
  startup errors instead of panicking and writing crash logs (#32).
- Failed personal Scheme init files report the exception and source location,
  then restart once with clean bundled policy and display the error. A broken
  bundled init aborts startup. Ctrl+Alt+Backspace exits independently of
  Scheme keybindings (#141).
- X11 unmaps cancel their move/resize grabs. Losing Xwayland removes its
  managed windows, menus and foreign handles and reports a failed server;
  native clients keep running. Spawned children receive only minde's live
  Xwayland display, with `DISPLAY` removed before readiness or after failure
  (#31, #40, #84).
- X11 menus map at their requested position, and window geometry is
  refreshed when the Wayland surface association arrives after its buffer.
- Resize cancellation sends a configure that clears the resizing state and
  preserves the committed floating rectangle. Left/top-edge resizes keep
  their opposite edges anchored through delayed commits, and store the
  client's final geometry for later layout syncs (#81, #82).
- Native Wayland toplevels enter the layout only after their first buffer
  commit. Null-buffer commits remove hidden windows, focus, fullscreen and
  foreign-toplevel handles, and end their grabs and popups. Remapping uses
  a fresh window ID and preserves requests made before mapping (#39).
- Invalid `XKB_DEFAULT_*` keymaps log their requested settings and fall back
  to a plain US keymap without reusing the failing environment. If the
  fallback cannot compile either, startup exits cleanly instead of panicking
  on `BadKeymap` (#60).
- Automation drops refuse active pointer grabs and held buttons before
  moving the pointer or injecting input, preserving client drags and
  compositor move/resize operations and reporting cancellation correctly (#49).
- Invalid saved layouts and placement rules are preserved and block saves
  until reloaded successfully or moved aside. Startup failures are logged and
  shown when an output appears; failed saves leave memory unchanged (#102).
- Layout saves exclude unsaved configuration presets, and configuration
  definitions take precedence over legacy saved built-ins. Equivalent
  placement rules no longer accumulate on restart or repeated remember
  commands (#99, #101).
- Clipboard, primary-selection and automation drag transfers share
  payloads and a transfer quota instead of spawning a writer per receive.
  Stalled pipes expire, replaced selections cancel pending writes, and prompt
  pastes deliver partial text on timeout or at 64 KiB (#52, #62, #143).
- Capture and fallback presentation timestamps use system monotonic time.
  Presentation feedback handles unknown refresh rates without dividing by
  zero and reports active adaptive sync as variable refresh (#47, #152).
- Output mode configuration rejects negative, non-finite and out-of-range
  refresh values before conversion or matching. Extreme refresh rates cannot
  overflow mode comparisons or abort the compositor (#55).
- Extreme virtual-pointer wheel steps are bounded before conversion, keeping
  both legacy discrete scrolling and v8 wheel values free of overflow (#89).
- NUL-padded EDID monitor names and serials are sanitized before Wayland
  advertisement, preventing malformed display metadata from aborting the
  compositor. Empty descriptors retain the existing identity fallbacks (#75).
- Cancelling synthetic typing releases its held keys without releasing an
  already held physical modifier. Session locking cancels queued automation
  and rejects new synthetic input. Virtual pointer destruction releases its
  buttons without releasing presses owned by another device (#61, #149, #165).
- Scheme numeric arguments are checked for exactness and range before
  applying commands or allocating automation tokens. Malformed placement
  batches return `#f` without partial changes; negative or overflowing
  geometry cannot abort the compositor (#25, #26, #142).
- Keyboard and output-policy hooks accept ordinary Scheme truth values
  without terminating the compositor on a non-boolean result (#27).
- The portable prefix followed by the space bar opens the command palette
  using xkbcommon's canonical `space` keysym name (#100).
- Frame and placement tests now check actual focus, geometry and restored
  rule behavior; nested protocol checks catch errors without unrelated log
  markers, and the input smoke test reports its no-device scope (#170).
- Repeated modifier names and aliases now keep the intended key binding
  instead of adding their bits into a different modifier mask (#5).
- CI, `./check`, and `make check-rust` now share Rust build, unit-test and
  Clippy gates. Guix package checks also run Rust tests and Clippy offline
  in release mode (#169).
- Guile string conversion uses explicit byte lengths, preserving Unicode and
  embedded NULs without reading past the allocated buffer. Non-string inputs
  return failure without raising through Rust (#6).
- The opt-in unsafe REPL now requires a private, owned runtime directory,
  creates its socket with mode `0600`, and preserves existing socket-path
  entries instead of deleting them (#172).
- Installed Scheme policy takes precedence over development checkouts, and
  release binaries never load policy from their former build directory (#171).
- Fullscreen state is now local to each group. Switching away leaves other
  groups free to tile and receive keyboard focus; returning restores the
  original fullscreen window without shrinking it in between. Closing hidden
  fullscreen windows and moving them to another group clear stale ownership.
- Fullscreen follows output size, scale and hotplug changes, including windows
  restored from a hidden group after their monitor moved or disappeared.
- Absolute-input relative pointer events now use consecutive samples from the
  same device, even while the cursor is locked or confined. Warps, device/input
  switches and coordinate-space changes reset the baseline without a jump.

- Pointer confinement now applies to the surface input region when the client
  supplies no explicit region. Explicit regions are intersected with surface
  bounds/input regions, respect subtracted holes, and keep motion inside
  exclusive edges without jumping between disconnected regions.
- Xwayland fullscreen requests, including startup fullscreen hints, now use
  the same layout policy as native clients and cover the window's own output.
  Fullscreen hides the tiled focus border and Top-layer desktop panels in both
  rendering and pointer hit testing; Overlay-layer surfaces remain available.
- Managed tiled X11 clients can no longer shrink themselves independently of
  the placement cache, leaving content smaller than the selected frame.
- Added a nested regression gate for actual pointer lock/confinement, X11
  grabs, native/X11 fullscreen, panel visibility/input and tiled self-resizes.

### Added

- A typed agent control interface over the existing Scheme socket: desktop-wide
  snapshots, discoverable schemas, explicit targets, session/revision checks,
  bounded action receipts, asynchronous completion, and sequenced event replay.
  `mindectl` adds machine/JSON output, discovery, calls, waits, watch recovery,
  and a multiline remote REPL with registry completion and private history.
  The optional `minde-agent` selector defaults to dry-run; deterministic,
  Jev, and loopback transports share the validated executor. Live model
  evaluation remains deferred. See [agent control](doc/agent-control.md).
- IPC reply serialization now checks value types, preserves literal `#<`
  strings, represents unspecified values, and distinguishes execution from
  result-encoding failures without suggesting that prior effects rolled back.
  Targeted screenshots accept window ID zero and select the target's output.

- StumpWM-style window gaps with configurable inner, outer and head padding,
  live enable/disable/toggle commands, transient and fullscreen exceptions,
  and safe sizing on small frames. Existing `set-gaps!` calls retain their
  shared-spacing semantics; `configure-gaps!` selects per-edge padding.

- Automation primitives now provide paced, focus-pinned `wm-send-string` /
  `wm-type`, paced `wm-send-key` (including the `Enter` alias), symbolic and
  evdev pointer buttons with click counts, `wm-paste`, `wm-scroll`, pointer and
  window geometry inspection, and separate CLIPBOARD/PRIMARY setters. Native
  Wayland targets additionally support compositor-owned `wm-drop-files` and
  `wm-drop-text` copy operations with bounded token status history and
  `automation-result` events; file URI lists are validated, percent-encoded,
  multi-file capable, and CRLF terminated. XWayland drop targets explicitly
  report `unsupported-target` pending a separate XDND implementation.
- A native `wm-screenshot` primitive (`path [window-id]`) with bounded
  token status history via `wm-automation-status`, capturing an output or
  a single window to PNG without an external screenshot tool (see
  `doc/notes/issue-wm-screenshot-primitive.md`).
- Keyboard layout groups: `XKB_DEFAULT_LAYOUT=de,us` (with matching
  `XKB_DEFAULT_VARIANT=bone,`) builds one keymap with several XKB groups;
  `(keyboard-layouts)` lists them and `(set-keyboard-layout! spec)` switches
  by index, `next`/`prev` or a name prefix, so the toggle can live under the
  prefix key instead of a `grp:*` chord (which keeps working too).
  `(handle-keyboard-layout-changed! name)` runs on every change, defaulting
  to the message overlay. Exported by `(minde groups)` over the primitives
  `wm-keyboard-layouts` and `wm-set-keyboard-layout!`; nested e2e gate
  `tests/keyboard-layout-e2e.sh`.
- Output layout: heads whose position was never set explicitly (by an
  output-management client or `configure-output!`) are re-packed edge to
  edge whenever the layout changes, so `wlr-randr --output eDP-1 --scale 2`
  alone no longer opens a hole between heads that the pointer cannot cross;
  a clamped pointer now lands strictly inside an output so the cursor stays
  visible at the far edges.  Re-packing only runs when the layout is
  actually unsound (an overlap, or an auto head detached from both the
  origin and every neighbouring right edge): a sound layout -- including a
  vertical stack under an explicit head, or a profile that omits
  `position` -- is left alone instead of being shuffled to y = 0 on every
  layer-surface commit.
- `wlr-output-power-management-unstable-v1` (hand-written, both backends) so
  `wlopm` and `swayidle` (`timeout N 'wlopm --off *' resume 'wlopm --on *'`)
  can blank outputs and light them again. Power is orthogonal to the layout:
  a powered-off head stays enabled and mapped with its windows and
  `wl_output` global, wlr-randr keeps reporting `Enabled: yes`, and only the
  backend stops driving it (udev releases the CRTC, winit paints black). Any
  key press, pointer button or pointer motion powers every head back on
  (`wake_on_input`, default on; a Scheme toggle is planned). New nested e2e
  gate `tests/output-power-e2e.sh`.
- udev backend: mode changes requested through wlr-output-management
  (`wlr-randr --output DP-1 --mode 1920x1080@60`, kanshi/shikane `mode`)
  are real modesets on the connector's CRTC. Custom modes are matched to a
  connector mode of the same size whose refresh is within 1 Hz; custom
  modelines are refused, and a failed modeset (or any other head in the same
  configuration failing) restores the previous scanout mode. Known
  limitation: when the new mode exceeds the available bandwidth, Smithay's
  fallback may show a single black frame on the other heads.
- udev backend: adaptive sync (VRR) per head through wlr-output-management
  (`wlr-randr --output DP-1 --adaptive-sync enabled`). Heads report the real
  `Adaptive Sync` state, enabling it on a connector without VRR fails the
  configuration, and the setting survives disabling and re-enabling the head.
- udev backend: heads can be disabled and re-enabled through
  wlr-output-management (`wlr-randr --output DP-1 --off` / `--on`, kanshi and
  shikane profiles). Every connector is tracked as a head whether or not it
  is enabled: disabling switches the CRTC off, withdraws the `wl_output`
  global (as wlroots does) and moves windows to the remaining heads while the
  head stays advertised with `Enabled: no`; enabling brings it back at the
  requested or preferred mode, right of the other heads unless a position was
  given, and the head keeps its stable Scheme id across the cycle. Hotplugged
  connectors still come up enabled by default.
- Scheme output API: `(output-heads)` returns every known head (disabled
  ones included) as alists with make/model/serial/description, mode list,
  current mode, position, scale, transform, enabled and adaptive-sync state,
  and `(configure-output! name #:mode #:position #:scale #:transform
  #:enabled #:adaptive-sync)` applies a change through the same validated,
  revert-on-failure routine as wlr-output-management clients (bypassing
  `output-configuration-allowed?`, which gates external clients only). Both
  are exported by `(minde groups)` over the new primitives `wm-output-heads`
  and `wm-configure-output!` (name, alist). `(minde groups)` now also ships
  documented defaults for `output-configuration-allowed?` (`#t`),
  `handle-output-configured!` (no-op) and the new
  `handle-output-configure-failed!` (logs), which a user init file may
  shadow. Malformed settings raise a Scheme error. A nested e2e gate
  (`tests/output-scheme-e2e.sh`) drives both through `mindectl eval` and
  cross-checks with wlr-randr.
- udev backend: every non-interlaced connector mode is advertised (with the
  DRM `PREFERRED` mode flagged as preferred), and monitor identity is read
  from the connector's EDID blob by a small in-tree parser (`src/edid.rs`,
  base block only: PNP manufacturer ID, monitor-name and serial descriptors,
  product/serial fallbacks). Outputs carry real make/model/serial and subpixel
  layout, and `crate::edid::output_description` yields the wlroots-style
  `"Make Model Serial"` string that kanshi/shikane match against (falling
  back to the connector name, never `Unknown Unknown Unknown`).
- Multi-monitor tooling and documentation (no compositor change): `shikane`,
  the wlr-output-management profile daemon, is packaged in the in-repo Guix
  channel (`guix build -L guix-channel shikane`; crate graph pinned in
  `guix-channel/minde/rust-crates.scm`), `wdisplays`, `wlopm` and `kanshi`
  join the development shell (`manifest.scm`), and `doc/configuration.md`
  gains "Outputs and multiple monitors": layout is owned by an external
  daemon (shikane recommended, kanshi as the no-channel fallback), with
  `set-head-mode!`, `output-configuration-allowed?` and
  `handle-output-configured!` as the Scheme-side knobs.
- Clipboard ecosystem: primary selection
  (`zwp_primary_selection_device_manager_v1`) for middle-click paste, wired
  across Wayland clients and Xwayland (the X11 selection loop now mirrors the
  primary selection in both directions), plus clipboard-manager support via
  `zwlr_data_control_manager_v1` and `ext_data_control_manager_v1`. All three
  ride Smithay's selection delegates and share the compositor selection. A
  bounded nested e2e gate (`tests/clipboard-e2e.sh`, wired into
  `make check-e2e`) asserts the manager globals are advertised and that both
  the clipboard and the primary selection round-trip through wl-clipboard.
- `wlr-foreign-toplevel-management-unstable-v1` (v3, hand-written on the wlr
  bindings) so external bars, docks and switchers can enumerate windows and
  request activate/close/fullscreen/minimize. Per-window title, app-id,
  activated state and output association are kept in sync from the existing
  window-lifecycle and focus paths; activate/close/fullscreen route through
  the group/frame model (new Scheme hooks `handle-foreign-activate!`,
  `handle-foreign-fullscreen!`, `handle-foreign-minimize!` in
  `(minde groups)`), and minimize is a documented no-op (the tiling model
  has no minimized state). A bounded nested e2e gate
  (`tests/foreign-toplevel-e2e.sh`) binds the manager against a live toplevel.
- `wlr-output-management-unstable-v1` (v4, hand-written) so wlr-randr, kanshi
  and wdisplays can query and set the output layout (mode, position, scale,
  transform). Accepted configurations apply to the compositor outputs and
  reconcile back into the Scheme head model (`(wm-outputs)` /
  `handle-heads-change!`) exactly as a hotplug/resize does; external changes
  re-advertise to bound managers. Acceptance is gated by an optional Scheme
  policy predicate `output-configuration-allowed?` (default accept), and
  `handle-output-configured!` fires after an external change. Mode changes are
  refused under the winit backend (the host window fixes the size). A bounded
  nested e2e gate (`tests/output-management-e2e.sh`) queries the head and
  applies a scale via wlr-randr.
- Pointer constraints and relative pointer: `zwp_pointer_constraints_v1`
  (pointer lock and confinement) and `zwp_relative_pointer_manager_v1` (raw
  relative motion) via Smithay's server modules, for games and pointer-lock
  clients. A locked pointer stays parked while relative motion still flows, and
  the client's cursor-position hint is honored by warping the cursor there on
  unlock; a confined pointer is clamped to its region; constraints activate when
  the pointer enters their surface (focus-driven), per the protocol. Relative
  motion is forwarded from real libinput deltas on the udev backend and
  synthesized from consecutive absolute positions on the winit (nested) backend,
  so the relative-pointer protocol is usable in both. No Scheme surface: these
  are per-surface client requests, not compositor policy. A bounded nested e2e
  gate (`tests/pointer-constraints-e2e.sh`, wired into `make check-e2e`) asserts
  both manager globals are advertised; Rust unit tests cover the region-clamp
  and membership logic.
- Fractional scaling and viewport: `wp-fractional-scale-v1` and
  `wp-viewporter` served on both backends via Smithay's server modules.
  Clients learn each surface's preferred fractional scale, which follows the
  output the surface sits on and is re-sent to every mapped surface when an
  output's scale changes (e.g. wlr-randr `--scale 1.5`, through the
  output-management path). The render and screencopy paths now use the
  output's `f64` fractional scale throughout -- window content, the
  compositor chrome (focus border, message and frame-number overlays) and
  capture buffers -- replacing the previous integer-only assumptions; the
  winit backend renders at the output's fractional scale via its damage
  tracker, and screencopy buffer sizes stay physical (mode size), so a
  fractionally-scaled output is captured at full resolution. Viewporter is
  validated by Smithay's commit buffer handler and its surface render
  elements honor the destination size automatically. No new public Scheme
  exports: output scale (already settable through wlr-output-management)
  accepts fractional values. A bounded nested e2e gate
  (`tests/fractional-scale-e2e.sh`, wired into `make check-e2e`) asserts both
  globals via wayland-info, applies a 1.5 scale with wlr-randr and confirms
  the compositor reflects it and stays alive, then runs foot at that scale
  without a protocol error.
- Idle notify and idle inhibit: `ext-idle-notify-v1` and
  `zwp_idle_inhibit_manager_v1` (idle-inhibit) via Smithay's server modules,
  advertised on both backends. Per-seat idle timers reset on every input
  event at the central `process_input_event` dispatch, so a swayidle-style
  daemon can drive auto-lock, DPMS and dimming (idle policy stays external,
  the same stance `(minde session)` takes for the locker -- no Scheme
  surface is added). An idle inhibitor on any surface suppresses idle
  notifications so fullscreen video and calls keep the screen awake; idle is
  inhibited while any live inhibitor exists (per-surface visibility is not
  tracked, documented in `src/handlers/idle.rs`), and inhibitors whose client
  died are pruned so a crash cannot pin the screen awake. A bounded nested
  e2e gate (`tests/idle-e2e.sh`, wired into `make check-e2e`) asserts both
  globals via wayland-info and runs swayidle with a one-second timeout,
  confirming the compositor actually fires an idle notification.
- Cursor shape and Xcursor theming: `wp_cursor_shape_manager_v1`
  (`cursor-shape-v1`) via Smithay's `cursor_shape` module, advertised on both
  backends, so clients request a named cursor shape instead of attaching their
  own surface. Named cursors (the default pointer and every cursor-shape
  request) now resolve through an Xcursor theme loaded from `XCURSOR_THEME`
  and `XCURSOR_SIZE` (defaults: theme `default`, size 24), replacing the
  previously always-on hardcoded fallback bitmap; the requested cursor size
  scales with the output's integer-ceil scale so cursors stay crisp on HiDPI
  outputs. Themed images are cached per shape and scale, honour the image
  hotspot, and render through the same `CursorState` path as client-surface
  cursors -- so themed, shape-requested and client cursors all appear on the
  udev on-screen pointer and in screen captures. When no theme is installed
  (a bare environment), rendering falls back to the built-in bitmap. Animated
  cursors use their first frame only (documented in `src/render.rs`). Theme
  and size come from the environment per ecosystem convention, so no Scheme
  surface is added. A bounded nested e2e gate (`tests/cursor-shape-e2e.sh`,
  wired into `make check-e2e`) asserts the manager global via wayland-info and
  that the compositor stays stable under an explicit theme/size environment;
  Rust unit tests in `src/render.rs` cover shape-name resolution and integer
  size selection.
- Input methods via `text-input-v3` and `input-method-v2`:
  `zwp_text_input_manager_v3` and `zwp_input_method_manager_v2` advertised on
  both backends through Smithay's `text_input`/`input_method` modules, so an
  IME (fcitx5/ibus) or on-screen keyboard can drive preedit and commit into
  Wayland text fields. Text-input focus follows keyboard focus
  (`MindeState::set_text_input_focus`, driven from `SeatHandler::
  focus_changed` and the focus-clear paths), the input-method popup (candidate
  window) is tracked in the shared `PopupManager` and rendered at the
  text-input cursor rectangle by the existing per-window popup path, and the
  input-method manager admits every client first-come-first-served (the
  protocol enforces one active IME per seat; no Scheme surface, as the IME is
  the user's session daemon). A bounded nested e2e gate
  (`tests/text-input-e2e.sh`, wired into `make check-e2e`) asserts both manager
  globals via wayland-info and that a real text-input-v3 client (foot) maps,
  gains focus, and stays connected with no protocol error.
- Virtual keyboard and pointer: `zwp_virtual_keyboard_manager_v1`
  (Smithay's `virtual_keyboard` module, client filter admits every client --
  these are user-session automation and accessibility tools) and a hand-written
  `zwlr_virtual_pointer_manager_v1` v2 on the wlr bindings (Smithay ships no
  server module for it), for wtype/ydotool/wlrctl-style automation. Injected
  keys reach the focused surface's keyboard directly (gated by focus, so they
  never bypass the session lock); virtual pointer motion/button/axis are
  translated into the *same* pointer processing the real input path uses
  (factored into `pointer_relative_motion`/`pointer_absolute_motion`/
  `pointer_button_event`/`pointer_axis_frame` in `src/input.rs`), so pointer
  constraints, idle-activity reset and the locked-session gate all apply to
  virtual input. No Scheme surface.
- Keyboard shortcuts inhibit: `zwp_keyboard_shortcuts_inhibit_manager_v1`
  (Smithay's `keyboard_shortcuts_inhibit` module) for remote-desktop and VM
  clients. Inhibitors are auto-granted (documented; no user prompt) but active
  only while their surface has keyboard focus. An active inhibitor makes
  `process_input_event` bypass the prefix-key grab and every Scheme shortcut --
  checked right after the session-lock gate, so it can never take effect on the
  lock surface -- and focus loss deactivates it immediately
  (`update_keyboard_shortcuts_inhibitors`, called from `focus_changed` and
  every focus-clear path) so it can never survive focus loss. No Scheme surface.
- A bounded nested e2e gate (`tests/virtual-input-e2e.sh`, wired into
  `make check-e2e`) asserts all three manager globals, uses wtype to type a
  marker line into a focused foot client and asserts the text lands, and drives
  the hand-rolled virtual pointer with wlrctl while asserting the compositor
  stays alive and logs no protocol error.
- Presentation timing, tearing control and explicit sync:
  - `wp-presentation-time` (Smithay's `PresentationState`) is served
    **udev-only**. Each output's DRM frame now carries an
    `OutputPresentationFeedback` as its queue user-data; on the matching vblank
    it is delivered with the kernel's real monotonic timestamp, sequence and
    `Vsync|HwClock|HwCompletion` flags (falling back to the compositor clock and
    `Vsync` only when the driver reports no monotonic timestamps), the refresh
    interval from the output mode, and per-surface zero-copy flags taken from
    the render report. The nested winit backend has no true presentation clock,
    so it deliberately does not advertise the global.
  - `wp-tearing-control-v1` (`wp_tearing_control_manager_v1`, hand-written on
    the staging bindings since Smithay ships no server module) is advertised on
    **both** backends so tearing-aware clients find it. It is **advisory only**:
    the vendored Smithay `DrmCompositor` exposes no async/immediate page-flip
    path, so the `async` hint is recorded but never turned into a real tearing
    flip -- clients render identically either way. This is documented honestly
    in the capability matrix rather than claimed as working tearing.
  - `linux-drm-syncobj-v1` (`wp_linux_drm_syncobj_manager_v1`, Smithay's
    `DrmSyncobjState`) explicit sync is served **udev-only** and only when the
    primary GPU exposes syncobj timeline eventfds (`DRM_CAP_SYNCOBJ_TIMELINE`,
    probed by `supports_syncobj_eventfd`); hardware without support never gets
    the global. Smithay's renderer imports the client acquire fence as a KMS
    in-fence and signals the release point when the buffer is dropped, so no
    compositor-side commit changes were needed. No new Scheme surface.
  - A bounded nested e2e gate (`tests/presentation-tearing-e2e.sh`, wired into
    `make check-e2e`) asserts `wp_tearing_control_manager_v1` is advertised and
    that neither `wp_presentation` nor `wp_linux_drm_syncobj_manager_v1` appear
    on the winit backend (locking in the udev-only split); the udev presentation
    and syncobj paths are flagged for hardware-owner verification.
- libinput device configuration surface in Scheme: two compositor
  primitives, `(wm-input-devices)` (enumerate seat devices with their
  capability names) and
  `(wm-configure-input! match #:tap-to-click #:natural-scroll #:accel-profile
  #:click-method)`, plus an optional `handle-input-device-added!` hook. Rules
  are stored (deduplicated by match) and the udev/DRM backend applies them to
  every matching device on arrival (hotplug) and re-applies to devices already
  present when a rule changes at runtime; settings a device does not support
  are logged and skipped, never fatal. These are Rust-registered primitives,
  not part of the frozen `(minde …)` module API. The nested winit backend
  has no libinput, so `wm-input-devices` returns `()` and rules configure
  nothing. Covered by a Scheme unit test (`tests/input-config-test.scm`,
  argument validation and rule normalization on the no-libinput path) and a
  bounded nested e2e gate (`tests/input-config-e2e.sh`, wired into
  `make check-e2e`) that drives both primitives through `mindectl eval`;
  real per-device tap/scroll/accel/click on a touchpad is hardware
  owner-verification.
- Screen capture via `ext-image-copy-capture-v1` and
  `ext-image-capture-source-v1` (output capture sources). Screenshot
  tools such as grim can now grab a frame of an output on both the winit
  (nested) and udev/DRM backends. Capture buffers are filled after the
  output's on-screen frame is composited; shm is supported everywhere
  (grim's path, verified nested), and the udev backend additionally
  advertises dmabuf constraints from the primary render node for a future
  zero-copy screen-cast path. This first version reports full-frame
  damage and advertises only output sources -- no foreign-toplevel
  capture and no separate cursor session (cursors are drawn inline only
  when a client requests paint-cursors). A bounded nested e2e gate
  (`tests/screencapture-e2e.sh`, wired into `make check-e2e`) launches the
  nested compositor and asserts that `grim` captures a correctly-sized,
  non-blank PNG over `ext-image-copy-capture-v1`.
- Screen capture via legacy `wlr-screencopy-unstable-v1` (v3), the
  protocol wf-recorder and `xdg-desktop-portal-wlr` (browser screen
  sharing) speak. Served on both the winit and udev/DRM backends and
  advertised alongside the ext protocol; `capture_output`,
  `capture_output_region` (honest sub-rectangle capture), `copy` and
  `copy_with_damage` are all implemented, with shm everywhere and dmabuf
  advertised on udev. Both protocols share the single capture queue drained
  after each composite, so wlr frames reuse the ext scene-assembly and
  buffer-fill machinery rather than a parallel render loop. The nested e2e
  gate (`tests/screencapture-e2e.sh`) now additionally records a bounded
  `wf-recorder` clip and validates it (via `ffprobe` when available).
- `scripts/ci`, the single noninteractive CI entry point: the fast gate
  plus release-metadata by default, and `--release-artifacts` builds the
  deterministic archives twice, proves byte-reproducibility, recreates
  the crate mirror on a fresh checkout, and writes both archives plus
  `SHA256SUMS`. The hosted GitHub Actions workflow
  (`.github/workflows/ci.yml`) contains no gate logic: it installs Guix,
  restores the store cache, calls `scripts/ci`, and on version tags
  publishes the artifacts to a GitHub Release.
- A Guix channel (`.guix-channel`, `guix-channel/`) exposing
  `guile-minde-foundation` and `guile-minde-ui` from the committed
  tree. The compositor package joins the channel once the first tagged
  release publishes the vendored archive at a stable URL; the channel
  deliberately does not ship a package that cannot build from a
  `guix pull` checkout.
- `doc/generated/packaging.md`, a script-generated packaging reference
  (`scripts/generate-packaging-reference`): build/runtime dependencies,
  installed layout, session entry, archive verification, channel usage,
  and the first-party-Guix support boundary, drift-checked by
  `make check-docs`.
- `scripts/run-nested`, the documented nested-session entry point, now
  exists: it guards against a missing Guix shell, cargo, or graphical
  session, and runs the nested instance under an isolated
  `XDG_RUNTIME_DIR` so it cannot displace the IPC socket of a live outer
  Minde session.
- `doc/testing.md` is now a generated verification-command reference
  (`scripts/generate-testing-reference`), wired into `make docs` and
  drift-checked by `make check-docs`.
- `doc/debugging.md` documents interactive Scheme debugging
  (`MINDE_UNSAFE_REPL=1` REPL versus one-shot `mindectl eval`),
  Rust-side debugging (backtraces, gdb on the nested backend, the DRM
  seat caveat, crash.log), cold-build expectations, and editor setup;
  `CONTRIBUTING.md` carries condensed pointers.

### Fixed

- Session locks now retain their protocol owner, reject unauthorized unlocks,
  and keep desktop windows and input-method/popup grabs from taking lock-screen
  keyboard input. Lock acknowledgement and suspend wait for locked-frame DRM
  presentation (or a successful nested-window buffer swap).
- Event subscribers drain their final buffered events when the socket becomes
  writable, without requiring another publication. Idle disconnects free their
  slots; subscriber write-half-close remains supported.
- Completed suspend attempts cannot cancel a later attempt through an old lock
  timeout. Event privacy begins immediately; `wm-session-lock-confirmed?` is the
  separate readiness check used for suspend.

- `mindectl subscribe --json` no longer trails the compositor by one
  change. It woke on the event line and re-read status.json before the
  deferred write had landed, so bars showed the previous state until the 2 s
  safety poll; it now waits briefly for the new document after each event.

- Disabling a head (hotplug, shikane, `configure-output!`) no longer kills
  clients that bind its `wl_output` late: the global is announced as gone
  first and only freed after a grace period, so eww bars and gammastep
  survive a monitor reconnect instead of dying with `invalid global
  wl_output`.

- Flush Wayland clients after every event-loop iteration. Since repaints
  became damage-driven, events that dirty nothing (key releases, pointer
  motion over a static scene) stayed buffered until the next render, so
  clients received key releases seconds late and auto-repeated the key
  meanwhile (`ovvvvvvvver`, runs of spaces) and pointer input felt sluggish.

- `focus-next-window!` / `focus-previous-window!` no longer do nothing when
  the target id cannot be resolved in the active group (a stale mirror after
  head churn or a drifted float list): `focus-window-by-id!` now reports
  whether the jump happened, and the cycling commands fall back to a
  `sync-frames!` so placements, hooks and the status line still self-heal.
- A compositor-side key repeat armed via `wm-set-key-repeat` no longer keeps
  firing its Scheme handler across a VT switch or suspend: pausing the
  session cancels the repeat timer, since the held key's release can never
  be delivered to a suspended libinput context.
- `handle-keyboard-layout-changed!` can no longer recurse without bound: a
  hook that itself calls `set-keyboard-layout!` still updates the
  `(wm-keyboard-layouts)` snapshot, but the nested hook fire is suppressed
  (and logged) instead of re-entering the hook on the same stack.
- The compositor callback registry gained the missing
  `handle-output-configure-failed!` and `handle-keyboard-layout-changed!`
  entries, so `define-compositor-callback!` accepts them and the
  registry/Rust-hook-table consistency test passes again.
- Move and resize grabs deferred their `on_window_moved` Scheme hook call to
  an idle callback instead of invoking it from inside `PointerHandle::button`;
  the hook running there while the pointer's non-reentrant lock was held
  meant any pointer-touching command it issued (`wm-warp-pointer!`,
  `wm-click`, `wm-scroll`) re-locked the same mutex on the same thread and
  froze the compositor.
- Output configuration no longer fires duplicate events: applying a
  configuration reports the final layout exactly once instead of also
  advertising the half-applied state from inside the per-head loop, a
  re-applied identical profile stays silent, and layer-shell changes (a bar
  mapping or unmapping) no longer bump the wlr-output-management serial —
  so daemons like shikane stop re-evaluating (and re-applying) profiles on
  every bar respawn, which duplicated `handle-output-configured!` /
  `handle-heads-change!` and with them eww bars. Also hardened: a reverted
  configuration hands heads it positioned back to the auto layout, a failed
  head enable restores the previous mode/position instead of keeping the
  one that failed, a DPMS power-on whose recorded mode fails retries at the
  preferred mode instead of staying dark, and a late vblank from a flip the
  watchdog already abandoned is no longer attributed to the frame in flight.
- Overrides of `(minde session)` configuration variables
  (`%lock-command`, `%suspend-command`, `%lock-on-suspend?`,
  `%lock-timeout-ms`) were silently ignored in compiled builds because
  the module was declarative; the module is now non-declarative and
  `set!` from a personal configuration works as documented.
- `doc/support.md` no longer claims the session-lock protocol is
  missing.
- `tests/check-portable-defaults.sh` now fails loudly when ripgrep is
  absent instead of printing a false "ok"; `manifest.scm` gained
  `ripgrep` and `diffutils` so `./check` is self-contained in the
  project shell.
- `wm-click` now hovers and settles before a synthetic click, fixing
  flaky clicks on custom controls and unrecognized double-clicks (see
  `doc/notes/issue-wm-click-paste-settle-timing.md`).
- `wm-scroll` sends discrete wheel steps with scroll frames instead of a
  continuous delta, fixing scrolling that had no effect on Firefox/Zen
  web content (see `doc/notes/issue-wm-scroll-no-effect.md`).
- `wm-type`/`wm-send-string` no longer drop characters the active keymap
  cannot type; unresolvable characters are now pasted instead (see
  `doc/notes/issue-wm-type-drops-modifier-chars.md`).
- `wm-drop-files`/`wm-drop-text` now dwell longer with per-step jitter
  during a negotiated drag-and-drop, fixing browser dropzones that
  rejected synthetic drops (see
  `doc/notes/issue-wm-drop-files-rejected-by-dropzones.md`).
- Fixed a GC-rooting hazard in `src/guile/mod.rs`: several functions
  (`wm-outputs`, `wm-output-heads`, `wm-timing-stats`, `wm-keyboard-layouts`,
  `wm-input-devices`, `wm-place-windows`, the `heads-change` and
  `input-device-added` hooks) collected heap-allocated `Scm` values (pairs,
  strings, symbols) into a plain `Vec<Scm>` while still making further
  allocating Guile calls; bdw-gc never scans the Rust heap, so a GC
  triggered mid-loop could free those values out from under the pending
  list build. Lists are now built incrementally into a stack-rooted `Scm`
  local (new `scm_list_map` helper) so every allocated value stays
  GC-reachable; `tests/lint-hook-borrows.sh` now also flags a bare
  `Vec<Scm>` in `src/guile/mod.rs`.

- Session management: `ext-session-lock-v1` compositor support (swaylock
  and other lockers can lock the session; while locked no desktop pixel
  is shown and no input reaches clients or keybindings), plus the
  `(minde session)` commands `lock-screen!`, `suspend!` (locks first
  and waits for lock confirmation, failing closed on timeout), and
  `logout!` (confirmation-gated), with `session-lock`/`session-unlock`
  hooks and the `wm-session-locked?` query.
- A Guix-first development environment plus a predictable fast `./check`,
  persistent nested runner, and explicit integration/release gates.
- Strict `--help` and `--version` output with an embedded build revision.
- Release, support, security, licensing, and provenance policies.
- Independently packaged `guile-minde-foundation` modules for geometry,
  trees, serialization, hooks, and key notation.
- Independently packaged, callback-driven `guile-minde-ui` prompt and menu
  engines that can be tested without a compositor or display.
- A canonical Scheme command registry, stable public module facades, and
  display-free configuration validation.
- A declarative default configuration and the `mindectl check-config`
  local validation command.
- A dependency-light portable C-t keymap, configurable terminal selection,
  and collision/personal-policy checks.
- A main-thread `mindectl eval` IPC endpoint and concurrent reload stress
  target; the separate-thread Guile REPL is now explicitly unsafe and opt-in.
- Schema-v1 `query state --json`, change-only `subscribe --json`, and atomic
  `status.json` interfaces for bars and monitoring.
- Human or newline-delimited JSON tracing, Scheme error backtraces, two-session
  log retention, and a conservatively redacted `mindectl report` bundle.
- Deterministic tree/geometry/serialization properties, Rust input/output and
  rendering tests, a structured application matrix, a reduced-keymap nested
  scenario, and a bounded RSS-reporting soak runner.
- Source-generated API and keymap inventories, a command/demo manifest, an
  offline HTML field manual, and deterministic WebM/poster/transcript capture
  for every registered command.
- Source-owned descriptions for all 274 bindings exported by the eight public
  Scheme modules, including explicit metadata for record accessors and values.
- Reproducible source and vendored-source archives, an archive-only public
  Guix package entry point, installed documentation/Guile modules, and a
  clean-tree local release pipeline with a self-contained bounded Guix
  environment, checksums, and release notes.
- A machine-readable 1.0 RC contract freezes public API/keymap inventories and
  configuration, status, persistence, and diagnostic schema versions.
- `make check-hardware` now writes a retained per-machine hardware snapshot and
  owner checklist; System/Home can opt into a vendored RC archive while keeping
  the checkout package as the default rollback path.

### Changed

- Output management (`wlr-output-management-unstable-v1`) reports state
  honestly and applies configurations in two phases. Heads carry their real
  `enabled` state (position/mode/transform/scale only while enabled) and the
  v4 `adaptive_sync` event; a disabled head stays advertised with its mode
  list instead of disappearing, and a refresh re-sends changed
  name/description/make/model/serial/physical-size/mode-list properties.
  A configuration is validated with no side effects (unknown head or mode,
  adaptive sync where the backend lacks it, and leaving zero enabled heads
  are all refused; enabling a head without a mode picks its preferred one;
  a no-op apply succeeds), then applied per head with snapshots that are
  reverted in reverse order if the backend refuses. `test` is now gated by
  `output-configuration-allowed?` like `apply`, `set_adaptive_sync` is
  honoured (invalid values are a protocol error), and under winit
  `--off`/`--on` unmap and remap the single output (black frame while
  disabled). Enable/disable and mode changes on the udev backend still
  fail until the DRM stages land. The test-only environment variable
  `MINDE_OUTPUT_MGMT_ALLOW_NO_HEADS=1` lets the single-output nested e2e
  exercise disable/enable; the gate now also asserts Enabled/Adaptive Sync
  reporting, refusals and the round trip.
- The project license metadata is consistently GPL-3.0-or-later while
  retaining the MIT terms for Smithay example-derived files.
- Mutable frame/group records now live in an explicitly internal compositor
  model module instead of being defined merely to connect public modules.
- Abbreviated StumpWM command and event names were replaced with descriptive
  kebab-case names; event handlers use the `handle-` prefix.
- Configuration reloads validate and build a candidate binding table before
  atomically publishing it.
- Keyboard layout, application launchers, locks, wallpaper, eww, and autostart
  moved out of repository defaults and into the personal Guix Home layer.
- Portable Alt and uppercase-letter bindings were replaced with lowercase
  nested sequences: `w h/j/k/l` move windows, `x h/j/k/l` exchange them,
  `w 0`–`w 9` pull numbered windows, and `f 0`–`f 9` select frames. Nested
  maps provide contextual `?` help.
- Directional exchange was flattened from `f x h/j/k/l` to `x h/j/k/l`; no
  portable binding now needs a second submap level for directional movement.
- Layouts, placement rules, and desktop dumps now use versioned, atomic
  persistence formats.

### Fixed

- The CLI contract check accepts semver pre-release versions
  (`1.0.0-rc1`), so `make check` passes on the release-candidate series.
- End-to-end tests no longer inherit host keymap or compositor REPL state.
- Client disconnect, popup/grab, DnD, missing-output, and hotplug paths recover
  from stale protocol state instead of panicking.
- Local Guix builds no longer include workspace-only agent, cache, or runtime
  artifact directories in the source derivation.
- Pointer clamping now handles negative, vertical, gapped, and non-uniform
  output arrangements instead of assuming one horizontal strip.
- The monolithic README is now an overview backed by focused tutorial,
  configuration, concepts, IPC/Eww, debugging, architecture, security,
  hardware, support, keymap, and demonstration guides.
- The end-to-end target now reports a missing `foot` dependency before entering
  the run-prompt scenario.
