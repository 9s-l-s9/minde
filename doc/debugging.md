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
