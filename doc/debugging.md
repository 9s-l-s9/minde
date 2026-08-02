# Debugging workflow

Start with the smallest reproducible layer and retain evidence before changing
configuration.

## Fast checks

```sh
./check
./check tests/config-test.scm
```

The default command always runs the same fixed fast gate and is also the normal
pre-commit command. Rust uses `cargo check` without final linking. Run
`./check --all` only when nested integration changed.
Toolkit/browser matrices and the soak runner remain separate to avoid
unnecessary closures and memory use; see
[`application-testing.md`](application-testing.md).

For an interactive reproduction, enter `guix shell -m manifest.scm` once and
keep `scripts/run-nested` running. Validate Scheme edits with `./check`, then
reload the live nested compositor without rebuilding it:

```sh
scripts/mindectl eval '(reload-configuration!)'
```

## Running-session evidence

```sh
mindectl query state --json | jq .
mindectl report --output /tmp/minde-report
```

Session logs are retained under `~/.local/state/minde/` as `session.log`,
`session.previous.log`, and `crash.log`. Set `MINDE_LOG_FORMAT=json` before
startup for structured tracing. Scheme binding, reload, hook and IPC failures
include Guile backtraces.

The report command conservatively redacts home paths and focused-window data,
but inspect every file before sharing it. Never add clipboard contents,
unrelated environment variables, or private command arguments manually.

## Interactive Scheme debugging

Two ways to run Scheme code against a live compositor, in order of
preference:

- **One-shot, safe**: `scripts/mindectl eval '(reload-configuration!)'`
  reads one datum, evaluates it on the compositor's event-loop thread (the
  same thread that owns policy state), and returns `(ok VALUE)` or
  `(error KEY ARGS)`. This is the supported automation interface; see
  [`ipc-eww.md`](ipc-eww.md).
- **Interactive, unsafe**: set `MINDE_UNSAFE_REPL=1` before startup.
  `scheme/init.scm` then starts a Guile REPL server
  (`(system repl server)`) on its own thread, listening on a Unix-domain
  socket at `$XDG_RUNTIME_DIR/minde-repl.sock` (or `/tmp` if
  `XDG_RUNTIME_DIR` is unset). A log line `REPL server listening at ...`
  confirms the socket path actually used. The server (`(system repl
  server)`) speaks Guile's plain-text REPL protocol directly over the
  socket -- it sends a banner and a prompt, then evaluates whatever is
  written to it -- so any tool that bridges a terminal to a Unix socket
  works as a client, for example:

  ```sh
  rlwrap nc -U "$XDG_RUNTIME_DIR/minde-repl.sock"
  # or
  socat -,raw,echo=0 UNIX-CONNECT:"$XDG_RUNTIME_DIR/minde-repl.sock"
  ```

  (verified by hand-driving `(system repl server)` over a raw Guile
  socket client: it sends the standard Guile banner and copyright text
  before its prompt, the same as a local `guile` REPL). This was not
  re-verified through Geiser or another editor-integrated REPL client in
  this pass; only the raw-socket protocol above is confirmed.

  This REPL runs on a separate thread from the compositor's event loop and
  can therefore race live policy mutation and violate the runtime's
  single-writer assumptions -- it is an explicit development escape hatch,
  never a normal login-session interface. See
  [`security.md`](security.md#unsafe-development-interface) for the full
  caveat. Prefer `mindectl eval` whenever a one-shot evaluation is
  enough.

## Rust-side debugging

- Set `RUST_BACKTRACE=1` (or `full`) before `cargo run`/`cargo build` output
  to get symbolized backtraces on panic.
- The nested (winit) backend is a normal userspace window and can be run
  under a debugger directly:

  ```sh
  guix shell -m manifest.scm -- cargo build
  guix shell -m manifest.scm -- gdb --args target/debug/minde --winit
  ```

- The DRM/TTY backend (`--tty`, driven by `debug-tty.sh`) owns the seat,
  VT, and input devices for the duration of the run. Attaching a debugger
  from the same VT is not possible; debug it either from another VT/SSH
  session with `gdb -p PID`, or reproduce the same failure under `--winit`
  first per the isolation order below, since a debugger stealing the seat
  mid-session can itself hang the console.
- Panics install a hook (`install_crash_log` in `src/main.rs`) that appends
  the panic message and a captured backtrace to
  `$XDG_STATE_HOME/minde/crash.log` (or `~/.local/state/minde/` if
  `XDG_STATE_HOME` is unset) before running the default hook, so a frozen
  TTY session still leaves evidence.

## Cold build

The vendored crate graph (`vendor/`, git-pinned Smithay and its
dependents) is large -- roughly 373 MB on disk -- so expect the first
`cargo build`/`cargo check` after a clean checkout, a `Cargo.lock` change,
or a manifest change to take a while: a measured cold fixed-gate run took
about two minutes versus 16 seconds warm on an unchanged profile, and a
cold `cargo check` took 2m10s versus 0.67s immediately repeated. `./check` uses `cargo check` rather than a full link so that
warm loops stay fast even though the initial one does not.

`mold` is available in the pinned Guix channels but
`sccache` is not, adding a third-party compiler-cache package would
complicate the default environment, and sccache cannot cache incrementally
compiled workspace crates anyway -- so neither is part of the default
toolchain. Do not add them to `manifest.scm` or the build scripts; if a
clean checkout or multiple checkouts become a measured, frequent
bottleneck, that tradeoff is worth revisiting, but it has not been made
yet.

## Editor setup

- **Rust**: rust-analyzer runs against this tree via `Cargo.toml`/`Cargo.lock`
  as usual, but expect its first index after a clean checkout to be slow
  for the same reason the first `cargo build` is: a large, git-pinned,
  vendored Smithay dependency graph (see "Cold build" above). It does not
  need special configuration beyond a working `cargo`.
- **Scheme**: Geiser (Emacs) or another Guile-aware editor client connects
  to a normal Guile 3 REPL. Load `scheme/` files with `-L scheme` on the
  path, matching how `./check` and the generator scripts invoke `guile`
  (e.g. `guile --no-auto-compile -L scheme ...`); the unsafe REPL server
  above is a separate, compositor-embedded connection point, not required
  for editing.

## Isolation order

1. Validate the declarative file without a display.
2. Reproduce with the repository default in a nested session.
3. Add the personal Scheme layer without autostart.
4. Add startup programs one at a time.
5. Only then reproduce on the DRM backend.

This ordering distinguishes compositor regressions from Home generation,
external bars/wallpaper, hardware, and client-specific behavior.

## Reporting

Include the revision, backend, Guix System revision, minimal configuration,
reproduction steps, expected/actual behavior, and the smallest relevant test
result. Follow [`../SECURITY.md`](../SECURITY.md) for vulnerabilities and
[`../SUPPORT.md`](../SUPPORT.md) for the supported pre-1.0 scope.
