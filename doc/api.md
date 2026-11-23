# Scheme API contract

The intended module boundary is below. Names use lower-case
kebab-case; predicates end in `?`, state-changing procedures end in `!`, and
compositor event entry points begin with `handle-`.

| Module | Purpose |
|---|---|
| `(minde windows)` | Curated window queries and actions |
| `(minde frames)` | Curated frame focus, splitting, layout, and interaction |
| `(minde groups)` | Workspaces, placement, and dynamic groups |
| `(minde layouts)` | Named layout registry and persistence |
| `(minde input)` | Key notation, registries, and prompts |
| `(minde commands)` | Typed command records, lookup, and invocation |
| `(minde hooks)` | Fault-isolated event hooks |
| `(minde status)` | Versioned structured state and text status for bars |

The modules under `(minde compositor ...)` are implementation details.
Rust-to-Scheme handlers are also an internal compositor boundary, not user
configuration API, with one deliberate exception: `(minde groups)` exports
the default definitions of the output-policy entry points
`output-configuration-allowed?`, `handle-output-configured!` and
`handle-output-configure-failed!` (documented so a user redefinition has a
reference to shadow), next to the output surface `output-heads` and
`configure-output!` that wrap the `wm-output-heads` / `wm-configure-output!`
primitives, and the keyboard-layout surface `keyboard-layouts`,
`set-keyboard-layout!` and the default `handle-keyboard-layout-changed!` over
`wm-keyboard-layouts` / `wm-set-keyboard-layout!`. See
[configuration.md](configuration.md#the-scheme-side) and
[configuration.md](configuration.md#keyboard-layouts).

The generated inventory in [`generated/api-reference.md`](generated/api-reference.md)
enumerates the actual Guile interfaces rather than copying this table. The
mutable frame model, Rust synchronization functions, and event adapters live
in the private `(minde compositor frames)` implementation module; the
public `(minde frames)` facade exports only configuration-facing actions.
Its `frame-api-groups` value classifies that surface for discovery and review:

| Capability group | Responsibility |
|---|---|
| `topology-and-layout` | Split, remove, resize, balance, and apply frame layouts |
| `focus-and-navigation` | Move focus among frames and windows |
| `window-placement` | Pull, move, exchange, select, and number windows |
| `window-lifecycle` | Fullscreen, close, rename, and persistent visibility state |
| `floating-windows` | Convert, position, and normalize floating windows |
| `input-and-pointer` | Synthetic keyboard, pointer, and remapping operations |
| `visual-interaction` | Overlays, expose mode, properties, and window summaries |
| `persistence-and-inspection` | Serialize and restore frame/layout state |

Every public frame operation belongs to exactly one group; the API contract
test rejects unclassified or multiply classified exports.

`frame-api-tags` adds overlapping, cross-cutting labels: `layout`, `focus`,
`window-placement`, `window-state`, `floating`, `keyboard`, `pointer`,
`visual`, `persistence`, and `inspection`. Tags may overlap, but contract tests
ensure that every tagged name is part of the curated public surface.

## Source documentation contract

Procedure descriptions live beside their definitions as Guile docstrings.
The generator parses source forms directly, including `define*` argument names,
so output is independent of Guile's compiled-module cache. A complete public
description records the signature, result, side effects, errors, one executable
example, stability, and whether the operation is visual. Command demo IDs come
from the command registry rather than from prose.

Bindings without a source description are emitted as explicit debt. Generated
documentation must never silently omit an export or invent behavior from its
name. Non-procedure values and record accessors need adjacent source metadata
or a narrower public interface before the zero-debt gate can be enabled.

## Commands

Each registered command has a canonical symbol, procedure, argument schema,
category, one-line summary, full documentation, and demo identifier. Use
`command-ref`, `command-names`, `commands-in-category`, and `invoke-command`
from `(minde commands)`. The built-in metadata is centralized in
`(minde command-catalog)` so offline validation and the live compositor use
the same definitions.

## Declarative configuration

[`scheme/default-config.scm`](../scheme/default-config.scm) contains the full
portable map as a single data expression. A minimal personal override file
inherits those bindings:

```scheme
(minde-config
 (version 1)
 (prefix (ctrl) "t")
 (bindings))
```

The loader rejects unknown fields, versions, modifiers, XKB key names and
commands, as well as duplicate bindings after normalizing modifier order and
keysym aliases. It constructs a complete candidate binding table and
publishes it only after validation succeeds. A failed live reload therefore
keeps the active prefix and bindings unchanged.

Validate without a display using either:

```sh
cargo run -- --check-config scheme/default-config.scm
scripts/mindectl check-config scheme/default-config.scm
```

These offline checks recognize bundled commands, including control commands,
and reject bindings that require arguments. Use `mindectl check-config FILE
--live` to validate against the running session's registry, including personal
commands. Checking neither loads personal init files nor applies configuration.

The public contract and registry metadata are checked with `make check-api`.

## Low-level automation primitives

The `wm-*` gsubrs are available through the compositor IPC and are listed by
`describe-api`. Pointer coordinates are zero-based global logical coordinates;
they include bars and gaps, while `window-geometry` reports the visible window
rectangle in that same coordinate system. Prefer the public
`focus-window-by-id!` operation when an automation script must switch groups or
frames: `wm-focus-window` and `wm-raise-window` only act on the currently shown
compositor objects.

`wm-session-lock-abandoned?` reports a locked session whose owner has vanished.
`wm-on-session-lock-lost` runs the zero-argument `session-lock-lost` hook once
for each vanished owner, including loss before lock confirmation. The default
`(minde session)` policy retries `%lock-command` with backoff while that predicate
is true; `%lock-restart?` disables automatic retries. See [session recovery](security.md).

`wm-send-string` and its alias `wm-type` pace characters at 20 ms by default
and accept an optional delay in milliseconds. Characters unavailable in the
active layout use a temporary client keymap, with a balanced press/release.
Native clients receive the configured keymap and modifiers again in the same
event-loop step. Xwayland restoration waits briefly for its X11 client to
consume the keystroke; focus changes, physical or virtual keyboard input,
layout changes and session locking restore the map before proceeding.
Xwayland uses at least 30 ms for each unmapped character; a larger requested
delay also applies to restoration after the final character.
Typing does not read or replace either selection and does not send a paste
shortcut. A focus change or session lock cancels the remaining text.
`wm-click` accepts
`left`/`middle`/`right`, 1/2/3, or evdev BTN_LEFT/BTN_MIDDLE/BTN_RIGHT codes;
its optional count is sequenced as one multi-click gesture.
`wm-button-press` and `wm-button-release` take the same button names and hold
or release a button in queue order with other synthetic input (a hover/settle
motion precedes the press); a held button is released when synthetic input is
cancelled. `(wm-drag x1 y1 x2 y2 [steps])` moves the pointer to the start,
presses the left button, sends `steps` paced motion events (default 10, at most
200, about 15 ms apart) and releases at the end, so toolkits that need several
motions to start a drag recognize it. The events use the normal seat path, so
native Wayland and XWayland clients behave alike. All three return `#f` for
invalid arguments.

New toplevels that are dialogs float instead of tiling. `(window-dialog? id)`
is true for a window with a parent (xdg parent or X11 `WM_TRANSIENT_FOR`), an
xdg-dialog object, or X11 type DIALOG, UTILITY or SPLASH. Such a window floats
centered over its parent when visible in the active group, otherwise over the
current frame, in the size it reports (clamped to the head), else the default
float size. `(wm-window-hints id)` returns the raw facts as
`((parent . id) (type . symbol) (size . (w . h)))`; see
`#:dialog?` in `add-placement-rule!` (doc/configuration.md) to override. `wm-set-clipboard`
sets CLIPBOARD for Ctrl+V/`wm-paste`; `wm-set-primary` sets the middle-click
PRIMARY selection. The older `wm-request-paste` asynchronously reads CLIPBOARD
into minde's active prompt. A client-owned paste completes on EOF, after
64 KiB, or after five seconds, delivering the text collected so far (possibly
empty). Clipboard, primary-selection and drag transfers share a limit of 32
queued or active transfers; excess receive requests close immediately and
excess paste requests complete with empty text. Stalled pipe transfers expire
after five seconds. Replacing a selection cancels its pending writes.

`wm-drop-files` accepts one or more absolute paths to existing regular files;
`wm-drop-text` offers UTF-8 plain text. Both return a request token or `#f` for
invalid arguments. Query `(wm-automation-status token)` for `(operation status)`
or subscribe to `(automation-result token operation status)`. Terminal statuses
are `accepted`, `rejected`, `no-target`, `cancelled`, and
`unsupported-target`. Drops are native Wayland only; XWayland requires XDND and
currently reports `unsupported-target`.
Each token publishes one terminal result. `accepted` confirms that the target
accepted the drop when the drag ended; the target may still be reading its
payload. A later offer finish or destruction cancels remaining transfers
without changing that result. Expired history entries are not recreated by
late offer cleanup.
Requests made while a pointer grab or button press is active report
`cancelled` before moving the pointer or injecting input. Existing clicks,
client drags and compositor move/resize grabs continue normally.

`(wm-screenshot path [window-id])` creates a new PNG below the directory
returned by `(wm-screenshot-directory)`. Set `MINDE_SCREENSHOT_DIR` before
starting Minde to choose another absolute directory; the default is
`minde/screenshots` inside the private runtime directory. Existing files,
symlinks and `..` traversal are rejected. The writer publishes a complete
mode-0600 file atomically and never replaces an existing entry, even if
another capture creates it after the request was accepted. Subdirectories
inside the capture directory must already exist. Invalid destinations return
`#f`; accepted requests return a token whose `(wm-automation-status token)`
becomes `(screenshot done)` or `(screenshot failed)`.

## Status

`(minde status)` exports `status-schema-version`, `current-state`,
`current-state-json`, `current-status-text`, `status-file-path`, and
`publish-status!`. `current-state` returns the schema-v1 alist; the JSON form
is used by `mindectl query state --json` and the atomic status file.
`current-state-json` accepts `#:redact? #t` for diagnostics that must omit
focused-window title and application ID. The complete schema and stability
rules are documented in [diagnostics.md](diagnostics.md).

## Accessible widgets

Use `mindectl a11y` to inspect a client window and operate its widgets through
AT-SPI, the Linux desktop accessibility API. Start with the Minde window ID
and inspect the roles and names the application actually exports. For example,
if a dialog has ID 7:

```sh
mindectl a11y tree 7 --depth 6
mindectl a11y find 7 --role push-button --name Finish
mindectl a11y select 7 --role combo-box --name 'Model time units' --value minutes
mindectl a11y set-text 7 --role text --name Capacity --value 3
mindectl a11y click 7 --role push-button --name Finish
```

The controller resolves the application's process ID and window title. A separate
helper queries the session accessibility bus, so a slow application does not
block the compositor's event loop. Selectors match exact accessible names and
roles. An ambiguous match fails before the helper acts. Selection, text edits
and button presses use the widget's own interfaces, independently of keyboard
focus. After an action opens or closes a dialog, resolve the new window ID and
inspect its tree before continuing.

Successful requests print JSON with `ok` and `result`. A tree lists role, name
and depth. Find reports window-relative bounds when the toolkit supplies them. Actions report whether the toolkit accepted the request.
Check the resulting application state when completion matters.

The session needs a working AT-SPI bus and a toolkit that exports accessible
objects. When running from a source checkout, the helper also needs Python
with PyGObject and the Atspi 2.0 introspection data. For GTK3, check that `NO_AT_BRIDGE` is unset. Custom-drawn diagram
canvases may expose only a single object. Use the compositor's screenshot and
pointer facilities for those regions. The headless GTK3 integration test and
its Guix invocation are in [tests/a11y-e2e.sh](../tests/a11y-e2e.sh).
