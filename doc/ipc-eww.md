# IPC, status, and Eww

The modeline is external. Minde publishes stable state so a bar such as Eww
can display groups, focused window, urgency, outputs, runtime information and
layout without parsing compositor logs.

## Query and subscription

```sh
mindectl query state --json
mindectl query status
mindectl subscribe --json
mindectl eval '(current-group-name)'
minde-msg -t 1500 "hello from a script"
```

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

## Published files

- `$XDG_RUNTIME_DIR/minde/status.json`: atomic schema-v1 JSON;
- `$XDG_RUNTIME_DIR/minde-status`: compatibility one-line status;
- `$XDG_RUNTIME_DIR/minde-ipc.sock`: main-thread request socket.

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
