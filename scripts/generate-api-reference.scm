#!/bin/sh
exec guile --no-auto-compile -L scheme -s "$0" "$@"
!#
;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Generate the public Scheme binding inventory from live Guile interfaces.

(use-modules (srfi srfi-1)
             (srfi srfi-13)
             (ice-9 ftw)
             (minde commands)
             (minde command-catalog))

;; Source positions, so the reference can link each binding straight to its
;; definition (like Zig's autodoc: docs that go to code, not just about it).
(read-enable 'positions)

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

(define (binding-name<? left right)
  (string<? (symbol->string (car left)) (symbol->string (car right))))

(define (module-bindings module-name)
  (sort
   (module-map (lambda (name variable)
                 (cons name (variable-ref variable)))
               (resolve-interface module-name))
   binding-name<?))

(define (one-line text)
  (string-join (string-tokenize text) " "))

(define (markdown text)
  (string-join (string-split (one-line text) #\|) "\\|"))

(define (binding-kind value)
  (cond
   ((procedure? value) "procedure")
   ((string-contains (format #f "~s" value) "syntax-transformer") "syntax")
   (else "value")))

(define (positional-arguments count label)
  (map (lambda (index) (format #f "~a-~a" label (+ index 1)))
       (iota count)))

;; Guile only retains define* keyword names in procedure properties for some
;; compiled procedures.  Parse the authoritative source signatures instead so
;; generated output is identical with a warm cache, a cold cache, and inside a
;; Guix shell.
(define source-signatures (make-hash-table))
(define source-documentation (make-hash-table))
(define source-locations (make-hash-table))

(define (source-key module-name name)
  (format #f "~s/~a" module-name name))

(define (record-signature! module-name name arguments)
  (when (and (symbol? name) (not (hash-ref source-signatures name)))
    (hash-set! source-signatures name (cons name arguments)))
  (when (and module-name (symbol? name))
    (hash-set! source-signatures (source-key module-name name)
               (cons name arguments))))

(define (record-location! module-name name path line)
  (when (and (symbol? name) path line)
    (let ((location (cons path (+ line 1)))) ; 1-index for display/GitHub links
      (unless (hash-ref source-locations name)
        (hash-set! source-locations name location))
      (when module-name
        (hash-set! source-locations (source-key module-name name) location)))))

(define (record-documentation! module-name name body)
  (when (and (symbol? name) (pair? body) (string? (car body))
             (not (hash-ref source-documentation name)))
    (hash-set! source-documentation name (car body)))
  (when (and module-name (symbol? name) (pair? body) (string? (car body)))
    (hash-set! source-documentation (source-key module-name name) (car body))))

;; SRFI-9 accessors are syntax transformers, and exported constants are values;
;; neither can carry a procedure docstring.  Their defining module may keep an
;; adjacent, quoted %api-binding-documentation alist as the source of truth.
(define (record-documentation-metadata! module-name form)
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
       (hash-set! source-documentation (source-key module-name (car entry))
                  (cdr entry))
       (unless (hash-ref source-documentation (car entry))
         (hash-set! source-documentation (car entry) (cdr entry))))
     (car (cdaddr form)))))

(define (record-definition! module-name form path line)
  (when (pair? form)
    (record-documentation-metadata! module-name form)
    (cond
     ((and (memq (car form) '(define define*))
           (> (length form) 2))
      (let ((target (cadr form)))
        (cond
         ((and (pair? target) (symbol? (car target)))
          (record-signature! module-name (car target) (cdr target))
          (record-documentation! module-name (car target) (cddr form))
          (record-location! module-name (car target) path line))
         ((and (symbol? target)
               (pair? (caddr form))
               (eq? (caaddr form) 'lambda))
          (record-signature! module-name target (cadr (caddr form)))
          (record-documentation! module-name target (cddr (caddr form)))
          (record-location! module-name target path line)))))
     ((and (eq? (car form) 'define-record-type)
           (> (length form) 3))
      (let ((constructor (list-ref form 2))
            (predicate (list-ref form 3))
            (fields (drop form 4)))
        (when (pair? constructor)
          (record-signature! module-name (car constructor) (cdr constructor))
          (record-location! module-name (car constructor) path line))
        (record-signature! module-name predicate '(RECORD))
        (record-location! module-name predicate path line)
        (for-each
         (lambda (field)
           (when (and (list? field) (> (length field) 1))
             (record-signature! module-name (list-ref field 1) '(RECORD))
             (record-location! module-name (list-ref field 1) path line)
             (when (> (length field) 2)
               (record-signature! module-name (list-ref field 2)
                                  '(RECORD VALUE))
               (record-location! module-name (list-ref field 2) path line))))
         fields))))))

(define (scheme-files directory)
  (append-map
   (lambda (entry)
     (if (member entry '("." ".."))
         '()
         (let ((path (string-append directory "/" entry)))
           (cond
            ((file-is-directory? path) (scheme-files path))
            ((string-suffix? ".scm" path) (list path))
            (else '())))))
   (sort (scandir directory) string<?)))

(for-each
 (lambda (path)
   (call-with-input-file path
     (lambda (port)
       (let loop ((module-name #f))
         (let ((form (read port)))
           (unless (eof-object? form)
             (let ((next-module
                    (if (and (pair? form) (eq? (car form) 'define-module))
                        (cadr form)
                        module-name))
                   (line (assq-ref (source-properties form) 'line)))
               (record-definition! next-module form path line)
               (loop next-module))))))))
 (scheme-files "scheme"))

(define (source-ref table module-name name)
  (or (hash-ref table (source-key module-name name))
      (hash-ref table name)))

(define (procedure-signature module-name name procedure)
  (let ((source (source-ref source-signatures module-name name)))
    (if source
        (format #f "~s" source)
        (let* ((arity (procedure-minimum-arity procedure))
               (required (list-ref arity 0))
               (optional (list-ref arity 1))
               (rest? (list-ref arity 2))
               (parts (append
                       (positional-arguments required "ARG")
                       (map (lambda (argument) (format #f "[~a]" argument))
                            (positional-arguments optional "OPTIONAL"))
                       (if rest? '(". REST") '()))))
          (format #f "(~a~a)" name
                  (if (null? parts)
                      ""
                      (string-append " " (string-join parts " "))))))))

(define (binding-signature module-name name value)
  (let ((source (source-ref source-signatures module-name name)))
    (cond
     (source (format #f "~s" source))
     ((procedure? value) (procedure-signature module-name name value))
     (else (symbol->string name)))))

(define (command-for name)
  (command-ref name))

(define (binding-documentation module-name name value)
  (let ((source-doc (source-ref source-documentation module-name name))
        (doc (and (procedure? value) (procedure-documentation value)))
        (command (command-for name)))
    (cond
     (source-doc source-doc)
     ((and doc (not (string-null? doc))) doc)
     (command (command-documentation command))
     (else "No Guile docstring is attached."))))

(define (binding-source module-name name)
  (let ((location (source-ref source-locations module-name name)))
    (if location
        (format #f "[`~a:~a`](~a~a#L~a)"
                (car location) (cdr location)
                %github-blob-base (car location) (cdr location))
        "—")))

(define (binding-demo name)
  (let ((command (command-for name)))
    (if command
        (symbol->string (command-demo-id command))
        "non-visual")))

(define (documented? module-name name value)
  (or (source-ref source-documentation module-name name)
      (and (procedure? value) (procedure-documentation value))
      (command-for name)))

(register-builtin-command-schemas!)

(define all-bindings
  (append-map (lambda (module-name)
                (map (lambda (binding) (cons module-name binding))
                     (module-bindings module-name)))
              public-modules))

(define missing-count
  (count (lambda (entry)
           (not (documented? (car entry) (cadr entry) (cddr entry))))
         all-bindings))

(display "# Generated Scheme API reference\n\n")
(display "<!-- Generated by scripts/generate-api-reference.scm; do not edit. -->\n\n")
(format #t "This inventory enumerates the live exports of all eight public modules. It currently contains **~a bindings**; **~a** still lack a source docstring, non-procedure metadata, or command-catalog description. Missing descriptions remain visible as release debt rather than being omitted.\n\n"
        (length all-bindings) missing-count)

(for-each
 (lambda (module-name)
   (let ((bindings (module-bindings module-name)))
     (format #t "## `~s`\n\n" module-name)
     (display "| Binding | Kind | Signature | Description | Demonstration | Source |\n")
     (display "|---|---|---|---|---|---|\n")
     (for-each
      (lambda (binding)
        (let ((name (car binding)) (value (cdr binding)))
          (format #t "| `~a` | ~a | `~a` | ~a | `~a` | ~a |\n"
                  name
                  (binding-kind value)
                  (binding-signature module-name name value)
                  (markdown (binding-documentation module-name name value))
                  (binding-demo name)
                  (binding-source module-name name))))
      bindings)
     (newline)))
 public-modules)

(display "## Registered commands\n\n")
(display "| Command | Arguments | Category | Summary | Demo ID |\n")
(display "|---|---|---|---|---|\n")
(for-each
 (lambda (name)
   (let ((command (command-ref name)))
     (format #t "| `~a` | `~s` | `~a` | ~a | `~a` |\n"
             name
             (command-arguments command)
             (command-category command)
             (markdown (command-summary command))
             (command-demo-id command))))
 (command-names))
