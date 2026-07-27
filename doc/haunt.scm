;;; SPDX-License-Identifier: GPL-3.0-or-later
;;;
;;; GitHub Pages source config. Built by .github/workflows/pages.yml via
;;; `guix shell -m doc/manifest.scm -- haunt build`, run with doc/ as the
;;; working directory and tools/guile-autodoc (a git submodule) added to
;;; GUILE_LOAD_PATH, e.g. locally:
;;;
;;;     cd doc
;;;     guix shell -m manifest.scm -- sh -c \
;;;       'GUILE_LOAD_PATH="$PWD/../tools/guile-autodoc:$GUILE_LOAD_PATH" haunt build'
;;;
;;; The actual page-rendering logic (docs-pages, GFM table reassembly,
;;; .md-link rewriting) lives in tools/guile-autodoc
;;; (github.com/9s-l-s9/guile-autodoc, vendored as a submodule) -- pulled
;;; out from an earlier version of this file so the same source-scanning
;;; and doc-site logic scripts/generate-api-reference.scm also depends on
;;; isn't maintained twice. All that's specific to Minde here is the
;;; page template (adds a footer) and the site's own title/domain/paths.
;;;
;;; doc/*.md and doc/generated/*.md each render to one page, title taken
;;; from the file's first `# Heading`. No front matter is required, so the
;;; doc files themselves stay untouched -- they're read elsewhere too, e.g.
;;; tests/check-doc-links.sh, and shouldn't carry site-only metadata.
;;; doc/generated/{manual.html,demo-manifest.json,api-catalog.scm} are
;;; copied through verbatim by `static-directory`.

(use-modules (autodoc site)
             (haunt builder assets)
             (haunt site))

(define %github-blob-prefix
  "https://github.com/9s-l-s9/minde/blob/main/")

(define (page-template site title body)
  "default-template plus a footer -- the one thing Minde's docs site
wants beyond what (autodoc site) provides out of the box. Spliced into
default-template's known shape (its own docstring is the contract) rather
than duplicating the rest of the page around it."
  (let* ((doc (default-template site title body))
         (doctype-node (car doc))
         (html-node (cadr doc))
         (html-attrs (cadr html-node))
         (head-node (caddr html-node))
         (body-node (cadddr html-node))
         (div-node (cadr body-node))
         (div-attrs (cadr div-node))
         (div-children (cddr div-node)))
    `(,doctype-node
      (html ,html-attrs
       ,head-node
       (body (div ,div-attrs
              ,@div-children
              (footer "Generated documentation; source lives in doc/ of the "
                      (a (@ (href ,(string-append "https://" (site-domain site))))
                         "Minde")
                      " repository.")))))))

(site #:title "Minde"
      #:domain "github.com/9s-l-s9/minde"
      #:builders (list (docs-pages "." #:template page-template
                                   #:external-prefix %github-blob-prefix)
                       (static-directory "generated"))
      #:build-directory "_site")
