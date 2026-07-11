;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (guix packages)
             (guix gexp)
             (guix build-system copy)
             ((guix licenses) #:prefix license:))

(define %repository-root
  (canonicalize-path (string-append (dirname (current-filename)) "/..")))

(define (ui-source? file stat)
  (let ((relative (string-drop file (+ 1 (string-length %repository-root)))))
    (or (member relative '("COPYING" "LICENSES"
                           "LICENSES/GPL-3.0-or-later.txt"
                           "scheme" "scheme/minde" "scheme/minde/ui"))
        (string-prefix? "scheme/minde/ui/" relative))))

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
  (license license:gpl3+))
