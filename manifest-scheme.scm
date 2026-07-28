;; Scheme/static/docs-only environment: `scripts/ci --ci-scheme`'s gates
;; (the Scheme test suites, static analysis, generated-docs drift) touch
;; neither Rust nor Smithay's native deps, so this omits the whole
;; rust/gcc-toolchain/wayland/mesa/x11 side manifest-check.scm carries for
;; `cargo check` -- installing none of that is what lets this run in
;; parallel with (and finish well ahead of) the Rust job in CI.
;;
;; Usage: guix shell -m manifest-scheme.scm
(specifications->manifest
 '("guile"
   "shellcheck"
   "ripgrep"
   "diffutils"
   "git"))
