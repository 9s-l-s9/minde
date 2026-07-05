# Diagnostics, status, and reports

Sprint 6 defines schema version 1 for external status consumers and diagnostic
reports. The public interfaces are `mindectl`, `(minde status)`, and the
atomic status file. Arbitrary `eval` remains available for trusted local
automation, but bars and monitoring tools should use `query` or `subscribe`.

## Status commands

```sh
scripts/mindectl query state --json
timeout 5 scripts/mindectl subscribe --json
```

`query` returns one current document. `subscribe` prints the current document
and then only changed documents, one compact JSON object per line. It watches
the atomically replaced status file while confirming that the compositor IPC
socket still exists.

The status file is mode `0600` at:

- `$MINDE_STATUS_PATH`, when set;
- otherwise `$XDG_RUNTIME_DIR/minde/status.json`;
- otherwise `/tmp/minde-UID/status.json`.

The historical `$XDG_RUNTIME_DIR/minde-status` one-line text file remains
available for existing eww configurations.

## State schema version 1

Every document contains:

| Field | Meaning |
|---|---|
| `schema_version` | Integer schema discriminator, currently `1` |
| `sequence` | Monotonic counter incremented only when published state changes |
| `generated_at_ms` | Unix timestamp in milliseconds |
| `runtime` | Backend, Xwayland state/display, and compositor uptime |
| `groups` | Ordered group names, focus, window counts, float/dynamic flags |
| `focused_group` | Current group name |
| `focused_window` | ID, title, app ID, and urgency, or `null` |
| `urgent_windows` | Ordered urgent window IDs |
| `outputs` | Stable ID, usable geometry, and connector name per output |
| `layout` | Manual layout spec or dynamic master position/ratio and head mode |

Adding optional fields does not change the schema version. Removing fields,
renaming fields, or changing their types requires a new version. Consumers
must reject unsupported versions instead of guessing.

Scheme callers can use `current-state`, `current-state-json`,
`current-status-text`, `status-file-path`, and `publish-status!` from
`(minde status)`. `current-state` is an alist; arrays are vectors.

## Logging and retained evidence

Human tracing is the default. Set `MINDE_LOG_FORMAT=json` before startup for
newline-delimited JSON containing `timestamp_ms`, `level`, `target`, and typed
event fields. `RUST_LOG` continues to select targets and levels in either
format.

The packaged login wrapper retains exactly two session generations:

- `$XDG_STATE_HOME/minde/session.log` (current);
- `$XDG_STATE_HOME/minde/session.previous.log` (previous).

It replaces the previous generation at the next login. Rust panics continue
to append to `crash.log`. Keybinding, configuration-reload, and IPC Scheme
errors now log a Guile backtrace before returning control to the compositor.

## Redacted report bundle

```sh
scripts/mindectl report --output /tmp/minde-report
```

The output directory must not already exist and is created mode `0700`.
It contains:

- `report.txt`: report version, minde version, kernel, redacted config path,
  seat selection, and manifest links;
- `state.json`: the redacted state view (focused ID retained; title and app ID
  omitted);
- `drm.txt`: visible DRM card and driver names;
- `recent-errors.log`: at most 200 warning/error/panic lines from the current,
  previous, and crash logs.

Recent-error collection drops lines likely to contain titles, app IDs,
clipboard data, commands, or evaluated source and replaces the home directory
with `~`. It never records clipboard contents or dumps the environment. Inspect
the bundle before sharing it because application and driver error text can
still contain information not known to minde.

## Sprint 6 owner verification

Run the local checks first:

```sh
make check-scheme check-api check-static
make check-rust
make check-e2e
```

In a nested or login session:

```sh
scripts/mindectl query state --json
timeout 5 scripts/mindectl subscribe --json
scripts/mindectl report --output /tmp/minde-report
```

Switch groups, focus windows, change a layout, and start/stop an application.
The subscription must emit changed documents with increasing sequence values.
Check `status.json` with a JSON parser, inspect every report file, and confirm
that no title, clipboard text, command argument, or absolute home path appears.
Then deliberately evaluate an error:

```sh
scripts/mindectl eval '(error "sprint-6-backtrace-check")'
```

It must fail without terminating minde, and `session.log` must contain a
Guile backtrace. After one compositor logout/login, both `session.log` and
`session.previous.log` must exist. Set `MINDE_LOG_FORMAT=json` for one
nested run and confirm every emitted tracing line is valid JSON.
