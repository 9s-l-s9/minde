;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Channel module: `guix pull` against this repository loads exactly this
;;; file (per .guix-channel -> directory "guix-channel"). It resolves package
;;; variables from the *committed* checkout of whatever ref the user pulled;
;;; it never sees the gitignored `vendor/` mirror that guix.scm and
;;; guix/release.scm depend on, and it must load without error under a bare
;;; `guix pull` (no maintainer environment variables set, no network access
;;; beyond fetching this repository itself).
;;;
;;; What this means in practice:
;;;
;;;  - guile-minde-foundation and guile-minde-ui are pure Scheme
;;;    (see guix/foundation.scm and guix/ui.scm) and need nothing beyond the
;;;    committed `scheme/` tree, so they are reproduced here as proper
;;;    channel-module definitions and are buildable today, from a bare
;;;    `guix pull`, with `guix build -e '(@ (minde packages)
;;;    guile-minde-foundation)'` or by name once the channel has been
;;;    pulled (`guix build guile-minde-foundation`).
;;;
;;;  - The full `minde` compositor is NOT exposed here. It vendors its
;;;    Rust/Smithay dependency graph under `vendor/`, which is deliberately
;;;    gitignored (see guix.scm's header comment and doc/releasing.md); a
;;;    channel checkout never has that tree, so a channel package cannot
;;;    build it the way `guix.scm` does from a local checkout. The public,
;;;    non-maintainer build path is `guix/release.scm`, which builds from a
;;;    *vendored release archive* fetched by URL and verified by hash
;;;    (MINDE_SOURCE_ARCHIVE) -- exactly the shape a channel package needs
;;;    (origin + url-fetch + sha256), but only once a tagged release has
;;;    published that archive at a stable URL. As of this writing (the
;;;    Sprint 12 in-repo channel work) no such release has been published
;;;    yet, so there is no URL+hash pair to pin. Once one exists, add a
;;;    `minde` variable here whose `origin` is a `url-fetch` of the
;;;    published `minde-<version>-vendored.tar.gz` archive (with its
;;;    published sha256 from SHA256SUMS, see doc/releasing.md) feeding the
;;;    same build recipe as guix.scm's archive branch. Do not add it with a
;;;    placeholder/dummy hash: an unbuildable-by-design package is worse than
;;;    an honestly absent one, and a bad hash would still let the module load
;;;    (only `guix build` would fail), silently misleading channel users.
;;;
;;; Trade-off, stated plainly: today, `guix pull`-ing this channel gets you
;;; the two reusable Scheme libraries only, not the compositor itself. Until
;;; the first release archive is published, installing the compositor still
;;; requires cloning the repository and using `guix.scm` (or `guix system
;;; reconfigure`/`guix home` referencing a checkout), as documented in
;;; doc/releasing.md and README.md.

(define-module (minde packages)
  #:use-module (guix packages)
  #:use-module (guix gexp)
  #:use-module ((guix utils) #:select (current-source-directory))
  #:use-module (guix build-system copy)
  #:use-module ((guix licenses) #:prefix license:)
  #:export (guile-minde-foundation
            guile-minde-ui))

;; Two directories up from this file (guix-channel/minde/packages.scm) is
;; the repository root, i.e. the same root `directory` in .guix-channel is
;; relative to. This must stay a plain path computation (no vendor/, no
;; environment variables) so the module loads under a bare `guix pull`.
;; The path is resolved through %load-path rather than current-filename:
;; `guix pull` compiles channel modules to bytecode, and current-filename is
;; #f when a module is loaded from a .go file, which made every package in
;; this module invisible after a real pull while `guix build -L guix-channel`
;; (which interprets the source) kept working. The channel module directory
;; is always on %load-path when this module is loadable at all.
(define %repository-root
  (canonicalize-path
   (string-append
    (dirname (dirname (search-path %load-path "minde/packages.scm")))
    "/..")))

(define (foundation-source? file stat)
  (let ((relative (string-drop file (+ 1 (string-length %repository-root)))))
    (or (member relative '("COPYING" "LICENSES"
                           "LICENSES/GPL-3.0-or-later.txt"
                           "scheme" "scheme/minde"
                           "scheme/minde/foundation"))
        (string-prefix? "scheme/minde/foundation/" relative))))

(define (ui-source? file stat)
  (let ((relative (string-drop file (+ 1 (string-length %repository-root)))))
    (or (member relative '("COPYING" "LICENSES"
                           "LICENSES/GPL-3.0-or-later.txt"
                           "scheme" "scheme/minde" "scheme/minde/ui"))
        (string-prefix? "scheme/minde/ui/" relative))))

(define-public guile-minde-foundation
  (package
    (name "guile-minde-foundation")
    (version "1.0.0-rc1")
    (source (local-file %repository-root "minde-foundation-source"
                        #:recursive? #t #:select? foundation-source?))
    (build-system copy-build-system)
    (arguments
     (list #:install-plan
           #~'(("scheme/minde/foundation"
                "share/guile/site/3.0/minde/foundation")
               ("COPYING" "share/doc/guile-minde-foundation/COPYING")
               ("LICENSES/GPL-3.0-or-later.txt"
                "share/doc/guile-minde-foundation/GPL-3.0-or-later.txt"))))
    (home-page "https://github.com/9s-l-s9/minde")
    (synopsis "Reusable Scheme foundations extracted from minde")
    (description
     "Pure Guile modules for rectangles and directional selection, binary
split trees, single-datum serialization, hook registries, and key
notation.")
    (license license:gpl3+)))

(define-public guile-minde-ui
  (package
    (name "guile-minde-ui")
    (version "1.0.0-rc1")
    (source (local-file %repository-root "minde-ui-source" #:recursive? #t
                        #:select? ui-source?))
    (build-system copy-build-system)
    (arguments
     (list #:install-plan
           #~'(("scheme/minde/ui"
                "share/guile/site/3.0/minde/ui")
               ("COPYING" "share/doc/guile-minde-ui/COPYING")
               ("LICENSES/GPL-3.0-or-later.txt"
                "share/doc/guile-minde-ui/GPL-3.0-or-later.txt"))))
    (home-page "https://github.com/9s-l-s9/minde")
    (synopsis "Injectable prompt and menu state machines from minde")
    (description
     "Guile prompt and menu engines whose rendering, repeat, and clipboard
operations are injected callbacks, allowing use without a compositor or
display server.")
    (license license:gpl3+)))
