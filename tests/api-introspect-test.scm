;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Runtime API introspection: describe-api returns non-empty, writable,
;;; re-readable data for each section and honors the apropos filter
;;; (scheme/api-introspect.scm).

(use-modules (srfi srfi-1)
             (srfi srfi-13)
             (ice-9 rdelim)
             (minde commands)
             (minde control)
             (minde command-catalog))

;; describe-api reflects the documented public modules; make them resolvable.
(use-modules (minde windows)
             (minde frames)
             (minde groups)
             (minde layouts)
             (minde input)
             (minde hooks)
             (minde status))

(load-from-path "api-introspect.scm")
(load-from-path "ipc-reply.scm")

(define failures 0)
(define (check description value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" description)))

;; A datum satisfies the writable-data guarantee when it survives a full
;; write/read cycle unchanged and prints without any raw #<...> object.
(define (writable-roundtrips? datum)
  (let ((text (call-with-output-string (lambda (p) (write datum p)))))
    (and (not (string-contains text "#<"))
         (equal? datum (call-with-input-string text read)))))

;; Populate the command registry the way the running compositor does.
(register-builtin-command-schemas!)
(register-control-commands!)

(define sections '(commands procedures gsubrs hooks))

;; --- Unfiltered: every section is present, non-empty and re-readable -------

(define catalog (describe-api))
(check "describe-api returns an alist"
       (and (list? catalog) (every pair? catalog)))
(for-each
 (lambda (key)
   (let ((entries (assq-ref catalog key)))
     (check (format #f "section ~a is present" key) (list? entries))
     (check (format #f "section ~a is non-empty" key)
            (and (list? entries) (> (length entries) 0)))
     (check (format #f "section ~a is writable re-readable data" key)
            (writable-roundtrips? entries))))
 sections)

;; The whole catalog round-trips as a single datum.
(check "whole catalog is writable re-readable data"
       (writable-roundtrips? catalog))
(check "complete catalog fits the bounded IPC reply without losing metadata"
       (equal? (ipc-ok-reply catalog) (list 'ok catalog)))

;; Public procedures must expose callable signatures and their actual docs,
;; including procedures re-exported from the private implementation module.
(define (entry-named entries name)
  (or (find (lambda (entry) (eq? (assq-ref entry 'name) name)) entries) '()))
(let* ((entries (api-module-procedures '(minde windows) #f))
       (geometry (entry-named entries 'window-geometry))
       (all (entry-named entries 'all-window-ids)))
  (check "public procedure has its required arity"
         (equal? (assq-ref geometry 'signature) "(window-geometry arg-1)"))
  (check "public procedure retains its documentation"
         (string-contains (assq-ref geometry 'documentation) "Return visible window ID"))
  (check "re-exported procedure has its zero-argument signature"
         (equal? (assq-ref all 'signature) "(all-window-ids)"))
  (check "re-exported procedure retains its documentation"
         (string-contains (assq-ref all 'documentation) "active-group window identifiers")))

;; An isolated public interface covers values and unbound exports too.
(let ((fixture (resolve-module '(minde-test introspection))) (called? #f))
  (set-module-public-interface! fixture (make-module))
  (module-define! fixture 'callable
    (lambda* (required #:optional optional #:rest rest)
      "Fixture procedure documentation."
      (set! called? #t)))
  (module-define! fixture 'undocumented (lambda () (set! called? #t)))
  (module-define! fixture 'constant 7)
  (module-define! fixture 'disabled #f)
  (module-export! fixture '(callable undocumented constant disabled unavailable))
  (let* ((entries (api-module-procedures '(minde-test introspection) #f))
         (callable (entry-named entries 'callable)))
    (check "signature distinguishes required, optional and rest arguments"
           (equal? (assq-ref callable 'signature) "(callable arg-1 [opt-1] ...)"))
    (check "procedure documentation is the bound procedure's string"
           (equal? (assq-ref callable 'documentation) "Fixture procedure documentation."))
    (check "undocumented procedure still has a callable signature"
           (equal? (assq-ref (entry-named entries 'undocumented) 'signature) "(undocumented)"))
    (check "missing procedure documentation remains empty"
           (equal? (assq-ref (entry-named entries 'undocumented) 'documentation) ""))
    (for-each
     (lambda (name)
       (let ((entry (entry-named entries name)))
         (check "unbound and non-procedure exports retain bare signatures"
                (equal? (assq-ref entry 'signature) (symbol->string name)))
         (check "unbound and non-procedure exports have empty documentation"
                (equal? (assq-ref entry 'documentation) ""))))
     '(constant disabled unavailable))
    (check "filtered lookup retains procedure metadata"
           (equal? (api-module-procedures '(minde-test introspection) "CALL") (list callable)))
    (check "mixed public bindings remain writable data" (writable-roundtrips? entries))
    (check "introspection never invokes a procedure" (not called?))))

;; --- Every registered command appears in the commands section --------------

(define catalog-command-names
  (map (lambda (entry) (assq-ref entry 'name))
       (assq-ref catalog 'commands)))
(for-each
 (lambda (name)
   (check (format #f "command ~a appears in describe-api" name)
          (memq name catalog-command-names)))
 (command-names))
(check "commands section count matches the registry"
       (= (length catalog-command-names) (length (command-names))))

;; --- Each item carries the documented fields -------------------------------

(for-each
 (lambda (entry)
   (check "command entry has category, summary, arguments, documentation"
          (and (assq 'category entry) (assq 'summary entry)
               (assq 'arguments entry) (assq 'documentation entry))))
 (assq-ref catalog 'commands))
(for-each
 (lambda (entry)
   (check "gsubr entry has a signature and documentation string"
          (and (string? (assq-ref entry 'signature))
               (string? (assq-ref entry 'documentation)))))
 (assq-ref catalog 'gsubrs))
(for-each
 (lambda (entry)
   (check "procedure entry names its module"
          (list? (assq-ref entry 'module))))
 (assq-ref catalog 'procedures))
(for-each
 (lambda (entry)
   (check "hook entry has an arguments list"
          (list? (assq-ref entry 'arguments))))
 (assq-ref catalog 'hooks))

(let ((urgent (entry-named (assq-ref (describe-api "urgent") 'hooks) 'urgent-window)))
  (check "urgency hook is discoverable with its window ID payload"
         (equal? (assq-ref urgent 'arguments) '(id)))
  (check "urgency hook has meaningful documentation"
         (let ((doc (assq-ref urgent 'documentation)))
           (and (string? doc) (not (string-null? doc))))))

;; --- Gsubr metadata covers every register_gsubr call in the Rust source ----
;; The gsubr table in api-introspect.scm is hand-maintained; parse the
;; registration sites out of src/guile/mod.rs and fail on any name missing
;; from describe-api, so the table cannot silently fall behind.

(define (registered-gsubr-names)
  (call-with-input-file "src/guile/mod.rs"
    (lambda (port)
      (let loop ((line (read-line port)) (pending #f) (names '()))
        (cond
         ((eof-object? line) (delete-duplicates (reverse names)))
         (else
          (let* ((source (if pending (string-append pending line) line))
                 (call (string-contains source "register_gsubr("))
                 (open (and call (string-index source #\" call)))
                 (close (and open (string-index source #\" (+ open 1)))))
            (cond
             (close
              (loop (read-line port) #f
                    (cons (string->symbol
                           (substring source (+ open 1) close))
                          names)))
             ;; register_gsubr( with the name on the next line: carry over.
             (call (loop (read-line port) source names))
             (else (loop (read-line port) #f names))))))))))

(define catalog-gsubr-names
  (map (lambda (entry) (assq-ref entry 'name))
       (assq-ref catalog 'gsubrs)))
(let ((registered (registered-gsubr-names)))
  (check "found the register_gsubr sites in src/guile/mod.rs"
         (> (length registered) 20))
  (for-each
   (lambda (name)
     (check (format #f "gsubr ~a has describe-api metadata" name)
            (memq name catalog-gsubr-names)))
   registered))

;; --- The apropos filter narrows every section by name ----------------------

(let ((filtered (describe-api "wm-")))
  (check "filter keeps matching gsubrs"
         (> (length (assq-ref filtered 'gsubrs)) 0))
  (check "filter drops non-matching commands"
         (null? (assq-ref filtered 'commands)))
  (check "filter is case-insensitive"
         (equal? (assq-ref (describe-api "WM-") 'gsubrs)
                 (assq-ref filtered 'gsubrs))))

(let ((filtered (describe-api "focus")))
  (check "filter keeps matching commands"
         (every (lambda (entry)
                  (string-contains (symbol->string (assq-ref entry 'name))
                                   "focus"))
                (assq-ref filtered 'commands)))
  (check "focus filter matches at least one command"
         (> (length (assq-ref filtered 'commands)) 0)))

;; A filter matching nothing yields empty sections, not an error.
(let ((none (describe-api "zzz-nonexistent-zzz")))
  (check "unmatched filter empties every section"
         (every (lambda (key) (null? (assq-ref none key))) sections)))

;; New user commands share the runtime and generated discovery contract.
(register-command!
 'example-focus! (lambda (window) window)
 #:parameters '(((name . window) (type . integer) (minimum . 1)))
 #:category 'window #:summary "Focus an explicit window"
 #:result-schema 'integer #:scope '(window explicit) #:effects '(focus)
 #:completion 'synchronous #:retry 'idempotent
 #:examples '((invoke-command/named 'example-focus! '((window . 42)))))
(let ((entry (car (assq-ref (describe-api "example-focus!") 'commands))))
  (check "describe-api and detailed action help agree"
         (equal? entry (describe-action 'example-focus!)))
  (check "typed introspection still includes the legacy command fields"
         (every (lambda (key) (assq key entry))
                '(name category summary arguments documentation)))
  (check "typed metadata provides argument/result/effect contracts"
         (and (equal? '(window) (assq-ref entry 'arguments))
              (assq-ref entry 'typed?)
              (equal? 'integer (assq-ref entry 'result-schema))
              (equal? 'synchronous (assq-ref entry 'completion))
              (assq-ref (car (assq-ref entry 'parameters)) 'required)))
  (check "typed catalog with executable examples is readable"
         (writable-roundtrips? entry)))
(check "concise discovery finds a custom command"
       (equal? '(example-focus!)
               (map (lambda (entry) (assq-ref entry 'name))
                    (search-actions "explicit window"))))
(check "concise discovery has fewer fields than detailed help"
       (< (length (car (search-actions "explicit window")))
          (length (describe-action 'example-focus!))))

(if (zero? failures)
    (format #t "api-introspect-test: all checks passed~%")
    (begin
      (format #t "api-introspect-test: ~a failure(s)~%" failures)
      (exit 1)))
