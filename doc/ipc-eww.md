# IPC, status, and Eww

The modeline is external. Minde publishes stable state so a bar such as Eww
can display groups, focused window, urgency, outputs, runtime information and
layout without parsing compositor logs.

## Query and subscription

```sh
mindectl query state --json
mindectl query status
mindectl subscribe --json
mindectl subscribe --events
mindectl eval '(current-group-name)'
minde-msg -t 1500 "hello from a script"
```

`subscribe --json` re-emits `status.json` on change by polling; `subscribe
--events` is the real-time push interface described below.

The local Unix socket is owned by the session user and accepts one Scheme datum
per request. Evaluation runs on the compositor event-loop thread, serializing
window-policy mutation with protocol events.

## Reply shape

Every reply is a single readable Scheme datum:

- `(ok RESULT)` on success;
- `(error KEY ARGS MESSAGE BACKTRACE)` on failure, where `KEY` is the Guile
  exception key (a symbol), `ARGS` its raw throw arguments, `MESSAGE` a
  human-readable rendering of the condition, and `BACKTRACE` a bounded Guile
  backtrace string. `MESSAGE` and `BACKTRACE` let an LLM or a human self-correct
  without a second round trip; the backtrace is capped to a few frames and a
  bounded length, and error formatting is itself guarded so it never throws.

**Writable-data guarantee.** `RESULT` is always `write`-able and re-`read`-able
data. Public catalog commands return plain data (lists, symbols, strings,
numbers, booleans), never opaque records, procedures, or other objects that
print as `#<...>`. The IPC reply path enforces this: an unreadable result is
converted into an `(error unreadable-result ...)` datum rather than emitted as
an unparseable reply, so a client's `read` never fails on a well-formed
response.

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

- `commands` — the registered command catalog: `name`, `category`, `summary`,
  `arguments`, `documentation`;
- `procedures` — the public bindings of the eight documented `(minde …)`
  modules: `name`, `module`, `signature`, `documentation`;
- `gsubrs` — the `wm-*` Rust primitives: `name`, `signature`, `documentation`;
- `hooks` — the `(minde hooks)` event hooks: `name`, `arguments` (the payload
  shape), `documentation`.

The optional string argument is an apropos filter: only items whose name
contains it (case-insensitively) are returned, in every section at once.

The same procedure produces the machine-readable
[`doc/generated/api-catalog.scm`](generated/api-catalog.scm) (emitted by
`scripts/generate-api-catalog.scm` and checked by the doc-drift gate), so the
committed catalog and the live reply are generated from one source and cannot
drift.

## Event subscription (push)

Polling `status.json` cannot observe transitions an agent wants to react to
(a window mapping, a focus change). A second, read-only socket pushes them:

- `$XDG_RUNTIME_DIR/minde-events.sock` (mode `0600`, session user only).

Every accepted client receives each fired compositor event as one s-expression
line — the event name followed by its hook payload, matching the shapes in
`describe-api`'s `hooks` section (and `%api-hook-metadata`):

```
(new-window 42 "firefox" "org.mozilla.firefox")
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

**Privacy while locked.** When the session is locked (ext-session-lock),
title- and content-bearing events are filtered, mirroring `status.json`'s
`redact?` policy (window id retained; human-readable title and app-id omitted):

- `new-window` keeps its id but reports empty `""` title and app-id;
- `message` events (arbitrary on-screen text) are suppressed entirely;
- id-only lifecycle and geometry events (`focus-window`, `destroy-window`,
  `focus-frame`, `focus-group`, `session-lock`, `session-unlock`) keep flowing,
  so an agent can still track focus and window churn while locked.

**Slow-consumer and eviction policy.** Delivery never blocks the compositor
event loop. Writes to a subscriber are non-blocking; unsent bytes are buffered
per subscriber up to a bounded backlog (256 KiB). A subscriber that falls
further behind is evicted — its connection is closed and the eviction is
logged. A clean client disconnect is likewise detected on the next write and
dropped. Multiple simultaneous subscribers are supported up to a fixed cap (16);
connections beyond the cap are rejected.

## Published files

- `$XDG_RUNTIME_DIR/minde/status.json`: atomic schema-v1 JSON;
- `$XDG_RUNTIME_DIR/minde-status`: compatibility one-line status;
- `$XDG_RUNTIME_DIR/minde-ipc.sock`: main-thread request socket;
- `$XDG_RUNTIME_DIR/minde-events.sock`: read-only event push socket.

If `XDG_RUNTIME_DIR` is unavailable, both sockets use the private fallback
directory `/tmp/minde-UID` (mode `0700`). Startup refuses a runtime
directory owned by another user or accessible to group/other users, and never
unlinks a non-socket entry or an active compositor's socket found at a socket
path.

Unchanged state is not rewritten. New consumers should use the JSON query or
subscription interfaces. Schema details and redaction guarantees are in
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
can violate compositor ownership assumptions. Use `mindectl eval` instead.
