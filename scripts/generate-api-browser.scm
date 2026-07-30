#!/bin/sh
exec guile --no-auto-compile -L scheme -L tools/guile-autodoc -s "$0" "$@"
!#
;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Generate the interactive Scheme API browser: navigation, docs, and a
;;; source panel showing each binding's real source text, not just a link
;;; to it. Same underlying scan as scripts/generate-api-reference.scm (and
;;; the same Minde-specific glue, in scripts/lib/api-doc-support.scm --
;;; see that file), rendered by (autodoc browser) instead of
;;; (autodoc reference). Output is a complete HTML file, not Markdown.

(use-modules (autodoc scan)
             (autodoc browser)
             (minde commands)
             (minde command-catalog))

(load (string-append (dirname (current-filename)) "/lib/api-doc-support.scm"))

(register-builtin-command-schemas!)

(display (generate-api-browser
          #:modules public-modules
          #:source-directory "scheme"
          #:repo-blob-prefix %github-blob-base
          #:title "Minde API browser"
          #:extra-columns (list (cons "Demonstration" binding-demo-id))
          #:missing-docstring-text "No Guile docstring is attached."
          #:documentation-fallback documentation-fallback
          #:on-form register-documentation-metadata!))
