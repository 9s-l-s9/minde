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
configuration API.

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

`scheme/default-config.scm` is a single data expression, not executable code:

```scheme
(minde-config
 (version 1)
 (prefix () "Print")
 (bindings
  ("g" switch-to-next-group!)
  ("z" reload-configuration!)))
```

The loader rejects unknown fields, versions, modifiers and commands, as well
as duplicate bindings. It constructs a complete candidate binding table and
publishes it only after validation succeeds. A failed live reload therefore
keeps the active prefix and bindings unchanged.

Validate without a display using either:

```sh
cargo run -- --check-config scheme/default-config.scm
scripts/mindectl check-config scheme/default-config.scm
```

The public contract and registry metadata are checked with `make check-api`.

## Low-level automation primitives

The `wm-*` gsubrs are available through the compositor IPC and are listed by
`describe-api`. Pointer coordinates are zero-based global logical coordinates;
they include bars and gaps, while `window-geometry` reports the visible window
rectangle in that same coordinate system. Prefer the public
`focus-window-by-id!` operation when an automation script must switch groups or
frames: `wm-focus-window` and `wm-raise-window` only act on the currently shown
compositor objects.

`wm-send-string` and its alias `wm-type` pace characters at 20 ms by default
and accept an optional delay in milliseconds. `wm-click` accepts
`left`/`middle`/`right`, 1/2/3, or evdev BTN_LEFT/BTN_MIDDLE/BTN_RIGHT codes;
its optional count is sequenced as one multi-click gesture. `wm-set-clipboard`
sets CLIPBOARD for Ctrl+V/`wm-paste`; `wm-set-primary` sets the middle-click
PRIMARY selection. The older `wm-request-paste` asynchronously reads CLIPBOARD
into minde's active prompt.

`wm-drop-files` accepts one or more absolute paths to existing regular files;
`wm-drop-text` offers UTF-8 plain text. Both return a request token or `#f` for
invalid arguments. Query `(wm-automation-status token)` for `(operation status)`
or subscribe to `(automation-result token operation status)`. Terminal statuses
are `accepted`, `rejected`, `no-target`, `cancelled`, and
`unsupported-target`. Drops are native Wayland only; XWayland requires XDND and
currently reports `unsupported-target`.

## Status

`(minde status)` exports `status-schema-version`, `current-state`,
`current-state-json`, `current-status-text`, `status-file-path`, and
`publish-status!`. `current-state` returns the schema-v1 alist; the JSON form
is used by `mindectl query state --json` and the atomic status file.
`current-state-json` accepts `#:redact? #t` for diagnostics that must omit
focused-window title and application ID. The complete schema and stability
rules are documented in [diagnostics.md](diagnostics.md).
