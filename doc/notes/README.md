# Working notes

Working notes from the automation/browser-interop pass (commits
9f133e0..726bc65, 2026-08-31/09-01). These are session notes kept for
their debugging detail, not maintained reference documentation; nothing
else in the repository links to them and they are outside `check-doc-links`
coverage. Each implemented issue has a one-line summary in `CHANGELOG.md`
under "Unreleased".

- `AUTOMATION-WISHLIST.md` — running list of automation primitive gaps
  found while scripting real browser workflows (clicks, paste, drag-and-
  drop, screenshots).
- `web-form-quirks-playbook.md` — notes on how real web forms (Firefox/Zen)
  react to synthetic input, informing the fixes below.
- `issue-wm-click-paste-settle-timing.md` — clicks on custom controls were
  flaky and double-clicks went unrecognized; fixed by hovering and settling
  before synthetic clicks. Implemented.
- `issue-wm-drop-files.md` — original feature request for a
  `(wm-drop-files ...)` synthetic drag-and-drop primitive. Superseded in
  part, not fully, by `issue-wm-drop-files-rejected-by-dropzones.md` below
  (that file fixes a specific rejection bug; it does not cover the full
  API design in this one), so both are kept.
- `issue-wm-drop-files-rejected-by-dropzones.md` — browser dropzones
  rejected synthetic drops; fixed by dwelling longer with jitter during
  negotiated drops. Implemented and verified against a real dropzone.
- `issue-wm-screenshot-primitive.md` — native `(wm-screenshot ...)`
  primitive design and implementation. Implemented.
- `issue-wm-scroll-no-effect.md` — `wm-scroll` did not move web content;
  fixed by sending discrete wheel steps with scroll frames. Implemented.
- `issue-wm-type-drops-modifier-chars.md` — `wm-type`/`wm-send-string`
  dropped characters requiring a modifier the keymap can't produce; fixed
  by pasting characters the layout cannot type. Implemented.
