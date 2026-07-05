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
