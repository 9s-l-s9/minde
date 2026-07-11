;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (guix packages)
             (guix gexp)
             (guix build-system copy)
             ((guix licenses) #:prefix license:))

(define %repository-root
  (canonicalize-path (string-append (dirname (current-filename)) "/..")))

(define (foundation-source? file stat)
  (let ((relative (string-drop file (+ 1 (string-length %repository-root)))))
    (or (member relative '("COPYING" "LICENSES"
                           "LICENSES/GPL-3.0-or-later.txt"
                           "scheme" "scheme/minde"
                           "scheme/minde/foundation"))
        (string-prefix? "scheme/minde/foundation/" relative))))

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
   "Pure Guile modules for rectangles and directional selection, binary split
trees, single-datum serialization, hook registries, and key notation.")
  (license license:gpl3+))
