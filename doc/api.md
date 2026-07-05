# Scheme API contract

Sprint 3 establishes the module boundary below. Names use lower-case
kebab-case; predicates end in `?`, state-changing procedures end in `!`, and
compositor event entry points begin with `handle-`.

| Module | Purpose |
|---|---|
| `(minde windows)` | Curated window queries and actions |
| `(minde frames)` | Frame trees, focus, splitting, and heads |
| `(minde groups)` | Workspaces, placement, and dynamic groups |
| `(minde layouts)` | Named layout registry and persistence |
| `(minde input)` | Key notation, registries, and prompts |
| `(minde commands)` | Typed command records, lookup, and invocation |
| `(minde hooks)` | Fault-isolated event hooks |
| `(minde status)` | Versioned structured state and text status for bars |

The modules under `(minde compositor ...)` are implementation details.
Rust-to-Scheme handlers are also an internal compositor boundary, not user
configuration API.

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
  ("R" reload-configuration!)))
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

## Status

`(minde status)` exports `status-schema-version`, `current-state`,
`current-state-json`, `current-status-text`, `status-file-path`, and
`publish-status!`. `current-state` returns the schema-v1 alist; the JSON form
is used by `mindectl query state --json` and the atomic status file.
`current-state-json` accepts `#:redact? #t` for diagnostics that must omit
focused-window title and application ID. The complete schema and stability
rules are documented in [diagnostics.md](diagnostics.md).
