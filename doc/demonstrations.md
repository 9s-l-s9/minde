# Scripted API demonstrations

Minde demonstrations are generated from version-controlled scenarios; no
clip requires manual recording or editing. The command registry supplies the
demo IDs, [`demos/scenarios.scm`](../demos/scenarios.scm) supplies deterministic
setup/action/cleanup expressions, and `scripts/capture-demos` performs the
capture in one isolated nested compositor session.

Titles, trigger labels, and setup/action/cleanup strings must be nonempty
and contain no tabs or line breaks. Use `#t` for a Scheme step that does
nothing. The input list may be empty, but individual key names follow the
same text restrictions. Manifest validation runs before the nested session
starts, so malformed fields cannot shift actions into the wrong capture step.

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
make demos check-demos
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

Commands with a recorded demonstration have a demo ID that names exactly one
scenario. Manifest generation fails on missing, duplicate or stale IDs.
Utility and modal commands without a dedicated recording, and public bindings
without a command scenario, use the explicit classification `non-visual` in
the generated API inventory. Focused Scheme suites cover their behavior. The
live boundary is fully described; some pre-1.0 curation debt remains (see
`architecture.md`).
