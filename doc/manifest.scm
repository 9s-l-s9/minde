;; Build environment for the GitHub Pages site (doc/haunt.scm).
;; Usage: guix shell -m doc/manifest.scm -- haunt build
;;
;; Deliberately separate from the top-level manifest.scm: the docs build
;; never needs the Rust toolchain or Smithay's native deps, and keeping it
;; apart means the pages workflow doesn't pull those in just to render
;; Markdown.
(specifications->manifest
 '("haunt"))
