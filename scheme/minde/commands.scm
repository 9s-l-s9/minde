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

(define (clear-command-registry!) (set! %commands (make-hash-table)))
(define (command-ref name) (hash-ref %commands name))
(define (command-name<? a b)
  (string<? (symbol->string a) (symbol->string b)))
(define (command-names)
  (sort (hash-map->list (lambda (name _) name) %commands) command-name<?))
(define (commands-in-category category)
  (filter (lambda (command) (eq? category (command-category command)))
          (map command-ref (command-names))))
(define (invoke-command name . arguments)
  (let ((command (command-ref name)))
    (unless command (error "unknown command" name))
    (unless (= (length arguments) (length (command-arguments command)))
      (error "wrong command argument count" name
             (length (command-arguments command)) (length arguments)))
    (apply (command-procedure command) arguments)))
