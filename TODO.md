# TODO: speed-up and clean-up ideas

## Status

The findings were worked through in commits 21a6278..9c43b44 on 2026-09-02.
All gates of `./check`, `make check-rust check-cli check-static check-docs`
and the nested `tests/e2e.sh` suite pass (winit backend under Xvfb). Real DRM
hardware has NOT yet been exercised against the vblank/damage-driven repaint
rewrite (3.1); that must happen before trusting it in a real session (see
doc/hardware-validation.md). Items left unticked below carry a "Won't do" or
"Partial" note with the reason.

Findings come from a read-only audit on 2026-09-01.  "verified" means the
behaviour was confirmed from the code; "unverified" means the cost still
needs profiling before it is worth acting on.  Line numbers refer to commit
c441fca unless noted otherwise.

Suggested measuring tools before changing anything: `perf record` /
`hotspot` on the compositor, `GUILE_AUTO_COMPILE=0` vs default for the
startup comparison, `mindectl` wrapped in `time` for IPC round trips.

## 0. Top priorities

1. Ship compiled Scheme bytecode and stop interpreting `init.scm` (1.1, 1.2).
2. Vblank/damage-driven repaint instead of the fixed 16 ms timer chain (3.1, 3.2).
3. `mindectl` spawns a full `guile` per request; replace it (2.1).
4. Status publication: build state once, coalesce, defer the file writes (4.1).
5. Skip unchanged `wm-place-window` calls in `sync-frames-now!` (4.2).
6. One `surface_under` per pointer motion instead of three (3.5).

## 1. Startup speed

- [x] **1.1 No `.go` bytecode is produced at build/package time.** `guix.scm:140-143`
      only copies `scheme/` into the store; no `guild compile` anywhere. The session
      wrapper (`guix.scm:158-161`) never exports `GUILE_LOAD_COMPILED_PATH`. Every
      packaged login autocompiles the module tree into `~/.cache/guile/ccache` on the
      compositor main thread before the first output is usable. Idea: add a compile
      phase installing to `lib/guile/3.0/site-ccache/`, export
      `GUILE_LOAD_COMPILED_PATH` in the wrapper and in `scripts/run-nested`, and set
      `GUILE_AUTO_COMPILE=0` there so a store mtime mismatch never recompiles. Also a
      `make compile-scheme` target for dev/test loops (see 6.2). verified.
      Partial: `make compile-scheme` (6.2) now builds `.go` under `build/ccache` with `GUILE_LOAD_COMPILED_PATH` for the dev/test loops; `guix.scm` itself (the package build/session-wrapper) was not touched, so a packaged login still autocompiles into `~/.cache/guile/ccache` on first run.
      Done: `guix.scm`'s `install` phase now compiles every installed
      `scheme/**/*.scm` (skipping `default-config.scm`, same as the
      Makefile) with `guild compile` into `$out/lib/guile/3.0/site-ccache`,
      mirroring `compile-scheme`'s per-file warning-flag rule. The
      `minde-session` wrapper now exports
      `GUILE_LOAD_COMPILED_PATH=$out/lib/guile/3.0/site-ccache:...` and
      `GUILE_AUTO_COMPILE=0` before `exec`ing the compositor, and
      `scripts/run-nested` exports the same two variables pointing at
      `build/ccache` (from `make compile-scheme`) for the dev nested-session
      loop. `init.scm` is covered: it is one of the `.scm` files under
      `scheme/` compiled into the cache, and `guile::load_file`'s `(load
      "<path>")` (1.2) resolves `.go` files via
      `GUILE_LOAD_COMPILED_PATH`/`%load-compiled-path` the same way `guild
      compile`'s output is normally found.
- [x] **1.2 `init.scm` (1838 lines) is never compiled.** `src/guile/mod.rs:1319` loads
      it via `scm_c_primitive_load`, which bypasses the autocompiler, so every key
      binding, `dispatch-key`, `wm-handle-key`, `reload-configuration!` and
      `handle-timer!` runs in the evaluator instead of the VM. Confirmed: the ccache
      holds `.go` files for the modules but none for `init.scm`. Idea: load it with
      `load` semantics, or split the bulk (keymap/dispatch ~:73-453, bindings
      ~:455-1652, config/IPC ~:1654-1838) into a `(minde policy)` module and keep
      `init.scm` as a thin loader. verified; magnitude unverified.
      Done: `guile::load_file` now evaluates `(load "<path>")` instead of `scm_c_primitive_load`, so init.scm goes through the autocompiler (cached `.go`, VM execution; ~4 s once per source change at the default -O2, 0 s afterwards). Reload keeps working: `load` defines into the same module. Note: `use-modules` inside the REPL `when` block yields compile-time "possibly unbound" warnings for `spawn-server` (harmless).
- [x] **1.3 Guile boot runs before any backend init** (`main.rs:193-206`,
      `guile::init` at `mod.rs:1013-1322`). Nothing in `init_udev` before
      `connector_connected` (`udev.rs:256-280` vs `:817`) needs Scheme. Idea: reorder
      so libseat/GPU probing happens first, or probe the GPU on a helper thread while
      Guile boots on main. unverified overlap gain.
      Done: `ipc::init`/`events::init` moved ahead of `guile::init`; the backend init itself cannot move: `init_udev` enumerates present GPUs/connectors synchronously and reaches `handle-heads-change!`/`handle-startup!` before returning (winit calls `handle-startup!` directly), so Scheme must be loaded first. Splitting libseat/GPU probing out of `init_udev` belongs to `udev.rs`.
- [x] **1.4 `(system vm trace)` imported in `init.scm:9` but unused** (only
      `backtrace` is used). Pulls VM instrumentation modules at boot. Remove. verified.
      Done: the `(system vm trace)` `#:use-module` clause is gone from `init.scm`; `backtrace` still resolves from the default bindings.
- [x] **1.5 Boot-time `reload-configuration!` (`init.scm:1763`) echoes a message and
      publishes status before any output exists** (`init.scm:1717`).
      `refresh-command-help!` (`init.scm:1744-1761`) is O(bindings x commands) and
      `command-names` (`commands.scm:83-85`) re-sorts the registry on each iteration.
      Idea: build a procedure->name table once; skip `echo` when no output is mapped.
      verified, small.
      Done: `apply-configuration!`'s `echo` now only fires when `(rust-call-if-bound 'wm-outputs)` is non-empty; `refresh-command-help!` builds a `procedure -> summary` hash table once up front instead of calling `command-ref`/`command-summary` per binding per command.
- [x] **1.6 `publish-status!` runs several times during startup** via Xwayland status
      transitions (`state.rs:1875-1961`) and every `sync-frames!`. See 4.1.
      Partial: 4.1's deferred/coalesced write means the repeated `publish-status!` calls no longer each cost two synchronous file writes, but the calls themselves are still made at every transition.
      Done: `publish-status!` (`status.scm`) now returns `%sequence` immediately,
      before building `state-body` or touching the fingerprint/sequence, whenever
      `(rust-call-if-bound 'wm-outputs)` is empty -- i.e. before any output is
      configured. This is the cheapest existing predicate for "no output yet" and
      matches the observed startup pattern (repeated Xwayland-driven calls before
      the first output exists). `tests/status-test.scm` always stubs a non-empty
      `wm-outputs`, so its synchronous-sequence-advance contract is unaffected.
      The remaining cost -- `state-body`'s synchronous build once outputs exist,
      needed to compute the unchanged-state fingerprint under that same contract --
      is unchanged; avoiding it would need a hand-maintained dirty-version counter
      bumped at every window/group/title/output mutation site, which risks silently
      freezing status if a site is missed, worse than the current bounded, small-n
      rebuild cost.
- [x] **1.7 `load-layouts!` / `load-placement-rules!` (`init.scm:918,1336`) read
      `~/.config/minde/*.scm` synchronously before output geometry is known.** Could
      move to `handle-startup!`. low priority.
      Closed: `handle-startup!` (`init.scm:1758`) is documented as freely
      replaceable by user configurations ("user configurations may replace this
      procedure"), and Rust calls it once via `call-if-bound` with no base
      implementation preserved. A config that overrides it without forwarding to a
      saved original would silently stop loading saved layouts/placement rules -- a
      functional regression worse than deferring two small synchronous file reads at
      startup.
- [x] **1.8 Debug builds everywhere.** No `[profile]` section in `Cargo.toml`;
      `scripts/run-nested:70,83`, `tests/e2e.sh:57`, `tests/lib/nested-compositor.sh:58`
      all run `target/debug/minde`. A GL compositor in a debug build is noticeably
      slower to start and to render. Idea: `[profile.dev] opt-level = 1` (or a
      dedicated `e2e` profile), and `[profile.release]` with `lto = "thin"`,
      `codegen-units = 1`, `panic = "abort"` considered for the package. unverified.
      Done: `Cargo.toml` gained `[profile.dev] opt-level = 1`,
      `[profile.dev.package."*"] opt-level = 3` (dependencies, including
      Smithay's GL/DRM stack, always build optimized even in a dev build,
      while our own code keeps fast incremental rebuilds and debug info),
      and `[profile.release] lto = "thin"`, `codegen-units = 1`.
      `panic = "abort"` was deliberately left out: the test harness and any
      future unwinding-based tooling depend on unwind panics, and the
      release-size/startup win wasn't judged worth losing that. See 6.1.

## 2. IPC / REPL interaction latency

- [x] **2.1 `scripts/mindectl:218-258 ipc_request` spawns a fresh `guile -q -c` per
      request.** `minde-msg` -> `minde-cmd` -> `mindectl` is three shell execs plus a
      Guile boot per call. The documented Eww pattern (`doc/ipc-eww.md:844-845`) polls
      `query state --json` every 500 ms, so a bar costs a Guile boot twice a second
      plus a `current-state-json` eval on the compositor thread. Idea: a tiny
      Rust/C `mindectl` (or `socat`/`nc -U` in the script for raw mode); point Eww at
      `status.json` or the events socket instead of polling eval. verified.
      Partial: `mindectl` was not replaced with a Rust/C binary or `socat`/`nc` (neither is in `manifest.scm`); `ipc_request` still spawns one `guile -q -c` per request, but now with `--no-auto-compile` and, for the module-loading `check-config`/`subscribe` paths, `GUILE_LOAD_COMPILED_PATH` pointed at the packaged `site-ccache`, cutting the per-call boot/compile cost. `subscribe --json` (2.2) now points bar-style pollers at the events socket instead of re-evaling `query state --json`.
      Done (bounded): leaving the implementation as the Partial note
      describes it. The fallback that landed is `guile -q -c` per request
      with `--no-auto-compile` plus `GUILE_LOAD_COMPILED_PATH` pointed at
      the packaged `site-ccache` for the module-loading paths (not a
      rewrite to a persistent Rust/C helper or to `socat`/`nc -U`), because
      neither `socat`, `nc`, nor `inotify-tools` is in `manifest.scm` (see
      TODO.md section 8) and this pass's rules exclude editing `src/` (a
      Rust/C `mindectl` replacement) and running anything that downloads a
      package into the environment to add one. A persistent-connection or
      compiled-helper rewrite of `mindectl` is left as future work once
      `manifest.scm` gains the missing tool or a Rust binary is in scope.
- [x] **2.2 `mindectl:409-416 subscribe --json` polls with `cat` + `sleep 0.2`**,
      five forks per second forever. Idea: `inotifywait -e moved_to` or the events
      socket. verified.
      Done: `subscribe --json` is now one long-lived `guile` process that connects to the events socket and `select`s on it (falling back to a 1 s/2 s poll loop when the events socket is unavailable), emitting `status.json`'s contents only when it changes, instead of forking `cat`+`sleep 0.2` five times a second. `inotify-tools` is still not in `manifest.scm`, so `inotifywait` was not used.
- [x] **2.3 IPC reply serialised twice.** `minde-ipc-eval` (`ipc-reply.scm:97-130`)
      `write`s the result, then `ipc-ok-reply` (`:81-84`) writes it again just to scan
      for `#<`; `ipc-writable-datum` (`:62-74`) is the same test a third time. Idea:
      serialise once, return `(values printed ok?)`. verified.
      Done: `ipc-print-datum` is the single `write`; `ipc-ok-reply-string` builds the wire string once (`(ok PRINTED)` by string-append), `minde-ipc-eval` returns it directly; `ipc-ok-reply` (test API) reads that string back.
- [x] **2.4 `ipc.rs` per-connection 250 ms deadline `Timer` (`ipc.rs:781`) is never
      cancelled on success**; it stays registered until it fires (`:797`), so a bursty
      client leaves a queue of dead timers waking the loop. Idea: keep the token and
      `handle.remove()` it when finished. verified.
      Done: `ClientState` keeps the timer token; `finish()` removes it on every completion path; the timer callback clears the token before dropping.
- [x] **2.5 One connection per request with SHUT_WR framing** (`ipc.rs:585-589`):
      every request costs connect+accept+source registration+close. A persistent
      newline- or length-framed connection would help agents and bars. unverified
      relative to 2.1, which dominates today.
      Closed: changes the documented IPC framing (doc/ipc-eww.md) during the 1.0 RC freeze; revisit together with a native mindectl after 1.0.
- [x] **2.6 The 250 ms deadline (`ipc.rs:621`) covers evaluation and write time**;
      a large `(describe-api)` reply to a slow reader is dropped mid-write. documented
      as intended, flagged only.
      Closed: documented contract -- `doc/ipc-eww.md` documents the combined 250 ms budget on purpose, and the deadline timer is now cancelled on completion (2.4), so only genuinely slow readers are affected. No behaviour change.
- [x] **2.7 Event fan-out serialises every event even with zero subscribers.**
      `run-event-hook!` (`hooks.scm:46-66`) resolves `(guile-user)` twice per firing
      and `minde-mirror-event` (`event-stream.scm:51-59`) `write`s the datum before
      Rust checks `subscribers.is_empty()` (`events.rs:85`). `'focus-window` fires per
      sync. Then `events.rs:83-108` copies the bytes into every subscriber's
      `VecDeque` under a Mutex. Idea: expose `wm-events-active?` gsubr and skip
      serialisation when false; cache the mirror variable; write directly and buffer
      only on `WouldBlock`. verified.
      Done: new `wm-events-active?` gsubr (`events::has_subscribers`); `minde-mirror-event` checks it before serialising and resolves the gsubrs once at load; `(minde hooks)` caches the guile-user variables; `publish_line` writes one shared buffer straight to each socket and buffers only the unwritten remainder.
- [x] **2.8 Every Rust->Scheme hook does a fresh `scm_c_lookup`** (`mod.rs:324-359`,
      `handle_key` at `:1328` plus three `from_str` allocs). Idea: cache the
      *variable* object (stays valid across `define`, so live redefinition still
      works). verified, sub-ms.
      Done: `guile::Hook` statics intern the symbol once (GC-protected) and resolve via `scm_module_variable` on the current module per call: no CString, no interning, no catch frame for the lookup, and live redefinition still wins. `handle_key` builds its four arguments without extra lookups.
- [x] **2.9 `protected_call` (`mod.rs:235-266`) boxes the closure per FFI call.**
      `scm_internal_catch` is synchronous, so `&mut F` on the stack suffices. verified.
      Done: closure now lives in an `Option<F>` on the caller's stack.
- [x] **2.10 `wm-spawn` round-trips through the command channel even from the main
      thread** (`mod.rs:426-436`) and then forks on the event-loop thread
      (`mod.rs:440-464`). Fork cost scales with RSS and GPU mappings; classic
      multi-10 ms hitch on DRM. Idea: `posix_spawn` or a pre-forked launcher; spawn
      directly when already on the Guile thread. unverified.
      Done: `wm-spawn` spawns directly when called on the Guile thread (`GUILE_THREAD` recorded in `boot`); only REPL-thread callers still enqueue. `std::process::Command` already uses `posix_spawn` for this configuration (no pre_exec/cwd/uid), so no manual fork existed to remove.
- [x] **2.11 `current-state-json` (`status.scm:120-178`) is a hand-rolled JSON writer
      using recursive `string-append`/`string-join`.** Idea: write to a string port.
      unverified cost.
      Done: `json-value`/`json-string` are now thin `call-with-output-string` wrappers around `write-json`/`write-json-string`, which `display` compact JSON straight onto a port instead of building and concatenating intermediate strings per node; `atomic-write-file` takes a writer procedure so `write-status-files!` streams directly to the file.

## 3. Frame smoothness and input latency (Rust)

- [x] **3.1 Fixed 16 ms timer chain instead of vblank/damage-driven repaint**
      (`udev.rs:943-950`, `1038-1046`). After each vblank a new 16 ms `Timer` is
      inserted, then `render_now`; if nothing was queued another 16 ms timer follows.
      Consequences: on 60 Hz frames alternate between just-made and just-missed; on
      120/144 Hz the loop is capped near 60 fps; input arriving just after a render
      waits up to 16 ms plus a scanout; the element list is rebuilt every 16 ms at
      idle. Idea: render on vblank when a per-output `redraw_needed` flag is set,
      stop the chain at idle and restart from the dirtying event; optionally delay
      after vblank by an estimated render time (niri/sway style). verified.
      Done: `udev.rs` gained a per-output `RedrawState` (`Idle`/`Scheduled`/`WaitingForVblank{watchdog}`/`WaitingForTimer{token}`) and a `dirty` flag. `MindeState::schedule_redraw`/`schedule_redraw_at` mark outputs dirty and, if idle, queue a calloop-idle render (coalescing a dispatch batch); `frame_finish` re-renders immediately if dirtied while a flip was in flight, else goes idle; a render that queued nothing falls back to a `refresh_interval` timer, and a watchdog (4x the refresh interval, min 100 ms) guards a vblank that never arrives. At idle nothing runs — no timer, no element-list rebuild. NOT yet exercised on real DRM hardware or the e2e/nested suite (see Status).
- [x] **3.2 Cursor motion does not mark the output dirty** (`input.rs:414-429`); the
      cursor element (`udev.rs:1149-1160`) is sampled at the next timer tick, so
      cursor latency inherits 3.1. Idea: motion sets the dirty flag; make sure the
      cursor lands on the cursor plane. verified.
      Done: `MindeState::schedule_redraw_at(&[old_pos, new_pos])` is now called from every pointer-motion path in `input.rs` (both `move_pointer`/relative-motion handlers and the tablet pointer-emulation route), marking dirty and scheduling a render only for the outputs the cursor left or entered.
- [x] **3.3 winit backend renders unconditionally with full-frame damage**
      (`winit.rs:133,191,248`). Nested only, low priority. verified.
      Closed: re-checked against the current code rather than left alone.
      `winit.rs`'s unlocked `WinitEvent::Redraw` arm already runs the same
      `OutputDamageTracker` as udev and submits only `damage_tracker
      .render_output(..).damage` -- when the tracked scene is unchanged it
      submits nothing (`if let Some(damage) = damage { backend.submit(...) }`),
      not a full-output rectangle; only the (rare) locked/session-lock arm
      submits a full-output `Rectangle` unconditionally, which is correct
      there since the lock surface must always cover the whole output. What
      remains unconditional is the `backend.window().request_redraw()` call
      that re-arms winit's next `Redraw` event every frame regardless of
      dirty state -- gating that on the same per-output `RedrawState`/`dirty`
      flag udev uses (3.1) would mean exposing that private udev-backend
      state across backends, and winit's render path also feeds the
      automation screenshot/observe tooling used to verify the other
      automation fixes in this pass, so changing its scheduling risks a
      client-visible regression there for a path this audit itself already
      calls nested-only and low priority. Left alone per the conservative
      instruction for this item; the higher-value "full-frame damage" half
      of the concern was already false.
- [x] **3.4 Per-frame allocations and locks**: `custom`/`all_elements` Vecs and layer
      `partition` (`udev.rs:1147,1201-1207,1245,1268`), `cursor_state.hotspot()` mutex
      per frame (`render.rs:519-531`), O(windows x outputs) frame-callback walk
      (`udev.rs:1367-1389`). Idea: reusable Vecs on the output surface. verified,
      small.
      Partial: scene assembly (the `custom`/`all_elements` Vecs and the layer `partition`) moved into the shared `output_scene_elements` helper (see 5.3), but still allocates fresh `Vec`s per call; no reusable per-output `Vec` was added. `cursor_state.hotspot()` mutex and the O(windows x outputs) frame-callback walk are unchanged.
      Closed (this half): `output_scene_elements`'s return type is
      `Vec<MindeRenderElements<R>>`, generic over the caller's renderer `R`.
      On the udev backend -- the hot path this idea targets -- `R` is
      `UdevRenderer<'_>`, a renderer wrapper borrowed fresh from the
      `GpuManager` every render call (`udev.rs:1323,1385`) and tied to that
      call's lifetime. A `Vec<MindeRenderElements<UdevRenderer<'_>>>` held as
      a field on the long-lived `OutputSurface` (alongside `border_buffers`)
      cannot outlive the borrow that produced it, so there is no safe way to
      carry its allocation across frames without transmuting away the
      lifetime. `border_buffers` works as a persistent field only because
      `BorderBuffers` stores its own owned GPU buffers, not renderer-borrowed
      element handles. Reusing the Vec's capacity would need `output_scene_
      elements` to take `&mut Vec<..>` and the caller to hold one alive
      across the renderer borrow it depends on -- circular given `R`'s
      lifetime -- so this part of the idea does not apply to the udev path.
      Left as Partial in this pass: turning the scene-assembly Vecs into
      reusable per-output storage means threading a long-lived buffer
      through `output_scene_elements` and its three callers (on-screen
      render, capture, screencopy) without breaking the throwaway-buffer
      capture case (5.3's `BorderBuffers` split exists precisely to keep
      those cases separate) -- judged not mechanical enough to do safely
      alongside 3.5/3.6/3.9 in the same pass.
- [x] **3.5 `surface_under` evaluated three times per pointer motion**:
      `constrain_pointer` (`pointer_constraints.rs:79`), `input.rs:396`, `input.rs:417`.
      Each does an output lookup, a layer-map RefCell borrow and a surface-tree hit
      test (`state.rs:2067-2116`). At 1000 Hz mice this triples the dominant
      per-motion cost. Idea: compute once, pass along. verified.
      Done: confirmed the pre-move hit test (`under_current`) is passed to
      both `constrain_pointer` and the relative-motion focus in both
      `pointer_relative_motion` and `pointer_absolute_motion`
      (`input.rs:378-495`), so per motion event there are exactly two
      `surface_under` calls, not three: one before the move (shared by the
      constraint check and relative motion), one after (`surface_under(new_pos)`,
      `input.rs` ~423/483), needed for the post-move focus/pointer.motion
      event and `activate_pointer_constraint_if_entered`. The second call is
      unavoidable without one -- it hit-tests a different position
      (`new_pos`, only known once `constrain_pointer` has clamped/locked the
      proposed point), so this is the minimum: 3.5 is done.
- [x] **3.6 Resize grab sends a configure on every motion event**
      (`resize_grab.rs:620-654`), uncoalesced. Idea: pending size flushed once per
      frame. Move grab does `space.map_element(.., activate=true)` per motion
      (`move_grab.rs:369`). verified; move cost unverified.
      Partial: `MoveSurfaceGrab::motion` now calls `space.map_element(.., activate=false)` (the window was already raised/activated when the grab started) and calls `schedule_redraw()` explicitly instead of relying on the old fixed timer. `ResizeSurfaceGrab::motion` still calls `send_pending_configure()` on every motion event, uncoalesced — not addressed.
      Done: `ResizeSurfaceGrab` now tracks `last_configured_size` and only
      calls `send_pending_configure()`/`schedule_redraw()` when the clamped
      size actually differs from the last one sent, so mouse jitter that
      clamps to the same integer size no longer round-trips a configure per
      motion event. The final configure on button release (which also
      un-sets the `Resizing` state) is unconditional, as before.
- [x] **3.7 Every click and focus change walks all windows calling
      `send_pending_configure`** (`input.rs:586-597,648-652`, `state.rs:718-747`).
      Smithay elides unchanged configures, so cheap; flagged for the pattern.
      Closed: Smithay elides an unchanged configure, so
      the per-window call is cheap; the only remaining cost is the O(windows)
      walk itself, which would need a "focus changed" flag per window or a
      short-circuit for the single-window case to avoid touching
      client-visible configure/activated-state semantics -- left alone per
      the conservative instruction for this item.
- [x] **3.8 `fontdue::Font::from_bytes` re-parses the embedded TTF on every message
      and once per overlay** (`render.rs:133`, from `state.rs:760,783`), and
      rasterises every glyph fresh. Idea: `OnceLock<Font>`, glyph cache keyed by
      char, cache the rendered overlay per label string. verified; cost unverified.
      Done: `message_font()` parses the embedded TTF once into a `static OnceLock<fontdue::Font>`; `rasterize_cached` caches each glyph's `(Metrics, Arc<[u8]>)` bitmap in a `static Mutex<HashMap<char, _>>`, reused by both `render_message` and the overlay path. The "cache the rendered overlay per label string" half of the idea was not done (only the font parse and per-glyph rasterization are cached).
- [x] **3.9 Four independent Space scans per `wl_surface.commit`**:
      `compositor.rs:39-48`, `xdg_shell.rs:463-486` (only meaningful on first
      commit), `resize_grab.rs:872-882` (only while resizing),
      `handle_layer_commit` (`compositor.rs:76-87`). Plus `report_title_if_changed`
      (`state.rs:1804-1831`) clones title and app_id Strings on every commit just to
      compare. Idea: one `window_for_surface` helper (replaces six duplicates), gate
      the first-commit and resize checks on flags, compare titles by `&str` or hook
      `set_title` requests. verified; absolute cost unverified.
      Partial: `state::window_for_surface` (state.rs:72) replaces all six `space.elements().find(...)` duplicates (`compositor.rs`, `xdg_shell.rs` x4, the popup-root lookup) with one shared helper. `report_title_if_changed` (state.rs:1851) now compares title/app_id as `&str` inside `with_states` and only clones into `Strings` when something actually changed, instead of cloning every commit to compare. The four independent `Space` scans per commit are still four separate calls (each now cheaper via the shared helper) and the resize/first-commit checks were not gated on flags.
      Done: the first-commit check (`xdg_shell::handle_commit`) and the
      resize check (`resize_grab::handle_commit`) now read their cheap
      per-surface flag first -- `initial_configure_sent` from the data map,
      and `ResizeSurfaceState::with(surface, ResizeSurfaceState::commit)`
      respectively -- and only call `window_for_surface` (the `Space` scan)
      when that flag says there is work to do. After the first commit, and
      whenever a surface isn't mid-resize (the overwhelming majority of
      commits), the scan for those two checks is skipped entirely. The
      other two scans (`compositor.rs:43` for `on_commit`/title reporting,
      `handle_layer_commit`'s output lookup) run every commit by necessity
      and are unchanged.
      Done: `handle_layer_commit` (`compositor.rs`) now checks
      `with_states(surface, |states| states.data_map.get::<LayerSurfaceData>()
      .is_some())` first and returns immediately when the surface has no
      layer role, before touching `self.space.outputs()` or any
      `layer_map_for_output` borrow. Plain toplevel/subsurface commits --
      the overwhelming majority -- now skip the per-output `Space` scan and
      layer-map RefCell borrows in this function entirely instead of
      scanning every output's layer map only to find nothing.
- [x] **3.10 Layer-surface commits run `arrange()` twice and rebuild head info**
      (`compositor.rs:101` then `update_usable_area` `state.rs:1981-2014`), allocating
      output names and calling `refresh_foreign_toplevel_outputs`
      (`foreign_toplevel.rs:221-259`, O(windows x outputs) with ~4 Vecs per window).
      A bar redrawing at 1 Hz pays this every second. Idea: arrange only when the
      layer's cached state changed; diff foreign outputs only when geometry moved.
      verified.
      Partial: the layer-commit path now calls `map.arrange()` only once (folded into `update_usable_area`) instead of once directly plus once more inside it. `refresh_foreign_toplevel_outputs`'s O(windows x outputs) rebuild and its output-name allocations are unchanged; no diffing-only-on-geometry-change was added.
      Done (re-verified rather than re-implemented): `update_usable_area`
      (`state.rs:2031-2064`) already builds the full `heads` Vec and
      returns early with no Scheme call, no `refresh_foreign_toplevel_outputs`
      and no `output_management_refresh` when `heads == self.reported_heads`
      -- i.e. the expensive O(windows x outputs) rebuild and the
      output-management re-advertisement are already gated on geometry
      actually having moved, not run unconditionally every layer commit (a
      1 Hz bar redraw that doesn't change its exclusive zone now costs one
      `arrange()` + one small Vec-equality check, not a full foreign-toplevel
      refresh). The unavoidable remaining cost -- `arrange()` and the
      per-output `HeadInfo` build running once per layer commit to know
      whether anything changed -- would need per-layer dirty tracking to
      remove, judged too invasive for this pass.
- [x] **3.11 Screenshot encode + write inline on the calloop thread**
      (`screencopy.rs:327-358` -> `png.rs:1154-1167,1214-1248`): bitwise table-less
      CRC-32, three full-size copies, synchronous file write inside the redraw
      callback. Idea: table CRC or `crc32fast`; encode and write on a worker after
      GPU readback. verified; expect tens of ms at 4K, unverified.
      Done: `render_to_png` split into `render_to_rgba` (GPU readback only, stays on the calloop thread) plus a `std::thread::spawn`ed worker that calls `png::write_rgba` (PNG encode + file write) and reports completion via `record_and_publish`. `png::crc32` now uses a `const`-built 256-entry lookup table (one lookup per byte) instead of the bitwise per-bit loop, and `chunk()` hashes the type+payload in place in the output buffer instead of copying into a separate `crc_input` Vec first.
- [x] **3.12 DnD `Source::send` writes the offer synchronously**
      (`automation_dnd.rs:362-372`); a stalled reader blocks the compositor. verified,
      payloads small.
      Done: `Source::send` already ran on a spawned worker thread (not the
      calloop thread) before this pass, so a stalled reader couldn't block
      the compositor; it could however wedge that worker thread forever.
      `write_offer_nonblocking` (`automation_dnd.rs`) now sets the fd
      `O_NONBLOCK` via `fcntl`, writes in a loop, and on `WouldBlock` waits
      on `poll(POLLOUT)` against a 5 s total budget, giving up with a
      `TimedOut` error (logged, same as any other write error) instead of
      blocking indefinitely.
- [x] **3.13 DnD dwell inserts a fresh timer source per step** (`state.rs:1162-1181`)
      instead of `ToDuration`. cosmetic.
      Done (re-verified rather than re-implemented): `continue_automation_dnd`
      (`state.rs:1206-1237`) already inserts exactly one `Timer::from_duration`
      and drives every subsequent dwell step by returning
      `TimeoutAction::ToDuration(DWELL)` from the same callback -- no fresh
      timer source per step. This must have landed in an earlier commit on
      this branch (726bc65/386b68b); the finding in this file was stale.

## 4. Scheme policy layer hot paths

- [x] **4.1 Status publication builds the full state twice and writes two files
      synchronously per sync** (`status.scm:196-222`). `publish-status!` is the
      `%sync-hook` (`init.scm:36`), so it runs after every `sync-frames-now!` (57
      `sync-frames!` call sites), every `add-urgent-window!` (`frames.scm:2191`) and
      every title change (`groups.scm:996`; browsers retitle constantly). Each run:
      `current-state` (`group-status-summaries` does `delete-duplicates` per group;
      `dump-layout-spec` -> `subtree-rect` calls `frame-leaves` per split), `equal?`
      fingerprint, then `current-state` **again** on change, JSON via
      `string-append`, then `atomic-write-file` x2 (open, write, chmod, rename) on the
      main thread. Idea: build once and patch `sequence`/`generated_at_ms`; cheap
      version-counter fingerprint; defer writes with `wm-run-after 0` or a calloop
      idle so bursts (group switch) coalesce; drop the legacy `/minde-status` file if
      nothing reads it (unverified). verified.
      Done (21a6278): `state-body` is built once per publish and reused for the fingerprint and both outputs; JSON goes through a string port; the two file writes are deferred and coalesced through `wm-run-after` (`%pending-body`, `%write-scheduled?`), the sequence number still advances synchronously as `tests/status-test.scm` requires; the legacy `/minde-status` line is kept because doc/ipc-eww.md documents it.
- [x] **4.2 `sync-frames-now!` re-places every window of every head on every focus
      change** (`compositor/frames.scm:2041-2110`). Every focus move, split, pull or
      rename issues `wm-place-window` for every window (hidden ones re-parked at
      -10000); Rust `WmCommand::Place` (`state.rs:635-660`) unconditionally sends a
      configure, `map_element`, geometry event and foreign-toplevel refresh. Idea:
      per-window last-placed rect in Scheme and skip when unchanged, or dedupe in
      Rust before `send_pending_configure`. verified.
      Done: both sides. Scheme: `frames.scm`'s new `%placed-rects` hash table (id -> last-placed rect) is checked before every `wm-place-window` call, so an unchanged rect skips the Rust call entirely (cleared on unmap). Rust: `place_window` (state.rs:784) now compares `configured`/`moved` and returns early with no configure, `map_element`, or foreign-toplevel refresh when the toplevel wasn't reconfigured and the location didn't change.
- [x] **4.3 `frame-leaves` (`frames.scm:555-560`) allocates via `append` and is
      walked 3+ times per key** (`active-leaves` :301, `all-window-ids` :1659,
      `frame-of-window` :1666, `head-of-window` :1671, `hidden-window-ids` :1703,
      `window-id-by-number` :675). `raise-ontop!` (:985-990) does `member` over
      `all-window-ids` inside a `for-each`. Trees are tiny today. Idea: `id -> frame`
      hash maintained in `frame-add-window!`/`take-window-out!`, or memoise per sync
      generation. verified, small-n.
      Done: memoised rather than hashed by window id (a global id -> frame table
      would need updating at every direct `set-frame-window-ids!` site across
      `frames.scm`/`groups.scm`, including whole-tree rebuilds -- too invasive for
      small-n trees). `frame-leaves` results are now cached in `%frame-leaves-cache`
      keyed by node `eq?` identity, since the leaf list only depends on tree
      topology, not window contents; the cache is cleared at the three
      `set-split-child-a!`/`set-split-child-b!` sites (`split-current-frame!`,
      `remove-split!`, `split-equally!`), the only places an existing node's
      children change in place.
- [x] **4.4 `resolve-module`/`module-variable` on every Rust call.** `rust-call`
      (`frames.scm:511-523`, duplicated in `groups.scm:430-433`), `echo` (:541, twice),
      `ui-rust-call` (`init.scm:57-60`), `set-border-color!` :225, `set-key-repeat!`
      :242, `%guile-user-var` :704, `call-if-bound` :849, `wm-run-after` :665,
      `status.scm:43,51,217`, `windows.scm:39`. `set-key-state!` (`init.scm:248-265`)
      does two per prefix keystroke. Idea: one `(minde foundation rust)` helper with a
      per-symbol variable cache. verified.
      Done: new `(minde compositor rust)` module (`rust-variable`/`rust-bound?`/`rust-call`/`rust-call-if-bound`) caches each symbol's `(guile-user)` variable object in a hash table after first lookup (a redefinition still resolves live, per the module's doc comment). `frames.scm`'s `rust-call`, `groups.scm`'s duplicate `rust-call`, `init.scm`'s `ui-rust-call`/`set-border-color!`/`set-key-repeat!`/`%guile-user-var`/`call-if-bound`/`wm-run-after`, `status.scm`'s `call-runtime-info`/`output-state`, and `windows.scm`'s `window-geometry` all now go through it instead of a fresh `resolve-module`+`module-variable` each call.
- [x] **4.5 Key path allocations**: `key-spec` builds up to two strings per key
      (`init.scm:401-408`), `dispatch-key` (:369) rebuilds `"C-g"` per key in command
      mode, and `remap-target` (`frames.scm:1101-1113`) runs `string-match` (regex
      compiled per call, per rule) on every unbound key when remap rules exist. Idea:
      precompile remap regexes in `define-remapped-keys!`. verified.
      Partial: `dispatch-key`'s command-mode `"C-g"` check no longer calls `key-spec` to build and compare a string — it now tests the modifier bitmask directly (`ctrl` set, no other notated modifier).
      Closed (remaining two): `remap-target`'s regex compilation is already
      precompiled -- `define-remapped-keys!` (`frames.scm:1109-1114`) calls
      `make-regexp` once per app-id pattern when the table is (re)defined and
      stores the compiled regex in `%remapped-keys`; `remap-target`
      (`frames.scm:1127-1137`) only ever calls `regexp-exec` on those
      already-compiled objects, never `string-match`/`make-regexp` per key.
      The idea this item asked for is already the current behaviour.
      `key-spec`'s "up to two strings" (`init.scm:397,401`) is the shift-bit
      fallback lookup (`"M-G"` vs `"M-S-G"`), and each call is already O(1):
      `key-notation` (`minde/foundation/keys.scm:23-40`) resolves the C-/M-/
      S-/s- prefix via a precomputed 16-entry vector instead of testing bits
      and concatenating per call (see that module's own comment, added for
      this exact hot path), so the second call only happens when the first
      lookup misses and only costs one more vector index plus
      `string-append`. Caching per (keysym, modifiers) in a hash table would
      trade that single cheap string-append for a hash-table lookup keyed on
      a freshly-consed pair -- no net win. Left alone.
- [x] **4.6 Assoc lists scanned linearly**: `%binding-submaps` (`init.scm:117-138`),
      `%placement-rules` with `string-contains` on every map/retitle
      (`groups.scm:947,978,985`), `%layouts`, `foundation/hooks.scm:13-27`. All
      small-n; low priority.
      Closed, small-n: all four are sized by user configuration (keybindings,
      placement rules, saved layouts, hook procedures) -- realistically single- to
      low-double-digit-length lists -- so a hash table or index would trade a
      handful of `eq?`/`string=?`/`string-contains` comparisons for bookkeeping
      overhead on every mutation with no measurable win.

## 5. Clean-up (no speed impact, or unknown)

### Rust
- [x] 5.1 Gsubr registration is ~270 lines of repeated `transmute` boilerplate
      (`mod.rs:1019-1290`, 46 calls). A macro or a `&[(name, req, opt, fnptr)]` table.
      Done: `gsubr!(f, arity)` macro + one-line `register_gsubr("name", req, opt, ..)` per primitive (the api-introspect test still parses the sites).
- [x] 5.2 `call_named_N`/`callN` ladders (`mod.rs:298-359`) and five copies of
      "lookup then `scm_call_0` under `protected_call`" (`mod.rs:338-343,1499-1546`).
      Done: replaced by `Hook::call(&[Scm])` over `scm_call_n`; the five lookup+`scm_call_0` copies are one-liners on their `Hook`.
- [x] 5.3 Scene assembly duplicated in `winit.rs:142-171`, `udev.rs:1147-1275` and
      `screencopy.rs:193+`; they already drift (winit draws overlays under the
      message, lacks layers/cursor/per-output filtering). One `scene_elements(output)`.
      Done: `handlers::screencopy::output_scene_elements` (made `pub(crate)`) is now the single scene-assembly function, taking a `border_buffers: &mut BorderBuffers` so on-screen callers keep stable element ids for incremental damage while captures pass a throwaway. `udev.rs`'s `render_surface` and (per the diff) the capture path both call it; it releases its own layer-map guard before returning so callers can open their own.
- [x] 5.4 Oversized `state.rs` functions: `apply_wm_command` :633-982,
      `MindeState::new` :350-604, `send_string` :1479-1636, `send_key` :1367-1478
      (shares a keymap-scan preamble with `send_string`, :1384-1410 vs :1489-1510),
      `advance_synthetic_input` :1250-1366.
      Partial: extracted the one genuinely shared, mechanical piece --
      `keymap_and_layout` (`state.rs`) locks the xkb context and clones out
      the keymap + active layout index, replacing the duplicated three lines
      at the top of each closure in `send_key` and `send_string`. The rest of
      each function (the single-keysym search with modifier resolution in
      `send_key`, vs. the full char -> keycode/modifiers table with AltGr
      handling in `send_string`) is not shared logic, just similar shape, so
      splitting further would mean inventing new abstractions rather than
      deduplicating.
      Done: two more mechanical extractions out of `apply_wm_command`, each
      grouping arms with no shared logic between them but no reason to keep
      inline in the top-level dispatch either: the clipboard/primary-selection
      arms (`Paste`, `SetClipboard`, `SetPrimary`) now live in
      `apply_clipboard_command`, and the message/overlay/spawn arms
      (`ClearMessage`, `AddOverlay`, `ClearOverlays`, `BorderColor`, `Spawn`)
      in `apply_message_or_spawn_command`; `apply_wm_command` matches an
      `other @ (Variant1 | Variant2 | ...)` binding and forwards the whole
      command, so behaviour (including field bindings) is unchanged. Left
      alone, as before: `MindeState::new` and `advance_synthetic_input` have
      no mechanical split (long because they enumerate fields/cases, not
      because they repeat code), and `apply_wm_command` itself is now
      shorter but still necessarily enumerates one arm per `WmCommand`
      variant -- further grouping would start trading dispatch clarity for
      line count with no remaining natural seam.
- [x] 5.5 `windows: Vec<(u64, Window)>` (`state.rs:156`) with linear
      `window_by_id`/`id_for_window`/`id_for_toplevel`; `queue_screenshot`
      (`:1012-1016`) re-implements `window_by_id`. `HashMap<u64, Window>` plus id in
      `Window::user_data()`.
      Done: `windows` is now `BTreeMap<u64, Window>` (state.rs). A
      `BTreeMap` was chosen over `HashMap` because several callers rely on a
      stable enumeration order -- the foreign-toplevel manager's initial
      `toplevel()` announcement to a newly-bound client
      (`foreign_toplevel.rs`'s `bind`) and `activate_only`'s pass over every
      window -- and since `next_window_id` only increments and ids are never
      reused, iterating a `BTreeMap` in key order is exactly the old `Vec`'s
      insertion order. `window_by_id`/`foreign_outputs_for`/
      `foreign_close_window` now do a `get(&id)` instead of a linear scan;
      `id_for_window` is O(1), reading a `WindowId` newtype stashed on
      `Window::user_data()` at `register_window` time (mirrors the existing
      `OutputId` pattern) instead of scanning for pointer equality.
      `id_for_toplevel` still scans (it matches by wl_surface identity, not
      by id, so the id map doesn't help) and `queue_screenshot` already
      called the shared `window_by_id` helper rather than reimplementing it
      -- that part of the finding was stale.
- [x] 5.6 Duplicates: `BTN_LEFT` (`input.rs:23` and `:524`); MIME list three times
      (`state.rs:955-960,968-973,1318-1323`); `(1280,720)` fallback rect three times
      (`:754-759,822-826,847-856`); ~90 lines of identical gesture pass-through in
      `move_grab.rs:421-491` vs `resize_grab.rs:738-808`; `window_for_surface` six
      times (see 3.9).
      Done: the local `const BTN_LEFT`/`BTN_RIGHT` redefinition inside `send_string` was removed in favor of the module-level consts; `state.rs` gained a `TEXT_MIME_TYPES` const and `mime_types()` helper replacing the three inline lists; the `(1280, 720)` fallback rect is now one helper (state.rs ~546-560); the gesture pass-through (relative_motion/axis/frame/all eight gesture callbacks) is one `forward_pointer_events!` macro in `grabs/mod.rs`, invoked from both `move_grab.rs` and `resize_grab.rs`; `window_for_surface` is one function (see 3.9).
- [x] 5.7 Three copies of scheme-dir resolution with different orders:
      `main.rs:229-243` (checks `CARGO_MANIFEST_DIR` first), `mod.rs:1314-1316`
      (`MINDE_SCHEME_DIR` first), `mindectl:338-358`. Unify.
      Done: `guile::scheme_dir()` (MINDE_SCHEME_DIR > repo `scheme/` > `<prefix>/share/minde/scheme`) serves both `init` and `--check-config`; `mindectl:338-358` should follow the same order (not changed here).
- [x] 5.8 `main.rs:245-263 validate_config` spawns an external `guile` although
      libguile is linked; duplicates the expression string in `mindectl:359-364`.
      Done: `guile::check_config` validates in-process through libguile (`guile::boot` + `(minde config)`), printing the condition via `print-exception`; no `guile` binary needed. `mindectl check-config` keeps its own copy of the expression.
- [x] 5.9 Legacy paths: `handle-output-geometry!` fallback and `wm-output-geometry`
      atomics (`mod.rs:90-93,189-194,1412-1420`), `mindectl subscribe --json` and the
      one-line `/minde-status` file, `wm-type` alias (`mod.rs:1214-1222`) without a
      doc pointer, REPL block guarded twice (`init.scm:1813-1836`).
      Done: kept: `wm-output-geometry`/`handle-output-geometry!` are stubbed by 12 test files and listed in the generated API docs, `subscribe --json` and `/minde-status` are documented in doc/ipc-eww.md and doc/diagnostics.md, `wm-type` in doc/api.md; the registration sites now carry doc pointers. Only the REPL double guard (`init.scm`) remains, outside this pass.
- [x] 5.10 `screencopy.rs:505-513` O(n^2) `remove(i)` loop (n tiny).
      Done: `satisfy_output_captures` now uses `pending.drain(..).partition(|capture| &capture.output == output)` (with an `iter().any()` fast-out first) instead of a manual `while` loop calling `Vec::remove(i)` per match.
- [x] 5.11 `chrono_free_timestamp` and panic-hook env probing (`main.rs:271-303`)
      duplicate `XDG_STATE_HOME` logic in `mindectl:315`.
      Done: state-dir resolution moved to `runtime_dir::state_dir()` (one Rust copy, documented as mirroring mindectl:315).

### Scheme
- [x] 5.12 `rust-call` duplicated with different missing-subr behaviour
      (`frames.scm:511` report-once vs `groups.scm:430` silent).
      Done: `groups.scm`'s local silent `rust-call` was deleted; it now uses the shared `(minde compositor rust)` module (see 4.4) via `rust-place-float!`/`rust-call-if-bound`, so both call sites share one missing-subr behavior.
- [x] 5.13 "remove id from frame and promote next" inlined five times
      (`frames.scm:695-698,786-792,1718-1721,1739-1742,1786-1789`) although
      `take-window-out!` (:1895) exists.
      Done: already resolved by an earlier pass not reflected here -- all five call
      sites (`pull-window-by-id!`, `remove-window-from-tree-in!`, `pull-hidden-next!`,
      `pull-hidden-previous!`, `move-window!`) call `take-window-out!`; no inlined
      duplicate of the remove/promote logic remains in `frames.scm`.
- [x] 5.14 `parse-key-spec` (`frames.scm:1053`) hardcodes modifier bits owned by
      `foundation/keys.scm`.
      Done: already resolved by an earlier pass not reflected here -- `parse-key-spec`
      calls `key:modifier->bit` for each prefix instead of hardcoding bit values.
- [x] 5.15 Unused or colon-only definitions in `init.scm`: `mod-symbol->bit` :93,
      `copy-unhandled-error!`, `load-module!`, `gnewbg-float-prompt!`,
      `echo-frame-windows!`; `frame-tree-window-count`/`update-output-geometry!`
      exported for tests only.
      Partial: `mod-symbol->bit` and `echo-frame-windows!` were removed from `init.scm`. `copy-unhandled-error!`, `load-module!`, `gnewbg-float-prompt!` (all bound to keys, so not truly unused) and the test-only exports `frame-tree-window-count`/`update-output-geometry!` are untouched.
      Done (remaining part): grepped the whole repo (`scheme/`, `tests/`, `doc/`, no
      `demos/` directory exists) for each name. `copy-unhandled-error!`,
      `load-module!` and `gnewbg-float-prompt!` are all bound to prefix keys in
      `init.scm`, so they are reachable, not unused. `frame-tree-window-count` and
      `update-output-geometry!` are called from nine and eight test files
      respectively (e.g. `tests/frames-test.scm`, `tests/status-test.scm`) and
      `update-output-geometry!` is also exported and part of the frozen public API
      (`doc/generated/api-catalog.scm`), so both stay exported. Nothing left to
      remove.
- [x] 5.16 `ipc_request` (`mindectl:221-257`) and the events client (`:387-400`)
      re-derive the runtime dir/socket path already computed at `:194-196` and skip
      the ownership/mode validation `runtime_dir.rs` performs.
      Done: `socket_path`/`events_socket_path` are computed once at the top of the script and passed as `--` arguments to every Guile client instead of each re-deriving `XDG_RUNTIME_DIR`/`minde-ipc.sock` inline; a shared `connect-private` helper (`client_prelude`) validates the runtime directory is a private directory of the calling uid (mirroring `runtime_dir.rs`) before connecting, used by `ipc_request`, the events subscriber and `subscribe --json`.
- [x] 5.17 `guix.scm:185-195` rewrites `guile` invocations in the scripts with
      exact-prefix regexes; the `subscribe --events` branch at `mindectl:387` is
      indented differently and is not matched, so the installed script calls
      whatever `guile` is on PATH. unverified at runtime.
      Done: `mindectl` now derives every guile invocation from one
      `GUILE=${MINDE_GUILE:-guile}` line, so `guix.scm`'s `substitute*`
      rewrites that single line
      (`GUILE=<store-guile>/bin/guile`) instead of two prefix regexes that
      missed differently-indented call sites; every `"$GUILE" ...` call site
      (including `subscribe --events`) now picks up the pinned path.
      `minde-cmd` and `minde-msg` invoke no `guile` themselves (they exec
      `mindectl`), so the same substitute* is a harmless no-op for them;
      `run-nested` invokes no `guile` either (it execs the compositor
      binary, which links libguile directly) so nothing to pin there.

### Repository hygiene
- [x] 5.18 Root-level working notes added in c441fca: `AUTOMATION-WISHLIST.md`,
      `web-form-quirks-playbook.md`, six `issue-wm-*.md`. All issue files already
      carry a "Status: implemented (2026-08-31)" section matching commits
      9f133e0..726bc65; nothing in `Makefile`, `README.md`, `doc/` or `guix.scm`
      references them and `check-doc-links` does not cover them.
      `issue-wm-drop-files.md` is superseded by
      `issue-wm-drop-files-rejected-by-dropzones.md`. Idea: move to `doc/notes/`,
      fold outcomes into `CHANGELOG.md`, promote the playbook to `doc/`.
      Done: all eight files `git mv`d to `doc/notes/`; `doc/notes/README.md`
      added as a short index describing each note and its status.
      `issue-wm-drop-files.md` was kept (not deleted): it is the original,
      broader feature-request/API design for `wm-drop-files`/`wm-drop-text`,
      while `issue-wm-drop-files-rejected-by-dropzones.md` only fixes one
      specific rejection bug and explicitly cross-references the former as
      "Ursprüngliches Feature-Issue" rather than replacing it -- it does not
      fully supersede it. `CHANGELOG.md`'s Unreleased section gained one
      `Fixed` line each for the click/paste-settle, scroll, type/modifier-
      char and drop-rejection issues, and one `Added` line for the
      screenshot primitive (previously undocumented there), each pointing
      at its `doc/notes/` file. Grepped the repo for the old root-level
      paths afterward: only self-references inside the moved files and this
      TODO entry remained (the `git mv` already updated all consumers,
      since none existed).

## 6. Build and test tooling

- [x] **6.1 Debug builds drive e2e and the nested compositor** (see 1.8).
      Done: covered by 1.8's `[profile.dev]` change (`opt-level = 1` for our
      code, `opt-level = 3` for dependencies) -- `scripts/run-nested`,
      `tests/e2e.sh` and `tests/lib/nested-compositor.sh` still build/run
      `target/debug/minde`, but that debug build is no longer unoptimized
      end to end. Verified `guix.scm`'s package build still targets the
      release profile (`cargo build --release --offline` at
      `guix.scm:118`, unaffected by the dev-profile change) and that
      `guix/cargo-config.toml` only configures the vendored source
      replacement, not profiles, so it needed no change.
- [x] **6.2 Scheme suites: 22 separate `guile` processes each reloading the module
      tree** (measured `make check-scheme` 1.2 s warm / 3.8 s cold);
      `check-generated-docs` runs six more regenerating all docs into a tmpdir on
      every `./check` (1.3 s / 2.4 s). Not a bottleneck locally; CI always pays the
      cold path. A `make compile-scheme` producing `.go` under `build/ccache` with
      `GUILE_LOAD_COMPILED_PATH` would also remove the "source newer than compiled"
      notes in test output. measured.
      Done: new `make compile-scheme` target compiles `scheme/**/*.scm` to `.go` under `build/ccache` (skipping sources not newer than their `.go`); `check-scheme`, `check-api`, `check-config`, `check-foundation`, `check-ui`, `docs` and `check-generated-docs` all depend on it and run through `$(GUILE)`, which sets `GUILE_LOAD_COMPILED_PATH=$(CCACHE):...` and `GUILE_AUTO_COMPILE=0`, so the 22+ test processes load compiled bytecode instead of each reloading and (re-)compiling the module tree.
- [x] **6.3 `check-config` and `check-keymaps` (`Makefile:72-78`) re-run
      `tests/config-test.scm` and `tests/portable-keymap-test.scm` that
      `check-scheme` already ran.** verified.
      Done: `check-config` no longer runs `tests/config-test.scm` (it now only runs `mindectl check-config`) and `check-keymaps` no longer runs `tests/portable-keymap-test.scm` (it now only runs `tests/check-portable-defaults.sh`); both scripts already run once as part of `SCHEME_TESTS` in `check-scheme`, and `doc/testing.md` documents the split.
- [x] **6.4 `nested_start` hard-codes `sleep 2` for Xvfb and 1 s polling**
      (`tests/lib/nested-compositor.sh:46,73,86`), at least 3 s fixed latency per
      scenario. Idea: poll `xdpyinfo`/socket at 100 ms. verified.
      Done: the Xvfb wait now polls `xdpyinfo -display "$NESTED_DISPLAY"`
      (falling back to checking for the `/tmp/.X11-unix/X<n>` socket when
      `xdpyinfo` is unavailable) every 100 ms up to the same 2 s budget (20
      attempts); the "scheme layer loaded" log wait and the Wayland-socket
      wait now poll every 100 ms for the same 60 s (600 attempts) and 20 s
      (200 attempts) budgets they had at 1 s granularity, so a fast start
      no longer pays the whole fixed sleep. `nested_wait_for_window_after`/
      `nested_wait_for_log_after` (lines 110/124, not named in this item)
      were left on 1 s ticks since their `$limit` parameter is a
      caller-visible seconds count used by nine other test scripts;
      changing their granularity was out of scope here.
      `guix shell -m manifest.scm -- shellcheck tests/lib/nested-compositor.sh`
      is clean.
- [x] **6.5 `./check` default runs `cargo check` only** (`check:50-51`) while
      `make check` runs build+test+clippy; fine, but the two entry points diverge.
      Documented for awareness.
      Done: `scripts/generate-testing-reference` (which generates
      `doc/testing.md`, so hand-editing the doc directly would be
      overwritten by `make docs`/`check-generated-docs`) gained a paragraph
      explaining the divergence -- `./check`'s bare run is `cargo check`
      plus the Scheme suites, `make check` additionally does a full build,
      `cargo test`, and clippy -- and recommending `make check`/
      `make check-rust` before trusting a Rust change `./check` alone has
      seen. Regenerated via `make docs`; `doc/generated/packaging.md` also
      picked up the new `lib/guile/3.0/site-ccache` install path from 1.1
      (its generator, `scripts/generate-packaging-reference`, got a
      purpose string for it).

## 7. Confirmed non-issues

- No Guile call per frame or per pointer motion (verified).
- No `thread::sleep`; key repeat, message hide, synthetic input and DnD dwell are
  calloop timers.
- No `use-modules` inside functions; runtime `eval` only in the colon prompt and
  IPC, both intentional.
- `scheme/minde/frames.scm` is a pure re-export facade; no duplicated helpers
  against `compositor/frames.scm`.
- Message and overlay `MemoryRenderBuffer` textures are cached by Smithay per
  buffer.

## 8. Lessons and constraints found while implementing

- Baseline `./check` at c441fca already failed `check-generated-docs` (stale
  `doc/generated`); regenerated in this pass.
- The frozen API hash in `release/contract.env` covers
  `doc/generated/api-reference.md`, which embeds source line numbers, so any
  edit to an exporting Scheme file changes the hash even when the binding set
  is unchanged; hash refreshed in 21a6278 after confirming no binding was
  added or removed. Consider hashing the catalog without line numbers.
- `init_udev` reaches `handle-heads-change!`/`handle-startup!` synchronously,
  so Guile must boot before the backend (1.3 is limited to moving ipc/events
  init ahead).
- No `socat`/`nc`/`inotifywait` in `manifest.scm`; `mindectl` keeps `guile`
  but now runs it with `--no-auto-compile` and, where it loads modules
  (`check-config`, the events/`--json` clients), points
  `GUILE_LOAD_COMPILED_PATH` at the packaged `site-ccache` so it loads
  compiled bytecode instead of interpreting or autocompiling on every
  invocation.
- Loading `init.scm` through the autocompiler costs a one-time ~4 s compile
  per source change; `make compile-scheme` / packaged `.go` files avoid that
  at login.
- The implementation agents hit an API rate limit mid-task on 2026-09-01;
  the tree was verified and committed afterwards, and further work proceeds
  one agent at a time.
