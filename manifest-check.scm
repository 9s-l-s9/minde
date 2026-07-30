;; Fast-gate environment: everything `scripts/ci`'s default gate (cargo
;; fmt/check, the Scheme suites, static analysis, docs, release metadata)
;; actually touches, and nothing else. manifest.scm's e2e/demo/app-matrix
;; tools (Xorg, xdotool, imagemagick, grim, wf-recorder, foot, xterm,
;; wl-clipboard, wlr-randr, wayland-utils, swayidle, wtype, wlrctl, ffmpeg,
;; jq, util-linux) are real runtime dependencies of gates this environment
;; never runs (./check --all, make check-e2e/check-apps*/demos), not of
;; the default gate -- installing them here would only slow down every
;; ordinary push for tools nothing in this path calls.
;;
;; Usage: guix shell -m manifest-check.scm
(specifications->manifest
 '(;; Rust toolchain
   "rust"
   "rust:cargo"
   "rust:tools"
   "gcc-toolchain"
   "pkg-config"
   ;; Guile (embedded via libguile)
   "guile"
   ;; Smithay native deps -- linked at compile time, so `cargo check`
   ;; needs these even though it never runs the compositor.
   "wayland"
   "wayland-protocols"
   "libxkbcommon"
   "libinput-minimal"
   "eudev"
   "libseat"
   "mesa"
   "libglvnd"
   "libx11"
   "libxcb"
   "libxi"
   "libxcursor"
   "libxrandr"
   ;; used by the static/doc-drift gates
   "shellcheck"
   "ripgrep"
   "diffutils"
   ;; misc
   "git"))
