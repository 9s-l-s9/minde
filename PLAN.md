# PLAN: Observability and agent-driven manipulation

## Goal

Make minde the desktop that an agent (or a curious user at the REPL) can
*understand* well enough to manipulate confidently: every fact the
compositor knows about a window, output, or input device is queryable
as data; every change the policy layer made can be explained; the screen
itself can be inspected; and temporary programs installed through the
REPL can be found, listed, and taken back without losing anything the
user created.

This plan is organised as epics (A–H), each broken into issues small
enough for one commit series.  Issues carry: motivation, design, tasks,
acceptance, dependencies.  Effort is S (< 1 day), M (1–3 days),
L (a week or more of real sessions).

Status: drafted 2026-09-05.  Nothing here is started.

## What already exists (do not rebuild)

| Surface | Where | Notes |
|---|---|---|
| Deferred screenshot with token | `wm-screenshot`, `wm-automation-status`, `automation-result` event | PNG of output under pointer or a window's region; async |
| Event push socket | `$XDG_RUNTIME_DIR/minde-events.sock`, `mindectl subscribe --events` | one s-expression per fired hook, filtered like status.json |
| Live API catalog | IPC `api-catalog`, `doc/generated/api-catalog.scm` | commands, procedures, gsubrs, hooks; apropos filter |
| Status snapshot | `current-state`, `status.json`, schema v1 | outputs, groups, layout; redactable |
| Consistency checks | `mirror-drift`, `check-compositor-callbacks!` | owner/mirror table in doc/architecture.md |
| Timing | `wm-timing-stats`, `src/timing.rs` | histograms per command / key / frame |
| Input injection | `wm-send-key`, `wm-send-string`, `wm-click`, `wm-scroll`, `wm-warp-pointer`, `wm-drop-text` | write-only; no confirmation the target received it |
| Window facts | `wm-window-title` (title . app-id), `wm-window-geometry`, `wm-floating-ids` | thin; see A1 |

## Design principles

1. **Facts as data, not prose.**  Every query returns an s-expression
   with a documented shape and a JSON projection.  No free text an
   agent has to parse.
2. **Ask the owner.**  Per the ownership table in doc/architecture.md,
   a query for a Rust-owned fact goes to Rust; a Scheme-owned fact to
   Scheme.  New queries must not create new mirrors.
3. **Nothing the user made is ever lost by an agent action.**
   Ownership tags and snapshots exist to remove *only* what a tag
   owns.  Untagged definitions are the user's and are never retracted
   implicitly.  Snapshots are kept in a bounded ring, never
   overwritten by a later snapshot.
4. **Async has a token, sync has a value.**  Anything that needs a
   render or a client round trip (screenshot, key injection with
   confirmation) returns a token and fires `automation-result`.  A
   blocking convenience wrapper may exist in Scheme, with a timeout.
5. **Every observable has a test on the nested backend.**  Extend
   `tests/e2e.sh` scenarios rather than adding hardware-only checks.
6. **Redaction rules follow status.json.**  Titles and content-bearing
   fields obey the same redaction switch in every new surface.

---

## Epic A — Complete window facts

The agent's most common question is "what is this window?".  Today the
answer is a title, an app-id, a rectangle and a floating flag.

### A1. `wm-window-info`: one primitive returning every Rust-owned fact  (M)

**Motivation.**  Distinguishing a Firefox Picture-in-Picture player from
the browser, a dialog from its parent, an Xwayland client from a native
one, needs facts Rust has and Scheme cannot see.

**Design.**  `(wm-window-info id)` → association list or `#f`:
`id`, `title`, `app-id`, `geometry` (placed rect), `requested-size`,
`min-size`, `max-size`, `parent` (id or `#f`), `kind`
(`toplevel|dialog|xwayland|popup-owner`), `pid`, `client-id`
(stable per Wayland connection), `states` (subset of `maximized
fullscreen resizing activated tiled-*`), `decoration`
(`server|client|none`), `mapped?`, `visible?`, `outputs` (ids
intersecting), `created-ms`, `last-commit-ms`, `xwayland-window-id`.
Fields absent for a protocol are omitted, not `#f`.

**Tasks.**
- Collect the fields from `MindeState` / Smithay toplevel data; add
  `pid` via `wl_client` credentials, `xwayland-window-id` via `X11Wm`.
- Register the gsubr; document in `doc/api.md`; add to the catalog.
- Scheme wrapper `(window-info id)` in `(minde windows)` with the
  redaction switch applied to `title`.
- Extend `mirror-drift` to compare `geometry` here with the placed
  rect cache (should be identical; makes the new primitive the single
  geometry source).
- e2e: foot and xterm scenarios assert `kind`, `pid` > 0, `parent`
  `#f`; a GTK dialog scenario asserts `parent` set.

**Acceptance.**  `mindectl eval '(window-info (focused-window-id))'`
answers within the existing IPC budget; e2e passes; no new mirror.

### A2. `all-windows` with filters and a JSON projection  (S)

`(windows #:app-id "firefox" #:kind 'dialog #:group "work"
#:visible? #t)` returning `window-info` alists.  `mindectl query
windows --json [--app-id ...]`.  Depends on A1.

### A3. Window lifecycle events carry the info alist  (S)

`new-window`, `destroy-window`, `focus-window` hook payloads and the
event socket line gain the A1 alist (redacted) instead of a bare id,
behind a schema bump.  `title-change` and `state-change`
(maximize/fullscreen/parent set) become first-class hooks; today
`handle-window-title-change!` exists but no hook fires.  Keep the id
as first element for compatibility.  Depends on A1.

### A4. Layer-shell and non-managed surfaces are listable  (S)

Panels (eww), backgrounds, launchers and lock screens are invisible to
the policy layer today.  `(wm-layer-surfaces)` → list of alists with
`namespace`, `layer`, `anchor`, `exclusive-zone`, `output`,
`geometry`.  Needed to explain "why is the usable area smaller" and
to let an agent avoid covering a bar with a float.

---

## Epic B — Seeing the screen

Screenshots are the agent's eyes.  The primitive exists; it needs to be
precise, easy, and self-describing.

### B1. Synchronous `screenshot` wrapper with timeout  (S)

`(screenshot path #:window id #:frame idx #:output id #:timeout-ms
2000)` in `(minde compositor automation)` (new module).  Calls
`wm-screenshot`, then spins the event loop via `run-after` until the
`automation-result` arrives or the timeout expires; returns
`(ok path w h)` or `(failed reason)`.  Frame capture computes the rect
from the frame tree (gaps and reservations included) and passes it as
a region.  Requires B2 for region capture.

### B2. Region capture in `wm-screenshot`  (M)

Accept `(x y w h)` in global logical coordinates in addition to a
window id.  Scale-aware on mixed-DPI setups: return the pixel size and
the scale in the completion event.  Extend `src/png.rs` with cropping
before encoding rather than after.  e2e: capture a frame containing
foot, assert PNG dimensions equal the frame rect × scale.

### B3. Screenshot sidecar: geometry map  (S)

Alongside the PNG, optionally write `path.geometry.scm`: the capture
rect, scale, and for every visible window intersecting it, its id and
rect in *image pixel coordinates*.  An agent looking at the image can
then map "the thing at (412, 88)" back to a window id without a
second query, and compute click targets.  Depends on A1, B2.

### B4. Cursor and overlay visibility flags  (S)

`#:cursor? #f` and `#:overlays? #f` so automation captures do not
include the pointer or frame-number overlays.  Requires the render
pass to skip those elements for capture requests.

### B5. Output enumeration with modes and scale  (S)

`(wm-outputs)` exists; make sure it returns `name`, `make/model`,
`logical rect`, `physical mm`, `scale`, `transform`, `refresh-mhz`,
`enabled?`, and a `connector` string.  Add a `heads-change` hook
payload that includes the full list, and an `output-added` /
`output-removed` pair.  This is the "new monitor" trigger.

---

## Epic C — Explaining what the policy layer did

An agent that can observe but not explain will fix the wrong thing.

### C1. Placement trace  (M)

Every `handle-window-map!` records why the window went where it went:
`(placed id #:group g #:frame f #:reason (rule "firefox" #:frame 2)
| (current-frame) | (float #:pip) | (parent-follow 12))`.  Ring
buffer of the last 256 decisions in Scheme; `(placement-trace [id])`
returns them; `mindectl query trace --json`.  Also fired as a
`placed` hook so the event socket carries it.

### C2. Focus trace  (S)

Same for focus changes: `(focused id #:from previous #:reason
(key "C-t n") | (new-window) | (client-request activation-token)
| (pointer))`.  Answers "why did focus jump".  Depends on the
input-side attribution in C3.

### C3. Attribute policy actions to their trigger  (M)

A dynamic parameter `current-trigger` set by the Rust→Scheme entry
points: `(key "C-t n")`, `(ipc client-pid)`, `(hook new-window 12)`,
`(timer)`, `(repl)`.  Traces in C1/C2 read it.  This is also the hook
for ownership tagging in D1: definitions made while `current-trigger`
is `(ipc ...)` default to that connection's tag.

### C4. `explain-layout`  (S)

`(explain-layout)` → the frame tree annotated with each leaf's rect,
its windows, which window is current, and the dynamic-layout master
ratio.  Essentially `dump-frames` plus geometry plus prose-free
labels; the JSON projection is what an agent renders before deciding
a split.

### C5. Structured eval errors  (S)

`mindectl eval` returns, on error, an s-expression `(error #:key
wrong-type-arg #:message ... #:procedure ... #:args (...) #:where
(file line))` with the *innermost* frame from the backtrace, instead
of a Guile backtrace dump.  `--json` projects it.  Success replies
stay unchanged.

---

## Epic D — Ownership and take-back

Temporary programs installed through the REPL need a lifecycle.  The
risk the user named: ownership and state tracking becoming a nuisance
when information is lost.  The design below therefore never retracts
anything implicitly and never drops a snapshot on its own.

### D1. Owner tags on every registry  (M)

`register-command!`, `register-key!`, `add-event-hook!`,
`add-placement-rule!`, `define-layout!`, `run-after` gain an optional
`#:owner tag`.  Default: the value of `(current-owner)`, a parameter
that defaults to `'user`.  `(with-owner 'meeting-macros body ...)`
sets it lexically; the IPC entry point may set it from a
`--owner` flag on `mindectl eval`.  Each registry stores the tag next
to the entry.

Never: auto-assign a tag from the IPC connection alone.  The agent
must opt in, so definitions typed by the user through `mindectl`
remain `'user`.

### D1a. Public unbind for keys and prefix keys  (S)

**Motivation.**  `bind-key!` and `bind-prefix-key!` (scheme/init.scm)
have no public inverse; removing a temporary binding means
`hash-remove!` on `%prefix-bindings` or `%keybindings` directly
(use-cases/04-key-macros.md).  Without it, D2's `retract!` cannot
remove keys cleanly and neither can a user.

**Design.**  `(unbind-key! mods key)` and `(unbind-prefix-key! key)`
remove the binding, its doc entry, and any submap relationship
recorded by `set-binding-submap!`.  Return the removed thunk or `#f`.
Nested keymaps: `(unbind-in-keymap! keymap key)`.  The which-key echo
and generated key map update on the next render.  Bindings carry the
owner tag from D1 so `retract!` can call these.

**Tasks.**  Implement next to the bind functions; export from init.scm
alongside `bind-key!`; document in doc/keybindings.md; add a
`tests/keybindings-test.scm` case that binds, unbinds, and asserts the
key map no longer lists it.  Also move `bind-key!`, `bind-prefix-key!`
and the new unbinders into a documented `(minde keys)` module so they
appear in the API catalog; today they live in init.scm and are
invisible to `api-catalog`.

### D2. `owned-by` and `retract!`  (S)

`(owned-by tag)` → alist of `commands`, `keys`, `hooks`, `rules`,
`layouts`, `timers` with entries.  `(retract! tag)` removes exactly
those, cancels the timers, and returns what it removed (so the agent
can report and so it can be re-applied).  Refuses `'user`.  Depends
on D1.

### D3. Retract on IPC disconnect is opt-in  (S)

`mindectl eval --owner X --retract-on-exit` registers the tag to be
retracted if that client disconnects.  Default off: a temporary
program should survive its author's session unless asked otherwise.
The event socket announces `(owner-retracted tag reason)`.

### D4. Desktop snapshots as first-class values  (M)

`(snapshot-desktop [label])` stores `dump-desktop` plus float
geometries, group order, current group/head, and the owner tag, in a
bounded ring (default 16, oldest evicted, eviction announced).
`(snapshots)` lists them with timestamps; `(restore-desktop!
snapshot-or-label)` restores.  Windows that no longer exist are
skipped and reported; windows that appeared since are left where
they are and reported.  Snapshots optionally persist to
`$XDG_STATE_HOME/minde/snapshots/` so a compositor restart does not
lose them.  Replaces the hand-rolled `before`/`restore-frames!`
pattern.

### D5. `with-session`  (M)

```scheme
(with-session 'review #:timeout-ms (* 90 60 1000)
  body ...)
```
Takes a snapshot, sets `current-owner`, runs body.  On normal end,
error, or timeout: `retract!` the tag, then `restore-desktop!`.  Both
steps run even if the other fails; failures are logged and returned
as data.  `(sessions)` lists active ones with remaining time;
`(end-session! tag)` ends one early.  Depends on D1, D2, D4, and the
transactional application work already in doc/architecture.md.

### D6. Reload preserves runtime definitions per policy  (S)

`(reload-configuration!)` today rebuilds registries from the config.
Decide and implement: entries owned by tags other than `'user` and
`'config` survive reload (they are a running session, not
configuration); `'config` entries are replaced.  Document it.  Add
`--drop-sessions` to force the old behaviour.

---

## Epic E — Confirmed input injection

Injection is write-only.  An agent that types into the wrong window
does damage silently.

### E1. Target-addressed injection  (S)

`wm-send-key`, `wm-send-string`, `wm-click`, `wm-scroll` accept an
optional window id.  Rust focuses the target for the duration of the
injection and restores keyboard focus and pointer position after,
atomically with respect to other input.  Removes the focus/restore
dance from user code.

### E2. Injection receipts  (M)

Injection returns a token; the completion event reports whether the
target surface committed a new buffer within N ms afterwards
(`(injected token #:committed? #t #:frames 2)`).  Not proof the app
understood the key, but proof it was alive and repainted.  Combined
with a B1 screenshot this is the agent's feedback loop.

### E3. Injection guard  (S)

A configurable predicate `(injection-allowed? window-info)` consulted
before any injection; default allows everything except windows whose
app-id matches a small deny list (password managers, polkit agents,
lock screens by layer).  Denials are logged and returned as data, not
silently dropped.  Depends on A1.

### E4. Clipboard read with consent  (S)

`wm-request-paste` exists.  Expose `(clipboard-text #:timeout-ms)`
returning the text or `#f`, and make the consent rule explicit: only
when the requesting evaluation's `current-trigger` is a key or a
command the user invoked, or when a configuration flag allows IPC
reads.  This is the one content-bearing read the compositor can
legitimately offer.

---

## Epic F — The event journal

Push events are lost if nobody is listening.  An agent that connects
after the fact needs "what happened since".

### F1. Sequence numbers on every event  (S)

Every line on the event socket and every hook payload carries a
monotonically increasing `seq`.  `status.json` already has a
sequence; unify.

### F2. Bounded journal with replay  (M)

Keep the last N (default 4096) events in Scheme.  `(events-since seq
[#:names '(new-window focus-window)])` returns them.  `mindectl
subscribe --events --since SEQ` replays then follows.  The journal
respects redaction.

### F3. `describe-desktop`: one call, whole picture  (S)

A convenience combining B5 outputs, A2 windows, C4 layout, `groups`,
active `sessions`, `snapshots` summary, `owned-by` summary for every
non-user tag, and the last 20 journal entries.  This is what an agent
calls first.  JSON projection under `mindectl query desktop --json`.
Depends on most of the above; ship incrementally with whatever
exists.

---

## Epic H — Annotation layer: drawing on the live screen

Spectacle-style markup, but without the screenshot step.  The
compositor already renders overlay elements (`wm-add-overlay` draws
the frame-number labels), so a drawing layer is an extension of an
existing render path, not a new subsystem.  Two directions of use:

- **User → agent.**  The user presses a key, circles a misrendered
  widget with the pointer, types a note.  The agent receives the
  shapes in logical coordinates *and* the window ids underneath them,
  then takes a screenshot with the annotations baked in.  "Fix this"
  becomes precise without the user opening an image editor.
- **Agent → user.**  The agent highlights the windows it is about to
  move, or draws the frame split it proposes, and asks for
  confirmation through `read-one-line`.  Proposals become visible
  before they are applied.

### H1. Shape overlays  (M)

Generalise `wm-add-overlay` from text-only to a shape list:
`(wm-add-overlay '(rect x y w h #:stroke "#ff4040" #:width 3
#:fill #f) )`, plus `ellipse`, `line`, `arrow`, `polyline` (freehand),
and the existing `text` with `#:size` and `#:background`.  Each call
returns an overlay id; `(wm-remove-overlay id)`, `(wm-clear-overlays
[#:owner tag])`.  Overlays are in global logical coordinates, live
above all windows and layer surfaces, and are damage-tracked so an
idle annotation costs no repaints.  Owner tag from D1 applies so
`retract!` clears an agent's drawings.

### H2. Annotation mode: pointer grab that records shapes  (M)

`(annotate! #:tool 'rect|'ellipse|'freehand|'arrow #:colour ...)`
starts a compositor grab (same pattern as `grabs/move_grab.rs`):
pointer motion draws a live preview overlay, button release commits
it, Escape ends the mode, Return ends it and fires the hook.  Keys
during the mode switch tools (`r`, `e`, `f`, `a`, `t` for a text
label via `read-one-line`, `u` undo).  On end:
`(run-event-hook! 'annotation shapes)` where each shape carries its
geometry, tool, colour, the window ids it intersects (A1), and the
frame index.  Bind under the prefix by default, e.g. `C-t D`.

### H3. Bake annotations into screenshots  (S)

`screenshot` (B1) gains `#:annotations? #t` (default when any overlay
exists) so the PNG includes the drawn layer.  The B3 sidecar lists
the shapes alongside the window rects, so an agent gets the same
information in both image and data form.  Capture with
`#:overlays? #f` (B4) still hides frame-number labels while keeping
annotations; the two overlay classes are distinguished.

### H4. Annotate-and-send command  (S)

A registered command `annotate-report`: enters H2, on Return takes an
H3 screenshot to `$XDG_STATE_HOME/minde/annotations/<timestamp>.png`,
writes the sidecar next to it, and publishes an
`annotation-report` event with both paths.  An agent subscribed to
the event socket picks it up without polling.  This is the
one-keystroke "look at this" path.

### H5. Agent-side highlight helpers  (S)

Scheme conveniences over H1: `(highlight-window! id #:colour
#:label)`, `(highlight-frame! idx)`, `(show-proposed-layout! spec)`
which draws the frame rectangles a layout spec *would* produce without
applying it, and `(clear-highlights!)`.  These are what an agent uses
to say "I mean this one" before acting.

### H6. Annotation persistence and replay  (S, optional)

Shapes are values; `(annotations)` lists live ones, and an
`annotation` hook payload can be stored in the journal (F2).  Replaying
a shape list onto a later screen state, e.g. after a fix, lets the
user compare before and after with the same markup.

**Open questions for H.**
- Freehand polylines at pointer rate can be thousands of points; the
  render element should simplify (Douglas–Peucker) before storing.
- Overlays must not receive input themselves: the grab owns the
  pointer during H2, and committed overlays are pass-through.
- Mixed-DPI: shapes are logical, rendered per output at its scale;
  the sidecar reports pixel coordinates per output.
- Whether text labels use the existing overlay font path or need
  a proper glyph cache; the frame-number path is fine for short
  labels.

## Epic G — Beyond the compositor's knowledge (research)

The compositor sees surfaces, not content.  These are bridges; each
is a separate decision and possibly a separate package.

### G1. AT-SPI bridge  (L, research)

GTK, Qt, Firefox, Chromium and LibreOffice expose accessibility trees
over D-Bus.  A helper (outside the compositor, Scheme or Python) that
maps a `window-info` `pid` to its AT-SPI application and returns the
focused widget, its role, name, and text.  This turns "type into the
Calc window" into "put this in cell B7".  Decide whether it belongs
in guile-minde-ui, a new `minde-bridge-atspi`, or is out of scope
and merely documented as a recipe.

### G2. Foreign-toplevel and idle protocols  (M)

Implement `ext-foreign-toplevel-list-v1` so external tools (and the
Eww panel) get the same window list minde's IPC has, and
`ext-idle-notify-v1` so agents can observe user idleness rather than
poll `wm-idle-ms`.

### G3. Xwayland window properties  (S)

For X11 clients expose `WM_CLASS`, `WM_WINDOW_ROLE`, `_NET_WM_PID`
and `WM_TRANSIENT_FOR` through A1's alist.  Xwayland clients are the
ones most likely to misbehave and least likely to have a Wayland
identity.

---

## Epic I — Defect backlog (adversarial review, 2026-09-06)

Findings from a two-lens review of the ~18 commits ending at 402e537
(plus the uncommitted `layout_needs_repack` gate).  Verified against the
vendored Smithay 0.7 checkout.  Severity ordered.

### I1 — Deadlock: pointer-grab hooks re-enter the pointer mutex (S, high)

Smithay dispatches grab callbacks while holding the pointer's
non-reentrant mutex; `move_grab.rs:66` / `resize_grab.rs:219` call
`guile::on_window_moved` from inside that dispatch.  A
`handle-window-move!` hook that issues any pointer-touching command
(`wm-warp-pointer!`, `wm-click`, `wm-scroll`) re-locks the same mutex
on the same thread and freezes the compositor.
Fix: defer the hook to a calloop idle callback (or queue commands while
in grab context).  Acceptance: hook calling `wm-warp-pointer!` on drag
release does not hang; e2e scenario added.

### I2 — GC hazard: heap `Scm`s in Rust `Vec`s across allocations (S–M, high)

`on_heads_changed`, `wm_timing_stats`, `on_input_device_added`,
`wm_outputs` (src/guile/mod.rs) collect heap-allocated conses/strings
into `Vec<Scm>` while continuing to allocate.  bdw-gc does not scan the
Rust heap, so a GC mid-loop can collect earlier values
(use-after-free).  The 600d853 lint skips `src/guile/*`.
Fix: build lists incrementally in a stack-held `Scm` (cons as you go,
reverse at the end) instead of `Vec<Scm>`; extend the lint to cover
src/guile.  Acceptance: no `Vec<Scm>` of heap objects crossing an
allocating call remains.

### I3 — Mid-apply re-advertisement causes duplicate events (M, high)

`enable_head`/`disable_head` call `update_usable_area()` inside
`apply_output_configuration`'s per-head loop (udev.rs:920, :1061),
advertising a half-applied layout with a bumped serial before
`succeeded()`.  shikane files a corrective config, is cancelled by the
serial bump, retries a no-op — which fires `handle-heads-change!` and
`handle-output-configured!` again because `output_configuration_applied`
clears `reported_heads` unconditionally (state.rs:2181).  This is the
root of the eww duplicate-bar workaround in the user config.
Fix: suppress the refresh/hook inside the apply path (post-apply
notification already covers it); drop the unconditional
`reported_heads.clear()`.  Acceptance: one profile application fires
each hook exactly once; the `eww close` workaround becomes redundant.

### I4 — Layer-shell changes bump the output-management serial (S, medium)

`update_usable_area` (state.rs:2422) resends full head state + `done`
whenever usable rects change — an eww bar mapping/unmapping is not
protocol-visible output state, yet it makes shikane re-evaluate and
cancels in-flight configurations (feeds I3's loop).
Fix: only `output_management_refresh` when Output-level state
(mode/position/scale/transform/enabled/identity) changed.

### I5 — Key repeat survives session pause (S, medium)

`SessionEvent::PauseSession` (udev.rs:459) never calls
`cancel_key_repeat()`; a held key + `chvt`/suspend leaves the repeat
timer firing the key hook until the next local key press.
Fix: cancel in the pause arm.  Acceptance: repeat stops on VT switch.

### I6 — Unbounded re-entrancy in `handle-keyboard-layout-changed!` (S, medium)

`refresh_keyboard_layouts` fires the hook with no depth guard; a hook
that switches layouts recurses (stack overflow) or ping-pongs through
the command queue.  Fix: re-entrancy guard around the hook (skip the
nested fire, log once).

### I7 — `ExplicitPosition` is permanent (S, low)

Set on any positioned apply — even one later reverted — and never
removable; no head can return to auto layout (state.rs:2194).
Fix: unmark on revert; consider a `#:position 'auto` escape hatch.
Status: revert-correctness fixed (the flag is now a mutable
`AtomicBool`; a reverted configuration unmarks heads it positioned).
Follow-up: a user-facing explicit→auto escape hatch
(`configure-output! #:position 'auto`) is deliberately deferred — it
grows the Scheme API; the `mark_position_auto` primitive it would need
already exists.

### I8 — `next!`/`previous!` skip their sync (S, low)

0d39ccb moved the trailing `(sync-frames!)` behind the fallback branch;
`focus-window-by-id!`'s three silent no-op branches now skip placement
entirely (frames.scm:~1783/~1858).  Fix: sync (or at least run
`%sync-hook`) on the no-op branches too.

### I9 — Failed enable leaves requested mode on the disabled output (S, low)

`enable_head`'s failure path keeps the just-failed mode/position
(udev.rs:898-914), so a later "enable without mode" can retry the exact
mode that failed; `udev_set_mode` on a powered-off head records the
mode unvalidated, and the later power-on can fail dark (udev.rs:1525).
Fix: restore prior state on failure; validate modes on powered-off
heads.

### I10 — Watchdog/vblank mis-accounting after a driver hiccup (S, low)

After `vblank_watchdog_fired` gives up on a flip, the old flip's late
vblank is attributed to the new one (udev.rs:1194, :1282).
Self-healing (one stutter); fix by tagging flips with a sequence.

---

## Suggested order

1. **A1, C5, B1, B2** — the agent can see a window's facts, get a
   useful error, and take a precise screenshot.  Every later example
   in the discussion becomes testable.
2. **D1, D1a, D2, D4** — definitions have owners, sessions can be taken
   back, snapshots are values.  This is what makes handing the
   compositor a program safe.
3. **H1, H2, H3** — shape overlays, annotation mode, baked
   screenshots.  Small on top of the existing overlay path, and the
   most visible payoff: the user circles a problem on the live
   screen and the agent receives shapes, window ids and an image.
4. **C1, C3, A3, B5** — traces and richer events: the agent can
   explain, not only observe.
5. **E1, E2, E3** — injection becomes addressed, confirmed and
   guarded.
6. **D5, D6, F1, F2, F3, H4, H5** — `with-session`, reload semantics, the
   journal, and the one-call description.
7. **G** and **H6** as separate decisions.

## Non-goals

- Reading window content through the compositor.  Screenshots and the
  AT-SPI bridge are the two sanctioned routes.
- Auto-tagging user actions.  `'user` is the default owner and stays
  untouched by `retract!`.
- A second IPC protocol.  Everything lands on the existing eval and
  event sockets with s-expression shapes and JSON projections.
- Hardware-only observability.  Everything above must be testable on
  the nested backend.

## Open questions

- Should `window-info` include `pid` by default or only when
  `redact?` is off?  PIDs are not content, but they let a bar
  correlate with process lists; lean towards always.
- Ring sizes (snapshots 16, journal 4096, traces 256) are guesses;
  make them configuration keys from the start.
- `with-session` timeout on an idle machine: should the countdown
  pause while the session is locked?  Probably yes; needs
  `session-lock` integration.
- Whether `retract!` of a tag that owns a group should delete the
  group or only stop owning it.  Lean towards: never delete windows'
  containers, merge the group into the previous one and report.
