# Security model

Minde is unreleased and not yet suitable as a security boundary without
owner review. The public reporting policy is in [`../SECURITY.md`](../SECURITY.md).

## Trusted components

The compositor, loaded Scheme entry point, declarative configuration, Guix
generation, and commands launched through `wm-spawn` execute with the session
user's authority. Personal configuration is code and must be reviewed like any
other executable program.

## Wayland client trust

Every process that can connect to Minde's Wayland socket must be trusted as
a desktop component. The compositor exposes privileged Wayland interfaces
to all connected clients, without per-client permission prompts or identity
filters. Access to the separate control socket is not required for these
capabilities:

| Interface | Authority granted to a connected client |
|---|---|
| `wlr-screencopy-unstable-v1`, `ext-image-copy-capture-v1` and output capture sources | Capture output contents while the session is unlocked. |
| `wlr-data-control-unstable-v1`, `ext-data-control-v1` | Read and replace clipboard and primary-selection contents without keyboard focus. |
| `virtual-keyboard-unstable-v1`, `wlr-virtual-pointer-unstable-v1` | Inject keyboard and pointer input into the session. |
| `input-method-unstable-v2` | Become the seat's input method, receive text context and insert text for participating applications; the first active instance owns the seat. |
| `ext-session-lock-v1` | Lock the session; only the active lock object may unlock it. |
| `wlr-gamma-control-unstable-v1` | Change an output's hardware gamma ramp on the DRM backend; one controller may own each output. This interface is absent from the nested backend. |
| Foreign-toplevel, output-management and output-power interfaces | Enumerate/control windows and request output configuration or power changes. Output configuration remains subject to the configured policy predicate. |

The session-lock, input-focus, output-availability and single-owner checks
still apply to protocol requests. They do not establish per-application
permissions. Using a screen-sharing portal likewise does not restrict a
client's direct access to the capture interfaces.

Minde does not implement `wp_security_context_v1`, the protocol for attaching
sandbox identity to client connections, or filters that use such identity to
restrict these interfaces. Do not give untrusted or sandboxed applications
the compositor socket while relying on desktop isolation. For example, a
Flatpak application's `--socket=wayland` access exposes these capabilities
as well as ordinary window rendering. Supporting untrusted Wayland clients
requires security-context support and per-interface authorization first.

## Control socket

The main IPC socket is created with mode `0600` inside the user's runtime
directory. Requests are Scheme evaluation, so access to that socket is full
control of the compositor session. Do not proxy or expose it across users or
machines. Its `0600` permissions protect this IPC endpoint; the Wayland
socket has the separate trust implications described above.

## Input, clipboard, and diagnostics

The compositor necessarily observes keyboard, pointer, window metadata and
clipboard protocol traffic. Logs and reports avoid clipboard contents and
redact focused title/application identifiers when requested, but owner
inspection remains mandatory before sharing diagnostic material.

## Session locking

`ext-session-lock-v1` backs `(minde session)`'s `lock-screen!`
(`C-t s l` with the portable prefix), which spawns a configurable external locker
(default `swaylock -f`, `%lock-command`). `suspend!` (`C-t s z`) does not suspend
until the compositor confirms that locked content has replaced the desktop
(`wm-on-session-lock` / `wm-session-lock-confirmed?`), and refuses to suspend at all if that confirmation
times out (`%lock-timeout-ms`), so a suspend cannot silently wake the machine
unlocked. The DRM backend waits for a tagged locked-frame page flip on every active
output. The nested backend acknowledges a successful buffer swap into its host
window; physical presentation and locking of the surrounding desktop remain
under the host compositor's control. `wm-session-locked?` becomes true immediately
for input isolation and event redaction, before presentation confirmation.
Only the active lock object may unlock; another client can take over after the
owner dies, and the session stays locked throughout recovery. A client that
abandons an unconfirmed lock must reconnect before trying again: the pinned
protocol library retains output bindings from its abandoned lock.

If locker startup throws or timeout registration fails, `suspend!` removes
its pending confirmation hook and propagates the error. A later manual lock
cannot complete that abandoned suspend, and a new attempt can start immediately.
Timeouts left by an abandoned attempt cannot cancel a newer one.

Screen captures requested through either capture protocol or `wm-screenshot`
fail while the session is locked. Accepting a lock also fails captures still
waiting for a rendered frame, before confirming lock presentation. Those
requests cannot capture the desktop later on unlock. Existing ext capture
sessions remain usable for fresh requests after unlock.

Layer shell is not a safe substitute for this protocol: a fake lock
surface without exclusive input and correct output lifecycle would create a
false security boundary.

## Unsafe development interface

The supported IPC evaluator is serialized on the event-loop thread.
`MINDE_UNSAFE_REPL=1` opts into a separate-thread Guile REPL and may violate
runtime ownership assumptions. Never enable it in a normal login session.
The REPL requires a private, session-user-owned `XDG_RUNTIME_DIR` and binds
`minde-repl.sock` with mode `0600` before starting its server. It refuses an
unset, relative, shared, or symlinked runtime directory, and never removes
existing entries at the socket path. Failure disables only the REPL.

## Dependencies and provenance

Smithay-derived files retain MIT identifiers and provenance in `NOTICE`; project
code is GPL-3.0-or-later. Guix definitions and Cargo.lock pin the build inputs.
Release archives are offline-reproducible by construction (see doc/releasing.md).
