;;; SPDX-License-Identifier: GPL-3.0-or-later
;;;
;;; Shared between scripts/generate-api-reference.scm and
;;; scripts/generate-api-browser.scm -- both render the same underlying
;;; (autodoc scan) of scheme/ against Minde's live public modules, just
;;; as different views (a Markdown table vs. an interactive HTML browser),
;;; so what's specific to Minde (which modules are public, the
;;; %api-binding-documentation convention, the command-catalog-derived
;;; demo/description) is defined once here rather than kept in sync
;;; between two copies. `load'ed, not a module: it runs in the caller's
;;; module, which must already have (autodoc scan), (minde commands),
;;; and (minde command-catalog) use-module'd before loading this.

(define %github-blob-base
  "https://github.com/9s-l-s9/minde/blob/main/")

(define public-modules
  '((minde windows)
    (minde frames)
    (minde groups)
    (minde layouts)
    (minde input)
    (minde commands)
    (minde hooks)
    (minde status)))

;; SRFI-9 accessors are syntax transformers, and exported constants are
;; values; neither can carry a procedure docstring. Their defining module
;; may keep an adjacent, quoted %api-binding-documentation alist as the
;; source of truth -- fed into the scan via autodoc's on-form hook.
(define (register-documentation-metadata! scan module-name form path line)
  (when (and module-name
             (pair? form)
             (memq (car form) '(define define-public))
             (> (length form) 2)
             (eq? (cadr form) '%api-binding-documentation)
             (pair? (caddr form))
             (eq? (caaddr form) 'quote)
             (list? (car (cdaddr form))))
    (for-each
     (lambda (entry)
       (unless (and (pair? entry) (symbol? (car entry)) (string? (cdr entry)))
         (error "invalid %api-binding-documentation entry" entry))
       (register-documentation! scan module-name (car entry) (cdr entry))
       (register-documentation! scan #f (car entry) (cdr entry)))
     (car (cdaddr form)))))

(define (command-for name)
  (command-ref name))

;; Raw demo id, shared between both generators; each wraps it for its own
;; output format (Markdown backticks vs. plain text in an HTML <code>-free
;; label) rather than baking a format's syntax into the shared value.
(define (binding-demo-id module-name name value info)
  (let ((command (command-for name)))
    (if command (symbol->string (command-demo-id command)) "non-visual")))

(define (documentation-fallback module-name name value)
  (let ((command (command-for name)))
    (and command (command-documentation command))))

(define (documented? module-name name value info)
  (or (source-info-documentation info)
      (and (procedure? value) (procedure-documentation value))
      (command-for name)))
