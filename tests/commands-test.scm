;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Validate registry contracts before a procedure can change state.
(use-modules (minde commands) (srfi srfi-1))

(define failures 0)
(define (check description value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" description)))
(define (rejects? thunk)
  (catch #t (lambda () (thunk) #f) (lambda args #t)))
(define (validation-code thunk)
  (catch 'command-validation-error
    (lambda () (thunk) #f)
    (lambda (key code details) code)))
(define (roundtrips? value)
  (equal? value
          (call-with-input-string
              (call-with-output-string (lambda (p) (write value p))) read)))

(clear-command-registry!)
(define mutations 0)
(register-command! 'legacy! (lambda (x) (set! mutations (+ mutations 1)) x)
                   #:arguments '(x) #:category 'test #:summary "Legacy command")
(check "legacy positional call unchanged" (eq? 'yes (invoke-command 'legacy! 'yes)))
(check "legacy arity still enforced" (rejects? (lambda () (invoke-command 'legacy!))))
(check "legacy metadata remains unknown"
       (let ((help (describe-action 'legacy!)))
         (and (not (assq-ref help 'typed?))
              (not (assq-ref help 'parameters))
              (eq? 'unknown (assq-ref help 'result-schema))
              (eq? 'unknown (assq-ref help 'effects)))))

(register-command!
 'typed! (lambda (window enabled direction title)
           (set! mutations (+ mutations 1))
           (list window enabled direction title))
 #:parameters '(((name . window) (type . integer) (minimum . 1))
                ((name . enabled) (type . boolean) (default . #f))
                ((name . direction) (type . (enum left right)) (default . left))
                ((name . title) (type . string) (min-length . 1)
                 (max-length . 40) (default . "hello #< λ")))
 #:category 'test #:summary "Change a window"
 #:result-schema '(list integer boolean symbol string)
 #:scope '(window explicit) #:effects '(focus geometry)
 #:preconditions '(window-exists) #:completion 'synchronous #:retry 'receipt
 #:examples '((invoke-command/named 'typed! '((window . 5)))))

(check "typed named values are reordered and defaults filled"
       (equal? '(5 #t right "hello #< λ")
               (invoke-command/named 'typed!
                                     '((direction . right) (enabled . #t) (window . 5)))))
(check "typed positional call can omit trailing defaults"
       (equal? '(5 #f left "hello #< λ") (invoke-command 'typed! 5)))
(check "argument names remain backward compatible"
       (equal? '(window enabled direction title)
               (command-arguments (command-ref 'typed!))))
(check "unknown command has stable validation code"
       (eq? 'unknown-command
            (validation-code (lambda () (invoke-command/named 'missing! '())))))

(define before mutations)
(for-each
 (lambda (arguments)
   (check (format #f "rejects invalid arguments ~s" arguments)
          (eq? 'invalid-argument
               (validation-code (lambda () (invoke-command/named 'typed! arguments))))))
 '(() ((window . 0)) ((window . 1.0)) ((window . "1"))
   ((window . 1) (enabled . 1)) ((window . 1) (direction . up))
   ((window . 1) (title . "")) ((window . 1) (unknown . #t))
   ((window . 1) (window . 2)) ((window . 1) . bad) (window 1)))
(check "no rejected named call changes state" (= mutations before))
(check "positional values receive type validation"
       (eq? 'invalid-argument
            (validation-code (lambda () (invoke-command 'typed! "1")))))
(check "excess positional values rejected"
       (eq? 'invalid-argument
            (validation-code (lambda () (invoke-command 'typed! 1 #t 'left "x" 9)))))
(check "pure validation does not invoke a procedure"
       (begin (validate-command-arguments 'typed! '((window . 1)))
              (= before mutations)))

(for-each
 (lambda (parameters)
   (check (format #f "rejects invalid schema ~s" parameters)
          (rejects? (lambda ()
                      (register-command! 'bad! list #:parameters parameters
                                         #:category 'test #:summary "Invalid")))))
 '((((name . x) (type . bogus)))
   (((name . x) (type . integer) (default . #f)))
   (((name . x) (type . integer) (required . #f)))
   (((name . x) (type . integer) (required . #t) (default . 1)))
   (((name . x) (type . integer) (required . maybe)))
   (((name . x) (type . integer) (minimum . 2) (maximum . 1)))
   (((name . x) (type . integer) (min-length . 2)))
   (((name . x) (type . string) (minimum . 2)))
   (((name . x) (type . string) (max-length . -1)))
   (((name . x) (type . integer) (enum . (1 "two"))))
   (((name . x) (type . integer) (enum . ())))
   (((name . x) (type . integer) (unexpected . #t)))
   (((name . x) (type . integer)) ((name . x) (type . string)))
   (((name . x) (type . integer) (type . string)))
   (((name . x) (type . (enum))))
   (((name . x) (type . (maybe integer string))))))
(check "schema failures do not register the command" (not (command-ref 'bad!)))
(check "opaque defaults rejected at registration"
       (rejects? (lambda ()
                   (register-command! 'bad! list #:category 'test #:summary "Bad"
                                      #:parameters `(((name . x) (type . any)
                                                       (default . ,list)))))))
(check "opaque metadata rejected at registration"
       (rejects? (lambda ()
                   (register-command! 'bad! list #:category 'test #:summary "Bad"
                                      #:result-schema list))))
(check "conflicting legacy argument names rejected"
       (rejects? (lambda ()
                   (register-command! 'bad! list #:category 'test #:summary "Bad"
                                      #:arguments '(y)
                                      #:parameters '(((name . x) (type . any)))))))

(register-command! 'empty! (lambda () 'empty) #:parameters '()
                   #:category 'test #:summary "A typed zero-argument command")
(check "empty typed parameters differ from legacy metadata"
       (and (command-typed? (command-ref 'empty!))
            (null? (command-parameters (command-ref 'empty!)))
            (eq? 'empty (invoke-command/named 'empty! '()))))
(check "schema compound types validate recursive values"
       (and (schema-value-valid? '(list-of (maybe integer)) '(1 #f 3))
            (not (schema-value-valid? '(list-of integer) '(1 #f 3)))
            (schema-value-valid? 'alist '((x . 1) (y . #f)))
            (not (schema-value-valid? 'alist '((x . 1) (x . 2))))
            (not (schema-value-valid? 'number +nan.0))
            (not (schema-value-valid? 'number +inf.0))))
(let ((cycle (list 'x)))
  (set-cdr! cycle cycle)
  (for-each (lambda (type)
              (check "cyclic values fail bounded validation"
                     (not (schema-value-valid? type cycle))))
            '(any list alist))
  (check "cyclic named arguments fail without hanging"
         (eq? 'invalid-argument
              (validation-code (lambda () (invoke-command/named 'typed! cycle))))))
(check "oversized strings fail bounded validation"
       (not (schema-value-valid? 'any (make-string 65537 #\x))))
(check "deep values fail bounded validation"
       (let loop ((value 'x) (depth 150))
         (if (= depth 0)
             (not (schema-value-valid? 'any value))
             (loop (list value) (- depth 1)))))

(check "typed help has complete readable metadata"
       (let ((help (describe-action 'typed!)))
         (and (roundtrips? help)
              (every (lambda (key) (assq key help))
                     '(arguments parameters result-schema scope effects
                                 preconditions completion retry examples))
              (assq-ref (car (assq-ref help 'parameters)) 'required)
              (not (assq-ref (cadr (assq-ref help 'parameters)) 'required)))))
(check "unknown help returns false" (not (describe-action 'missing!)))
(check "discovery searches summaries case-insensitively"
       (equal? '(typed!) (map (lambda (entry) (assq-ref entry 'name))
                             (search-actions "WINDOW"))))
(check "summary discovery excludes verbose fields"
       (every (lambda (entry) (not (assq 'documentation entry))) (search-actions)))
(check "capabilities include registered user commands and version"
       (let ((summary (capabilities)))
         (and (= 1 (assq-ref summary 'schema-version))
              (equal? (command-names)
                      (map (lambda (entry) (assq-ref entry 'name))
                           (assq-ref summary 'actions)))
              (roundtrips? summary))))

(if (zero? failures)
    (format #t "commands-test: all checks passed~%")
    (exit 1))
