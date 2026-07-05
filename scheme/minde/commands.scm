;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Canonical command metadata and invocation.

(define-module (minde commands)
  #:use-module (srfi srfi-9)
  #:use-module (srfi srfi-1)
  #:use-module (ice-9 optargs)
  #:use-module (ice-9 regex)
  #:export (register-command!
            clear-command-registry!
            command?
            command-name
            command-procedure
            command-arguments
            command-category
            command-summary
            command-documentation
            command-demo-id
            command-ref
            command-names
            commands-in-category
            invoke-command))

(define-record-type <command>
  (make-command name procedure arguments category summary documentation demo-id)
  command?
  (name command-name)
  (procedure command-procedure)
  (arguments command-arguments)
  (category command-category)
  (summary command-summary)
  (documentation command-documentation)
  (demo-id command-demo-id))

;; Source documentation for SRFI-9 syntax bindings, which cannot carry Guile
;; procedure docstrings.  The API generator reads this adjacent metadata.
(define %api-binding-documentation
  '((command? . "Returns true when RECORD is a command metadata record.")
    (command-name . "Returns RECORD's canonical command name symbol.")
    (command-procedure . "Returns the procedure invoked by command RECORD.")
    (command-arguments . "Returns RECORD's ordered command argument names.")
    (command-category . "Returns RECORD's command category symbol.")
    (command-summary . "Returns RECORD's concise user-facing summary.")
    (command-documentation . "Returns RECORD's full user-facing documentation.")
    (command-demo-id . "Returns RECORD's scripted demonstration identifier.")))

(define %commands (make-hash-table))

(define (canonical-command-name? name)
  (and (symbol? name)
       (string-match "^[a-z][a-z0-9]*(-[a-z0-9]+)*[!?]?$"
                     (symbol->string name))))

(define* (register-command! name procedure
                            #:key (arguments '()) category summary
                            (documentation summary) demo-id)
  "Register one canonical command and its user-facing metadata."
  (unless (canonical-command-name? name)
    (error "command name is not canonical kebab-case" name))
  (unless (procedure? procedure) (error "command procedure required" name))
  (unless (and (list? arguments) (every symbol? arguments))
    (error "command arguments must be symbols" name arguments))
  (unless (symbol? category) (error "command category required" name))
  (unless (and (string? summary) (not (string-null? summary)))
    (error "command summary required" name))
  (unless (and (string? documentation) (not (string-null? documentation)))
    (error "command documentation required" name))
  (when (hash-ref %commands name)
    (error "duplicate command" name))
  (let ((command (make-command name procedure arguments category summary
                               documentation demo-id)))
    (hash-set! %commands name command)
    command))

(define (clear-command-registry!)
  "Removes every command from the process-local registry."
  (set! %commands (make-hash-table)))
(define (command-ref name)
  "Returns the registered command named NAME, or #f."
  (hash-ref %commands name))
(define (command-name<? a b)
  (string<? (symbol->string a) (symbol->string b)))
(define (command-names)
  "Returns every registered command name in lexical order."
  (sort (hash-map->list (lambda (name _) name) %commands) command-name<?))
(define (commands-in-category category)
  "Returns registered command records whose category is CATEGORY."
  (filter (lambda (command) (eq? category (command-category command)))
          (map command-ref (command-names))))
(define (invoke-command name . arguments)
  "Invokes command NAME with ARGUMENTS after validating existence and arity."
  (let ((command (command-ref name)))
    (unless command (error "unknown command" name))
    (unless (= (length arguments) (length (command-arguments command)))
      (error "wrong command argument count" name
             (length (command-arguments command)) (length arguments)))
    (apply (command-procedure command) arguments)))
