# IPC, status, and Eww

The modeline is external. Minde publishes stable state so a bar such as Eww
can display groups, focused window, urgency, outputs, runtime information and
layout without parsing compositor logs.

For desktop-wide snapshots, typed actions, receipts, completion waits, a
remote Scheme REPL, and sequenced observation, see
[agent control](agent-control.md). The status interface below remains useful
for bars and existing integrations.

## Query and subscription

```sh
mindectl query state --json
mindectl query status
mindectl subscribe --json
mindectl subscribe --events
mindectl eval '(current-group-name)'
minde-msg -t 1500 "hello from a script"
```

`minde-msg -t` accepts a nonnegative integer timeout in milliseconds. Invalid
timeouts or a missing message exit with usage status `2` before contacting the
compositor. Use `minde-msg -- "-t is literal message text"` when the message
starts with an option. Message text, including quotes and newlines, stays data.

`subscribe --json` re-emits `status.json` on change, using event-socket wakeups
and a two-second safety check (one-second polling if the event socket is
unavailable). `subscribe --events` streams the push events described below.

The local Unix socket is owned by the session user and accepts one Scheme datum
per request. Evaluation runs on the compositor event-loop thread, serializing
window-policy mutation with protocol events.

## Reply shape

Every reply is a single readable Scheme datum:

- `(ok RESULT)` on success;
- `(error KEY ARGS MESSAGE BACKTRACE METADATA)` on failure, where `KEY` is the Guile
  exception key (a symbol), `ARGS` its sanitized throw arguments, `MESSAGE` a
  human-readable rendering of the condition, and `BACKTRACE` a bounded Guile
  backtrace string. `MESSAGE` and `BACKTRACE` let an automated client or a human self-correct
  without a second round trip; the backtrace is capped to a few frames and a
  bounded length, and error formatting is itself guarded. The first five
  fields retain their legacy positions. `METADATA` records the failure stage
  (`validation`, `execution`, or `result-encoding`), whether execution began
  or completed, and whether effects may already have occurred. An encoding
  failure does not mean evaluation failed or changes were rolled back.

**Writable-data guarantee.** `RESULT` is always `write`-able and re-`read`-able
data. Supported values include lists/dotted pairs, vectors, bytevectors,
symbols, keywords, strings, characters, numbers, and booleans. Unspecified
values become `(unspecified)`. Strings containing `#<` are valid data.
Opaque objects and cycles are rejected by inspecting their types, and
traversal/output limits produce complete errors instead of truncated datums.
Errors such as `unreadable-result`, `cyclic-result`, and `result-too-large`
identify encoding failures. Clients must also handle missing or incomplete
transport replies, which can leave the action outcome unknown.

`mindectl --scheme`/`--machine` preserve complete envelopes; `--json` emits
schema-directed payloads and tagged arbitrary Scheme data. See the
[machine-output contract](agent-control.md#scheme-json-and-exit-codes).

## Discovering the API (`describe-api`)

An agent's first question is "what can I do here?" One IPC call answers it:

```
(describe-api)          ; the whole control surface
(describe-api "float")  ; only items whose name contains "float" (apropos)
```

`describe-api` is a top-level procedure (defined in `scheme/api-introspect.scm`,
loaded by `init.scm`), so it is callable over the eval socket without being part
of any frozen public module. It returns an alist of four sections, each a list
of per-item alists — all plain, re-readable data honoring the writable-data
guarantee above:

- `commands` — the registered command catalog: legacy fields plus typed
  parameters, result schemas, scope/effects, completion, retry, and examples;
- `procedures` — the public bindings of the documented `(minde …)`
  modules: `name`, `module`, `signature`, `documentation`;
- `gsubrs` — the `wm-*` Rust primitives: `name`, `signature`, `documentation`;
- `hooks` — the `(minde hooks)` event hooks: `name`, `arguments` (the payload
  shape), `documentation`.

Bound procedures carry signatures derived from their required, optional,
and rest arguments, plus any available procedure docstring. This also works
for re-exported procedures. Non-procedure and unbound exports remain named
entries with empty documentation.

The optional string argument is an apropos filter: only items whose name
contains it (case-insensitively) are returned, in every section at once.

The same procedure produces the machine-readable
[`doc/generated/api-catalog.scm`](generated/api-catalog.scm) (emitted by
`scripts/generate-api-catalog.scm` and checked by the doc-drift gate), so the
committed catalog and the live reply are generated from one source and cannot
drift for bundled registrations. User-defined runtime registrations naturally
extend the live catalog. `capabilities`, `search-actions`, and
`describe-action` provide more concise discovery.

## Event subscription (push)

Polling `status.json` cannot observe transitions an agent wants to react to
(a window mapping, a focus change). A second, read-only socket pushes them:

- `$XDG_RUNTIME_DIR/minde-events.sock` (mode `0600`, session user only).

Every accepted client receives each fired compositor event as one s-expression
line — the event name followed by its hook payload, matching the shapes in
`describe-api`'s `hooks` section (and `%api-hook-metadata`):

```
(new-window 42 "firefox" "org.mozilla.firefox")
(window-title-changed 42 "New title" "org.mozilla.firefox")
(window-geometry 42 (10 20 640 480))
(window-geometry-settled 42 (10 20 640 480))
(focus-window 42)
(focus-frame 0 0 1280 760)
(destroy-window 42)
```

`mindectl subscribe --events` connects and streams these lines to stdout, one
per line, until the compositor exits. The stream is read-only: it carries
events, it does not accept commands (use the eval socket for those).

Every firing is mirrored automatically at the hook layer
(`run-event-hook!` → `minde-mirror-event`, in `scheme/event-stream.scm`), so
no user-installed hook is required and every event reaches subscribers. Each
payload value honors the same **writable-data guarantee** as the eval reply: a
value that would print as `#<...>` is bounded to a string, so a subscriber's
`read` never fails on a well-formed line.

`automation-result`, `window-geometry` and `window-geometry-settled` are native
events written directly to this stream and the journal, rather than Scheme
hooks. Geometry events report committed client bounds, or `#f` when hidden or
unavailable; these can differ from a requested resize while the client
processes its configure.

During an interactive move or resize, geometry changes are combined into
one `window-geometry` event per window every 50 ms. The last pending position
is delivered even if motion stops. Release flushes the current committed
`window-geometry` immediately. A completed or cancelled grab also sends
`window-geometry-settled` with the final committed bounds. Native resizes
settle only after the client commits its final configure; X11 resizes settle
on release and may report later client commits as ordinary geometry changes.
Unmapping discards pending notifications before `destroy-window`. Other
geometry changes are published immediately.

Consumers interested only in completed drags can listen for
`window-geometry-settled`. Direct geometry queries and `wm-geometry-revision`
still reflect each committed change immediately, so action revision checks
cannot accept a stale snapshot during a drag.

**Privacy while locked.** When the session is locked (ext-session-lock),
title- and content-bearing events are filtered, mirroring `status.json`'s
`redact?` policy (window id retained; human-readable title and app-id omitted):

- `new-window` and `window-title-changed` keep their id but report empty `""` title and app-id;
- `message` events (arbitrary on-screen text) and unknown extension events
  are suppressed entirely;
- id-only lifecycle and geometry events (`focus-window`, `urgent-window`, `destroy-window`,
  `focus-frame`, `focus-group`, `window-geometry`, `window-geometry-settled`,
  `session-lock`, `session-unlock`) keep flowing,
  so an agent can still track focus and window churn while locked.

Live subscriptions and `events-since` share this policy. Replay also redacts
events recorded before locking, and filtering preserves the journal cursor.
Unknown extension events remain available on both paths while unlocked.

**Slow-consumer and eviction policy.** Delivery never blocks the compositor
event loop. Writes to a subscriber are non-blocking; unsent bytes are buffered
per subscriber up to a bounded backlog (256 KiB). A subscriber that falls
further behind is evicted — its connection is closed and the eviction is
logged. A clean client disconnect is likewise detected on the next write and
dropped. Multiple simultaneous subscribers are supported up to a fixed cap (16);
connections beyond the cap are rejected.

**Sequenced observation.** The legacy stream above keeps its payload shape.
New clients can use `mindectl watch` or `events-since` with a desktop snapshot
cursor. A separate journal retains at most 128 events and 16 KiB, records
events even without subscribers, and explicitly signals gaps requiring a
new snapshot. `watch` performs that recovery after gaps and reconnects; see
the [snapshot/journal handoff](agent-control.md#watch-changes-and-recover-from-gaps).

## Published files

- `$XDG_RUNTIME_DIR/minde/status.json`: atomic schema-v1 JSON;
- `$XDG_RUNTIME_DIR/minde-status`: compatibility one-line status;
- `$XDG_RUNTIME_DIR/minde-ipc.sock`: main-thread request socket;
- `$XDG_RUNTIME_DIR/minde-events.sock`: read-only event push socket.

The IPC helpers, `mindectl` and status writer share the private fallback
directory `${TMPDIR:-/tmp}/minde-UID` (mode `0700`) when `XDG_RUNTIME_DIR`
is unset or empty. Empty `TMPDIR` also means `/tmp`. In this fallback,
`status.json` and the compatibility `minde-status` file both live directly
inside the private directory, beside the sockets. A nonempty
`XDG_RUNTIME_DIR` takes precedence over `TMPDIR`.
Compositor startup itself requires `XDG_RUNTIME_DIR` for its Wayland
listener and exits with an error when it is unset. Startup refuses a runtime
directory owned by another user or accessible to group/other users, and never
unlinks a non-socket entry or an active compositor's socket found at a socket
path.

After a successful publication, unchanged state is not rewritten. If either
status-file write fails, the next publication retries both files even when
the desktop state is unchanged, retaining the same sequence number. Newer
state replaces the pending state and advances the sequence as usual. Failures
are logged without interrupting window management or starting a retry loop.

New consumers should use the JSON query or subscription interfaces.
Schema details and redaction guarantees are in
[`diagnostics.md`](diagnostics.md).

## Eww example

The maintained example is split into [`eww/eww.yuck`](eww/eww.yuck) and
[`eww/eww.scss`](eww/eww.scss). It should be started by personal session policy,
not by repository defaults. A typical polling source is:

```lisp
(defpoll minde_state :interval "500ms"
  `mindectl query state --json`)
```

Layer-shell exclusive zones reserve space before frame geometry is calculated,
so an Eww bar does not need to overlap managed windows.

## Unsafe REPL

`MINDE_UNSAFE_REPL=1` restores the old threaded Guile REPL for exceptional
interactive debugging. It is not an automation API: mutation from that thread
can violate compositor ownership assumptions. Use `mindectl eval` or the
client-side `mindectl repl` instead. Controlled actions reject unsupported
and reentrant execution contexts.
