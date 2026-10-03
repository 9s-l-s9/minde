;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Canonical command metadata and invocation.

(define-module (minde commands)
  #:use-module (srfi srfi-9)
  #:use-module (srfi srfi-1)
  #:use-module (srfi srfi-13)
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
            command-parameters
            command-result-schema
            command-scope
            command-effects
            command-preconditions
            command-completion
            command-retry
            command-examples
            command-typed?
            command-ref
            command-names
            commands-in-category
            invoke-command
            invoke-command/named
            validate-command-arguments
            schema-value-valid?
            describe-action
            search-actions
            capabilities))

(define-record-type <command>
  (make-command name procedure arguments category summary documentation demo-id
                parameters result-schema scope effects preconditions completion
                retry examples)
  command?
  (name command-name)
  (procedure command-procedure)
  (arguments command-arguments)
  (category command-category)
  (summary command-summary)
  (documentation command-documentation)
  (demo-id command-demo-id)
  (parameters command-parameters)
  (result-schema command-result-schema)
  (scope command-scope)
  (effects command-effects)
  (preconditions command-preconditions)
  (completion command-completion)
  (retry command-retry)
  (examples command-examples))

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
    (command-demo-id . "Returns RECORD's scripted demonstration identifier.")
    (command-parameters . "Returns RECORD's typed named parameter schemas, or #f for legacy commands.")
    (command-result-schema . "Returns RECORD's declared result schema, or unknown.")
    (command-scope . "Returns RECORD's declared target scope, or unknown.")
    (command-effects . "Returns RECORD's declared effects, or unknown.")
    (command-preconditions . "Returns RECORD's documented preconditions, or unknown.")
    (command-completion . "Returns RECORD's completion semantics, or unknown.")
    (command-retry . "Returns RECORD's retry behavior, or unknown.")
    (command-examples . "Returns RECORD's executable Scheme example data.")))

(define %commands (make-hash-table))

(define (canonical-command-name? name)
  (and (symbol? name)
       (string-match "^[a-z][a-z0-9]*(-[a-z0-9]+)*[!?]?$"
                     (symbol->string name))))

(define* (register-command! name procedure
                            #:key (arguments '()) category summary
                            (documentation summary) demo-id
                            (parameters #f) (result-schema 'unknown)
                            (scope 'unknown) (effects 'unknown)
                            (preconditions 'unknown) (completion 'unknown)
                            (retry 'unknown) (examples '()))
  "Register a command. PARAMETERS is #f for legacy positional commands, or a
list of named schemas. Each schema has name/type and optional required,
default, enum, minimum, maximum, min-length, max-length, and description.
An optional parameter must have a valid default. Typed parameters determine
argument order. Other metadata is readable data; unknown means undeclared."
  (unless (canonical-command-name? name)
    (error "command name is not canonical kebab-case" name))
  (unless (procedure? procedure) (error "command procedure required" name))
  (unless (and (list? arguments) (every symbol? arguments))
    (error "command arguments must be symbols" name arguments))
  (when parameters
    (validate-parameter-schemas! name parameters)
    (set! parameters
          (map (lambda (p)
                 (if (assq 'required p) p
                     (append p `((required . ,(not (assq 'default p)))))))
               parameters))
    (let ((names (map (lambda (p) (assq-ref p 'name)) parameters)))
      (unless (or (null? arguments) (equal? arguments names))
        (error "arguments disagree with typed parameters" name))
      (set! arguments names)))
  (unless (symbol? category) (error "command category required" name))
  (unless (and (string? summary) (not (string-null? summary)))
    (error "command summary required" name))
  (unless (and (string? documentation) (not (string-null? documentation)))
    (error "command documentation required" name))
  (unless (bounded-datum? (list name arguments category summary documentation
                               demo-id result-schema scope effects preconditions
                               completion retry examples))
    (error "command metadata must be bounded readable data" name))
  (unless (list? examples) (error "command examples must be a list" name))
  (when (hash-ref %commands name)
    (error "duplicate command" name))
  (let ((command (make-command name procedure arguments category summary
                               documentation demo-id parameters result-schema
                               scope effects preconditions completion retry
                               examples)))
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
  "Invoke NAME positionally; typed commands validate all supplied values and
fill omitted trailing defaults before calling their procedure."
  (let ((command (command-ref name)))
    (unless command (error "unknown command" name))
    (if (command-typed? command)
        (begin
          (when (> (length arguments) (length (command-arguments command)))
            (argument-error name 'too-many-arguments #f))
          (invoke-command/named name
            (map cons (take (command-arguments command) (length arguments))
                      arguments)))
        (begin
          (unless (= (length arguments) (length (command-arguments command)))
            (error "wrong command argument count" name
                   (length (command-arguments command)) (length arguments)))
          (apply (command-procedure command) arguments)))))

;; Bound traversal independently from the printer. Depth limits stop cycles;
;; a shared node budget also bounds wide lists. Opaque objects never reach
;; discovery output, even inside a user-defined default or example.
(define (bounded-datum? datum)
  (let ((remaining 16384))
    (let visit ((value datum) (depth 0))
      (set! remaining (- remaining 1))
      (and (>= remaining 0) (< depth 128)
           (cond
            ((pair? value)
             (and (visit (car value) (+ depth 1))
                  (visit (cdr value) depth)))
            ((vector? value)
             (and (<= (vector-length value) remaining)
                  (every (lambda (v) (visit v (+ depth 1)))
                         (vector->list value))))
            ((string? value) (<= (string-length value) 65536))
            ((number? value) (and (real? value) (finite? value)))
            (else (or (null? value) (boolean? value) (symbol? value)
                      (keyword? value) (char? value))))))))

(define %schema-types '(any boolean integer number string symbol list alist))

(define (schema-type? type)
  (or (memq type %schema-types)
      (and (list? type) (pair? type)
           (case (car type)
             ((enum) (pair? (cdr type)))
             ((maybe list-of)
              (and (= (length type) 2) (schema-type? (cadr type))))
             (else #f)))))

(define (unique-alist? value)
  (and (list? value)
       (every (lambda (entry) (and (pair? entry) (symbol? (car entry)))) value)
       (= (length value) (length (delete-duplicates (map car value))))))

(define (schema-value-valid? type value)
  "Whether VALUE is bounded readable data matching TYPE. Primitive types are
any, boolean, integer (exact), number (finite real), string, symbol, list,
and alist (unique symbol keys). Compound types are (enum VALUE ...),
(maybe TYPE), accepting #f, and (list-of TYPE)."
  (and (bounded-datum? type) (schema-type? type) (bounded-datum? value)
       (let match ((type type) (value value))
         (case type
           ((any) #t)
           ((boolean) (boolean? value))
           ((integer) (exact-integer? value))
           ((number) (and (real? value) (finite? value)))
           ((string) (string? value))
           ((symbol) (symbol? value))
           ((list) (list? value))
           ((alist) (unique-alist? value))
           (else
            (case (car type)
              ((enum) (and (member value (cdr type)) #t))
              ((maybe) (or (eq? value #f) (match (cadr type) value)))
              ((list-of) (and (list? value)
                              (every (lambda (v) (match (cadr type) v)) value)))
              (else #f)))))))

(define (parameter-value-valid? parameter value)
  (and (schema-value-valid? (assq-ref parameter 'type) value)
       (or (not (assq 'enum parameter))
           (member value (assq-ref parameter 'enum)))
       (every
        (lambda (constraint)
          (let ((entry (assq (car constraint) parameter)))
            (or (not entry) ((cdr constraint) value (cdr entry)))))
        (list
         (cons 'minimum (lambda (v n) (and (real? v) (>= v n))))
         (cons 'maximum (lambda (v n) (and (real? v) (<= v n))))
         (cons 'min-length (lambda (v n)
                             (cond ((string? v) (>= (string-length v) n))
                                   ((list? v) (>= (length v) n))
                                   (else #f))))
         (cons 'max-length (lambda (v n)
                             (cond ((string? v) (<= (string-length v) n))
                                   ((list? v) (<= (length v) n))
                                   (else #f))))))))

(define (validate-parameter-schemas! name parameters)
  (unless (and (bounded-datum? parameters) (list? parameters))
    (error "parameters must be bounded schema data" name))
  (let ((names '()))
    (for-each
     (lambda (parameter)
       (unless (and (unique-alist? parameter)
                    (every (lambda (entry)
                             (memq (car entry)
                                   '(name type required default enum minimum
                                          maximum min-length max-length description)))
                           parameter)
                    (symbol? (assq-ref parameter 'name))
                    (schema-type? (assq-ref parameter 'type)))
         (error "invalid parameter schema" name parameter))
       (let* ((parameter-name (assq-ref parameter 'name))
              (default (assq 'default parameter))
              (required (assq 'required parameter)))
         (when (memq parameter-name names)
           (error "duplicate parameter name" name parameter-name))
         (set! names (cons parameter-name names))
         (when (and (or (assq 'minimum parameter) (assq 'maximum parameter))
                    (not (memq (assq-ref parameter 'type) '(integer number))))
           (error "numeric constraints require integer or number type" name parameter-name))
         (when (and (or (assq 'min-length parameter) (assq 'max-length parameter))
                    (not (or (memq (assq-ref parameter 'type) '(string list alist))
                             (and (pair? (assq-ref parameter 'type))
                                  (eq? (car (assq-ref parameter 'type)) 'list-of)))))
           (error "length constraints require string or list type" name parameter-name))
         (when (and required
                    (or (not (boolean? (cdr required)))
                        (and (cdr required) default)
                        (and (not (cdr required)) (not default))))
           (error "optional parameters require defaults; required ones cannot have defaults"
                  name parameter-name))
         (for-each
          (lambda (key)
            (let ((entry (assq key parameter)))
              (when (and entry
                         (not (and (real? (cdr entry)) (finite? (cdr entry)))))
                (error "numeric constraint required" name parameter-name key))))
          '(minimum maximum))
         (for-each
          (lambda (key)
            (let ((entry (assq key parameter)))
              (when (and entry
                         (not (and (exact-integer? (cdr entry)) (>= (cdr entry) 0))))
                (error "nonnegative length constraint required" name parameter-name key))))
          '(min-length max-length))
         (for-each
          (lambda (keys)
            (let ((low (assq (car keys) parameter))
                  (high (assq (cdr keys) parameter)))
              (when (and low high (> (cdr low) (cdr high)))
                (error "inverted parameter constraints" name parameter-name))))
          '((minimum . maximum) (min-length . max-length)))
         (let ((enum (assq 'enum parameter)))
           (when (and enum
                      (not (and (list? (cdr enum)) (pair? (cdr enum))
                                (every (lambda (value)
                                         (parameter-value-valid? parameter value))
                                       (cdr enum)))))
             (error "enum choices must satisfy their schema" name parameter-name)))
         (when (and (assq 'description parameter)
                    (not (string? (assq-ref parameter 'description))))
           (error "parameter description must be a string" name parameter-name))
         (when (and default (not (parameter-value-valid? parameter (cdr default))))
           (error "default does not satisfy parameter schema" name parameter-name))))
     parameters)))

(define (command-typed? command)
  "Whether COMMAND declares typed named parameters, including an empty list."
  (not (eq? #f (command-parameters command))))

(define (argument-error name reason parameter)
  (throw 'command-validation-error 'invalid-argument
         `((command . ,name) (reason . ,reason) (parameter . ,parameter))))

(define (validate-command-arguments name arguments)
  "Validate a named ARGUMENTS alist and return values in procedure order,
including defaults. No procedure is called. Failure throws
command-validation-error with stable CODE and readable DETAILS. Legacy
commands require their declared argument names but cannot validate types."
  (let ((command (command-ref name)))
    (unless command
      (throw 'command-validation-error 'unknown-command `((command . ,name))))
    (unless (and (bounded-datum? arguments) (unique-alist? arguments))
      (argument-error name 'malformed-arguments #f))
    (for-each
     (lambda (entry)
       (unless (memq (car entry) (command-arguments command))
         (argument-error name 'unknown-parameter (car entry))))
     arguments)
    (map
     (lambda (parameter-name)
       (let* ((parameter (and (command-typed? command)
                              (find (lambda (p) (eq? (assq-ref p 'name) parameter-name))
                                    (command-parameters command))))
              (given (assq parameter-name arguments))
              (default (and parameter (assq 'default parameter)))
              (value (cond (given (cdr given))
                           (default (cdr default))
                           (else (argument-error name 'missing-parameter parameter-name)))))
         (unless (or (not parameter) (parameter-value-valid? parameter value))
           (argument-error name 'type-or-constraint parameter-name))
         value))
     (command-arguments command))))

(define (invoke-command/named name arguments)
  "Invoke NAME with a validated named ARGUMENTS alist and applied defaults."
  (let ((values (validate-command-arguments name arguments)))
    (apply (command-procedure (command-ref name)) values)))

(define (action-summary command)
  `((name . ,(command-name command))
    (category . ,(command-category command))
    (summary . ,(command-summary command))
    (typed? . ,(command-typed? command))
    (scope . ,(command-scope command))
    (effects . ,(command-effects command))))

(define (describe-action name)
  "Return NAME's full readable command contract, or #f if unregistered.
Missing legacy metadata is explicitly unknown; PARAMETERS is #f for legacy
commands. EXAMPLES are Scheme call data, never strings to evaluate."
  (let ((command (command-ref name)))
    (and command
         (append (action-summary command)
                 `((arguments . ,(command-arguments command))
                   (parameters . ,(command-parameters command))
                   (result-schema . ,(command-result-schema command))
                   (preconditions . ,(command-preconditions command))
                   (completion . ,(command-completion command))
                   (retry . ,(command-retry command))
                   (documentation . ,(command-documentation command))
                   (examples . ,(command-examples command)))))))

(define* (search-actions #:optional (query ""))
  "Return concise action summaries matching QUERY in name, category, or
summary, case-insensitively. Empty QUERY returns all registered actions."
  (unless (string? query) (error "search query must be a string" query))
  (let ((needle (string-downcase query)))
    (filter-map
     (lambda (name)
       (let ((command (command-ref name)))
         (and (any (lambda (text) (string-contains (string-downcase text) needle))
                   (list (symbol->string name)
                         (symbol->string (command-category command))
                         (command-summary command)))
              (action-summary command))))
     (command-names))))

(define (capabilities)
  "Return the versioned command schema capabilities and concise action list."
  `((schema-version . 1)
    (representation . scheme)
    (schema-types . ,%schema-types)
    (schema-constructors . (enum maybe list-of))
    (actions . ,(search-actions))))
