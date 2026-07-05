# Scripted API demonstrations

Minde demonstrations are generated from version-controlled scenarios; no
clip requires manual recording or editing. The command registry supplies the
demo IDs, [`demos/scenarios.scm`](../demos/scenarios.scm) supplies deterministic
setup/action/cleanup expressions, and `scripts/capture-demos` performs the
capture in one isolated nested compositor session.

## Resource policy

- Capture is opt-in: `make check-docs` never starts a compositor or encoder.
- One compositor, two foot windows and one ffmpeg encoder are active.
- The sixteen command clips are encoded sequentially with two encoder threads.
- Each clip defaults to three seconds at 1280×800 and 15 frames per second.
- Videos and posters are build artifacts under `build/demos`; they are not
  committed or added to the Guix package source.
- `MINDE_DEMO_DURATION`, `MINDE_DEMO_DISPLAY` and
  `MINDE_DEMO_OUT` may override the bounded defaults.

## Generate and validate

```sh
guix shell -m manifest.scm xorg-server xdotool imagemagick foot ffmpeg jq \
  util-linux -- make demos check-demos
```

`make demos` regenerates the manifest, starts an isolated Xvfb/winit session,
maps two known windows and processes every scenario. For each demo ID it
creates:

- `ID.webm`: VP9 video;
- `ID.png`: fallback poster frame;
- `ID.txt`: setup, action, cleanup and IPC responses;
- `manifest.json`: the generated command-to-artifact index.

Commands with portable bindings are driven through synthetic X input against
the nested compositor window. Commands without a direct portable sequence use
the serialized main-thread IPC path and are labelled `command:` in the clip.

`make check-demos` rejects a stale manifest, missing/empty artifact or a video
without a positive duration. The fast `make check-docs` gate independently
checks that the committed manifest and generated references match their
Scheme sources.

## Coverage model

Every registered command has exactly one demo ID and every demo ID has exactly
one scenario. Manifest generation fails on missing, duplicate or stale IDs.
Public bindings without a command scenario remain in the generated API
inventory with the explicit classification `non-visual`; their behavior is
covered by the focused Scheme suites rather than by meaningless UI clips. The
live boundary is fully described, while its pre-1.0 curation debt remains
recorded in `UNEXPECTED.md`.
