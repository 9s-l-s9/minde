# Application compatibility and soak testing

Sprint 7 separates the small always-run compatibility gate from heavyweight
toolkit/browser checks. Tests never require every application in one Guix
environment.

## Resource policy

- One application scenario runs at a time.
- Every client runs in a dedicated process group with a 20-second timeout and
  forced cleanup.
- Each run receives an isolated temporary home, cache, config, state and
  Wayland runtime directory. The directory is removed when the compositor
  stops.
- Client lifetime bounds log growth. The fixed scenario inventory creates at
  most one log and one screenshot per application, overwriting the same paths
  on the next run.
- `make check-all` runs only foot, wl-clipboard and xterm. Browsers, Electron,
  Firefox, Emacs, toolkit demos and layer-shell programs are separate targets.
- The soak runner records compositor RSS on every iteration and fails after
  256 MiB of growth by default (`SOAK_MAX_RSS_GROWTH_KIB` overrides it).

The Guix store downloads a package only when its specific batch is requested.
Do not assemble a monolithic application manifest.

## Scenario inventory

| Scenario | Surface | Default gate | Verification |
|---|---|---:|---|
| foot | native Wayland | required | managed toplevel appears |
| wl-clipboard | data device | required | copy/paste MIME round trip |
| xterm | Xwayland | required | managed X11 toplevel appears |
| GTK 3/4 | native Wayland | optional | toolkit demo maps |
| Qt 5/6 | native Wayland | optional | matching-major QML window maps |
| Electron, Chromium | native Wayland | optional | Ozone/Wayland window maps |
| Firefox | native Wayland | optional | isolated-profile window maps |
| Emacs PGTK | native Wayland | optional | clean PGTK frame maps |
| SDL2/LÖVE | native Wayland | optional | SDL Wayland window maps |
| swaybg | layer shell | optional | background namespace maps |
| fuzzel | layer shell | optional | launcher namespace maps |
| swaylock | session lock | manual/missing | reports the current protocol gap |
| eww | layer shell | optional | bar namespace maps |

Normal runs record unavailable optional scenarios as `skip`. Set
`MINDE_APPS_STRICT=1` to turn a selected missing scenario into a failure.
Machine-readable results are written to
`/tmp/minde-applications/results.json`; JSONL, per-client logs and bounded
screenshots remain beside it after failures.

## Small local batches

Core release gate:

```sh
make check-apps-core
```

Run toolkits separately so Qt and GTK closures are not combined:

```sh
MINDE_APPS_FILTER=gtk3 MINDE_APPS_STRICT=1 \
  guix shell -m manifest.scm xorg-server imagemagick jq util-linux \
  gtk+:bin -- sh tests/applications.sh

MINDE_APPS_FILTER=qt6 MINDE_APPS_STRICT=1 \
  guix shell -m manifest.scm xorg-server imagemagick jq util-linux \
  qtdeclarative qtwayland -- sh tests/applications.sh
```

Qt 5 uses the same command with `qt5`, `qtdeclarative@5` and `qtwayland@5`.
Other useful single-package filters are:

- `sdl2` with `love`
- `electron` with `electron`
- `chromium` with `ungoogled-chromium`
- `firefox` with `firefox`
- `emacs-pgtk` with `emacs-pgtk`

The layer-shell batch remains relatively small:

```sh
guix shell -m manifest.scm xorg-server imagemagick jq util-linux \
  swaybg fuzzel swaylock eww -- make check-apps-layer
```

## Portable and historical keymaps

`make check-e2e` runs two nested sessions. The broad behavior suite retains
the historical test map so it can reach old commands; a second reduced-map
session exercises contextual `?`, direct `w 0`/`f 0` numbering, and terminal
launch through the actual input path. Neither session reads the host's
personal keymap or startup configuration.

## Soak runner

A quick deterministic smoke run:

```sh
make check-soak SOAK_ITERATIONS=3
```

The release-duration run is:

```sh
make check-soak SOAK_MINUTES=60
```

Each iteration maps/unmaps a client, reloads configuration, creates/deletes a
group, changes the clipboard, simulates a two-output hotplug and restores the
single output, then validates schema-v1 state. Results and RSS measurements
are retained in `/tmp/minde-soak/results.json`.

Java, Wine, games, touch/tablet, IME, drag-and-drop and multi-GPU remain
manual/optional coverage. Modern swaylock requires `ext-session-lock-v1`,
which is not implemented; normal runs record an explicit skip and strict runs
fail. Actual Xwayland server crash/restart is also a manual limitation; client
churn is automated, but the compositor does not yet restart a post-startup
Xwayland process.
