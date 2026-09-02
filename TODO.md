# TODO: speed-up and clean-up ideas

Ideas only; nothing here has been implemented yet.  Findings come from a
read-only audit on 2026-09-01.  "verified" means the behaviour was confirmed
from the code; "unverified" means the cost still needs profiling before it is
worth acting on.  Line numbers refer to commit c441fca.

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

- [ ] **1.1 No `.go` bytecode is produced at build/package time.** `guix.scm:140-143`
      only copies `scheme/` into the store; no `guild compile` anywhere. The session
      wrapper (`guix.scm:158-161`) never exports `GUILE_LOAD_COMPILED_PATH`. Every
      packaged login autocompiles the module tree into `~/.cache/guile/ccache` on the
      compositor main thread before the first output is usable. Idea: add a compile
      phase installing to `lib/guile/3.0/site-ccache/`, export
      `GUILE_LOAD_COMPILED_PATH` in the wrapper and in `scripts/run-nested`, and set
      `GUILE_AUTO_COMPILE=0` there so a store mtime mismatch never recompiles. Also a
      `make compile-scheme` target for dev/test loops (see 6.2). verified.
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
- [ ] **1.4 `(system vm trace)` imported in `init.scm:9` but unused** (only
      `backtrace` is used). Pulls VM instrumentation modules at boot. Remove. verified.
- [ ] **1.5 Boot-time `reload-configuration!` (`init.scm:1763`) echoes a message and
      publishes status before any output exists** (`init.scm:1717`).
      `refresh-command-help!` (`init.scm:1744-1761`) is O(bindings x commands) and
      `command-names` (`commands.scm:83-85`) re-sorts the registry on each iteration.
      Idea: build a procedure->name table once; skip `echo` when no output is mapped.
      verified, small.
- [ ] **1.6 `publish-status!` runs several times during startup** via Xwayland status
      transitions (`state.rs:1875-1961`) and every `sync-frames!`. See 4.1.
- [ ] **1.7 `load-layouts!` / `load-placement-rules!` (`init.scm:918,1336`) read
      `~/.config/minde/*.scm` synchronously before output geometry is known.** Could
      move to `handle-startup!`. low priority.
- [ ] **1.8 Debug builds everywhere.** No `[profile]` section in `Cargo.toml`;
      `scripts/run-nested:70,83`, `tests/e2e.sh:57`, `tests/lib/nested-compositor.sh:58`
      all run `target/debug/minde`. A GL compositor in a debug build is noticeably
      slower to start and to render. Idea: `[profile.dev] opt-level = 1` (or a
      dedicated `e2e` profile), and `[profile.release]` with `lto = "thin"`,
      `codegen-units = 1`, `panic = "abort"` considered for the package. unverified.

## 2. IPC / REPL interaction latency

- [ ] **2.1 `scripts/mindectl:218-258 ipc_request` spawns a fresh `guile -q -c` per
      request.** `minde-msg` -> `minde-cmd` -> `mindectl` is three shell execs plus a
      Guile boot per call. The documented Eww pattern (`doc/ipc-eww.md:844-845`) polls
      `query state --json` every 500 ms, so a bar costs a Guile boot twice a second
      plus a `current-state-json` eval on the compositor thread. Idea: a tiny
      Rust/C `mindectl` (or `socat`/`nc -U` in the script for raw mode); point Eww at
      `status.json` or the events socket instead of polling eval. verified.
- [ ] **2.2 `mindectl:409-416 subscribe --json` polls with `cat` + `sleep 0.2`**,
      five forks per second forever. Idea: `inotifywait -e moved_to` or the events
      socket. verified.
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
- [ ] **2.5 One connection per request with SHUT_WR framing** (`ipc.rs:585-589`):
      every request costs connect+accept+source registration+close. A persistent
      newline- or length-framed connection would help agents and bars. unverified
      relative to 2.1, which dominates today.
- [ ] **2.6 The 250 ms deadline (`ipc.rs:621`) covers evaluation and write time**;
      a large `(describe-api)` reply to a slow reader is dropped mid-write. documented
      as intended, flagged only.
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
- [ ] **2.11 `current-state-json` (`status.scm:120-178`) is a hand-rolled JSON writer
      using recursive `string-append`/`string-join`.** Idea: write to a string port.
      unverified cost.

## 3. Frame smoothness and input latency (Rust)

- [ ] **3.1 Fixed 16 ms timer chain instead of vblank/damage-driven repaint**
      (`udev.rs:943-950`, `1038-1046`). After each vblank a new 16 ms `Timer` is
      inserted, then `render_now`; if nothing was queued another 16 ms timer follows.
      Consequences: on 60 Hz frames alternate between just-made and just-missed; on
      120/144 Hz the loop is capped near 60 fps; input arriving just after a render
      waits up to 16 ms plus a scanout; the element list is rebuilt every 16 ms at
      idle. Idea: render on vblank when a per-output `redraw_needed` flag is set,
      stop the chain at idle and restart from the dirtying event; optionally delay
      after vblank by an estimated render time (niri/sway style). verified.
- [ ] **3.2 Cursor motion does not mark the output dirty** (`input.rs:414-429`); the
      cursor element (`udev.rs:1149-1160`) is sampled at the next timer tick, so
      cursor latency inherits 3.1. Idea: motion sets the dirty flag; make sure the
      cursor lands on the cursor plane. verified.
- [ ] **3.3 winit backend renders unconditionally with full-frame damage**
      (`winit.rs:133,191,248`). Nested only, low priority. verified.
- [ ] **3.4 Per-frame allocations and locks**: `custom`/`all_elements` Vecs and layer
      `partition` (`udev.rs:1147,1201-1207,1245,1268`), `cursor_state.hotspot()` mutex
      per frame (`render.rs:519-531`), O(windows x outputs) frame-callback walk
      (`udev.rs:1367-1389`). Idea: reusable Vecs on the output surface. verified,
      small.
- [ ] **3.5 `surface_under` evaluated three times per pointer motion**:
      `constrain_pointer` (`pointer_constraints.rs:79`), `input.rs:396`, `input.rs:417`.
      Each does an output lookup, a layer-map RefCell borrow and a surface-tree hit
      test (`state.rs:2067-2116`). At 1000 Hz mice this triples the dominant
      per-motion cost. Idea: compute once, pass along. verified.
- [ ] **3.6 Resize grab sends a configure on every motion event**
      (`resize_grab.rs:620-654`), uncoalesced. Idea: pending size flushed once per
      frame. Move grab does `space.map_element(.., activate=true)` per motion
      (`move_grab.rs:369`). verified; move cost unverified.
- [ ] **3.7 Every click and focus change walks all windows calling
      `send_pending_configure`** (`input.rs:586-597,648-652`, `state.rs:718-747`).
      Smithay elides unchanged configures, so cheap; flagged for the pattern.
- [ ] **3.8 `fontdue::Font::from_bytes` re-parses the embedded TTF on every message
      and once per overlay** (`render.rs:133`, from `state.rs:760,783`), and
      rasterises every glyph fresh. Idea: `OnceLock<Font>`, glyph cache keyed by
      char, cache the rendered overlay per label string. verified; cost unverified.
- [ ] **3.9 Four independent Space scans per `wl_surface.commit`**:
      `compositor.rs:39-48`, `xdg_shell.rs:463-486` (only meaningful on first
      commit), `resize_grab.rs:872-882` (only while resizing),
      `handle_layer_commit` (`compositor.rs:76-87`). Plus `report_title_if_changed`
      (`state.rs:1804-1831`) clones title and app_id Strings on every commit just to
      compare. Idea: one `window_for_surface` helper (replaces six duplicates), gate
      the first-commit and resize checks on flags, compare titles by `&str` or hook
      `set_title` requests. verified; absolute cost unverified.
- [ ] **3.10 Layer-surface commits run `arrange()` twice and rebuild head info**
      (`compositor.rs:101` then `update_usable_area` `state.rs:1981-2014`), allocating
      output names and calling `refresh_foreign_toplevel_outputs`
      (`foreign_toplevel.rs:221-259`, O(windows x outputs) with ~4 Vecs per window).
      A bar redrawing at 1 Hz pays this every second. Idea: arrange only when the
      layer's cached state changed; diff foreign outputs only when geometry moved.
      verified.
- [ ] **3.11 Screenshot encode + write inline on the calloop thread**
      (`screencopy.rs:327-358` -> `png.rs:1154-1167,1214-1248`): bitwise table-less
      CRC-32, three full-size copies, synchronous file write inside the redraw
      callback. Idea: table CRC or `crc32fast`; encode and write on a worker after
      GPU readback. verified; expect tens of ms at 4K, unverified.
- [ ] **3.12 DnD `Source::send` writes the offer synchronously**
      (`automation_dnd.rs:362-372`); a stalled reader blocks the compositor. verified,
      payloads small.
- [ ] **3.13 DnD dwell inserts a fresh timer source per step** (`state.rs:1162-1181`)
      instead of `ToDuration`. cosmetic.

## 4. Scheme policy layer hot paths

- [ ] **4.1 Status publication builds the full state twice and writes two files
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
- [ ] **4.2 `sync-frames-now!` re-places every window of every head on every focus
      change** (`compositor/frames.scm:2041-2110`). Every focus move, split, pull or
      rename issues `wm-place-window` for every window (hidden ones re-parked at
      -10000); Rust `WmCommand::Place` (`state.rs:635-660`) unconditionally sends a
      configure, `map_element`, geometry event and foreign-toplevel refresh. Idea:
      per-window last-placed rect in Scheme and skip when unchanged, or dedupe in
      Rust before `send_pending_configure`. verified.
- [ ] **4.3 `frame-leaves` (`frames.scm:555-560`) allocates via `append` and is
      walked 3+ times per key** (`active-leaves` :301, `all-window-ids` :1659,
      `frame-of-window` :1666, `head-of-window` :1671, `hidden-window-ids` :1703,
      `window-id-by-number` :675). `raise-ontop!` (:985-990) does `member` over
      `all-window-ids` inside a `for-each`. Trees are tiny today. Idea: `id -> frame`
      hash maintained in `frame-add-window!`/`take-window-out!`, or memoise per sync
      generation. verified, small-n.
- [ ] **4.4 `resolve-module`/`module-variable` on every Rust call.** `rust-call`
      (`frames.scm:511-523`, duplicated in `groups.scm:430-433`), `echo` (:541, twice),
      `ui-rust-call` (`init.scm:57-60`), `set-border-color!` :225, `set-key-repeat!`
      :242, `%guile-user-var` :704, `call-if-bound` :849, `wm-run-after` :665,
      `status.scm:43,51,217`, `windows.scm:39`. `set-key-state!` (`init.scm:248-265`)
      does two per prefix keystroke. Idea: one `(minde foundation rust)` helper with a
      per-symbol variable cache. verified.
- [ ] **4.5 Key path allocations**: `key-spec` builds up to two strings per key
      (`init.scm:401-408`), `dispatch-key` (:369) rebuilds `"C-g"` per key in command
      mode, and `remap-target` (`frames.scm:1101-1113`) runs `string-match` (regex
      compiled per call, per rule) on every unbound key when remap rules exist. Idea:
      precompile remap regexes in `define-remapped-keys!`. verified.
- [ ] **4.6 Assoc lists scanned linearly**: `%binding-submaps` (`init.scm:117-138`),
      `%placement-rules` with `string-contains` on every map/retitle
      (`groups.scm:947,978,985`), `%layouts`, `foundation/hooks.scm:13-27`. All
      small-n; low priority.

## 5. Clean-up (no speed impact, or unknown)

### Rust
- [x] 5.1 Gsubr registration is ~270 lines of repeated `transmute` boilerplate
      (`mod.rs:1019-1290`, 46 calls). A macro or a `&[(name, req, opt, fnptr)]` table.
      Done: `gsubr!(f, arity)` macro + one-line `register_gsubr("name", req, opt, ..)` per primitive (the api-introspect test still parses the sites).
- [x] 5.2 `call_named_N`/`callN` ladders (`mod.rs:298-359`) and five copies of
      "lookup then `scm_call_0` under `protected_call`" (`mod.rs:338-343,1499-1546`).
      Done: replaced by `Hook::call(&[Scm])` over `scm_call_n`; the five lookup+`scm_call_0` copies are one-liners on their `Hook`.
- [ ] 5.3 Scene assembly duplicated in `winit.rs:142-171`, `udev.rs:1147-1275` and
      `screencopy.rs:193+`; they already drift (winit draws overlays under the
      message, lacks layers/cursor/per-output filtering). One `scene_elements(output)`.
- [ ] 5.4 Oversized `state.rs` functions: `apply_wm_command` :633-982,
      `MindeState::new` :350-604, `send_string` :1479-1636, `send_key` :1367-1478
      (shares a keymap-scan preamble with `send_string`, :1384-1410 vs :1489-1510),
      `advance_synthetic_input` :1250-1366.
- [ ] 5.5 `windows: Vec<(u64, Window)>` (`state.rs:156`) with linear
      `window_by_id`/`id_for_window`/`id_for_toplevel`; `queue_screenshot`
      (`:1012-1016`) re-implements `window_by_id`. `HashMap<u64, Window>` plus id in
      `Window::user_data()`.
- [ ] 5.6 Duplicates: `BTN_LEFT` (`input.rs:23` and `:524`); MIME list three times
      (`state.rs:955-960,968-973,1318-1323`); `(1280,720)` fallback rect three times
      (`:754-759,822-826,847-856`); ~90 lines of identical gesture pass-through in
      `move_grab.rs:421-491` vs `resize_grab.rs:738-808`; `window_for_surface` six
      times (see 3.9).
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
- [ ] 5.10 `screencopy.rs:505-513` O(n^2) `remove(i)` loop (n tiny).
- [x] 5.11 `chrono_free_timestamp` and panic-hook env probing (`main.rs:271-303`)
      duplicate `XDG_STATE_HOME` logic in `mindectl:315`.
      Done: state-dir resolution moved to `runtime_dir::state_dir()` (one Rust copy, documented as mirroring mindectl:315).

### Scheme
- [ ] 5.12 `rust-call` duplicated with different missing-subr behaviour
      (`frames.scm:511` report-once vs `groups.scm:430` silent).
- [ ] 5.13 "remove id from frame and promote next" inlined five times
      (`frames.scm:695-698,786-792,1718-1721,1739-1742,1786-1789`) although
      `take-window-out!` (:1895) exists.
- [ ] 5.14 `parse-key-spec` (`frames.scm:1053`) hardcodes modifier bits owned by
      `foundation/keys.scm`.
- [ ] 5.15 Unused or colon-only definitions in `init.scm`: `mod-symbol->bit` :93,
      `copy-unhandled-error!`, `load-module!`, `gnewbg-float-prompt!`,
      `echo-frame-windows!`; `frame-tree-window-count`/`update-output-geometry!`
      exported for tests only.
- [ ] 5.16 `ipc_request` (`mindectl:221-257`) and the events client (`:387-400`)
      re-derive the runtime dir/socket path already computed at `:194-196` and skip
      the ownership/mode validation `runtime_dir.rs` performs.
- [ ] 5.17 `guix.scm:185-195` rewrites `guile` invocations in the scripts with
      exact-prefix regexes; the `subscribe --events` branch at `mindectl:387` is
      indented differently and is not matched, so the installed script calls
      whatever `guile` is on PATH. unverified at runtime.

### Repository hygiene
- [ ] 5.18 Root-level working notes added in c441fca: `AUTOMATION-WISHLIST.md`,
      `web-form-quirks-playbook.md`, six `issue-wm-*.md`. All issue files already
      carry a "Status: implemented (2026-08-31)" section matching commits
      9f133e0..726bc65; nothing in `Makefile`, `README.md`, `doc/` or `guix.scm`
      references them and `check-doc-links` does not cover them.
      `issue-wm-drop-files.md` is superseded by
      `issue-wm-drop-files-rejected-by-dropzones.md`. Idea: move to `doc/notes/`,
      fold outcomes into `CHANGELOG.md`, promote the playbook to `doc/`.

## 6. Build and test tooling

- [ ] **6.1 Debug builds drive e2e and the nested compositor** (see 1.8).
- [ ] **6.2 Scheme suites: 22 separate `guile` processes each reloading the module
      tree** (measured `make check-scheme` 1.2 s warm / 3.8 s cold);
      `check-generated-docs` runs six more regenerating all docs into a tmpdir on
      every `./check` (1.3 s / 2.4 s). Not a bottleneck locally; CI always pays the
      cold path. A `make compile-scheme` producing `.go` under `build/ccache` with
      `GUILE_LOAD_COMPILED_PATH` would also remove the "source newer than compiled"
      notes in test output. measured.
- [ ] **6.3 `check-config` and `check-keymaps` (`Makefile:72-78`) re-run
      `tests/config-test.scm` and `tests/portable-keymap-test.scm` that
      `check-scheme` already ran.** verified.
- [ ] **6.4 `nested_start` hard-codes `sleep 2` for Xvfb and 1 s polling**
      (`tests/lib/nested-compositor.sh:46,73,86`), at least 3 s fixed latency per
      scenario. Idea: poll `xdpyinfo`/socket at 100 ms. verified.
- [ ] **6.5 `./check` default runs `cargo check` only** (`check:50-51`) while
      `make check` runs build+test+clippy; fine, but the two entry points diverge.
      Documented for awareness.

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
