;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Self-describing IPC reply envelope: error payloads and the
;;; writable-data guarantee (scheme/ipc-reply.scm).

(use-modules (system vm trace)
             (srfi srfi-9)
             (srfi srfi-9 gnu)
             (minde commands)
             (minde command-catalog))

;; wm-log is a Rust gsubr at runtime; stub it so the envelope loads headlessly.
(define %log '())
(define (wm-log message) (set! %log (cons message %log)) #t)

(load-from-path "ipc-reply.scm")

(define failures 0)
(define (check description value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" description)))

;; A reply string must always be exactly one datum that reads back cleanly.
(define (round-trip reply)
  (check "reply respects output budget"
         (<= (string-length reply) %ipc-reply-max-chars))
  (call-with-input-string reply
    (lambda (port)
      (let ((datum (read port)))
        (check "reply is a single datum" (eof-object? (read port)))
        datum))))

;; --- Success replies -------------------------------------------------------

(let ((datum (round-trip (minde-ipc-eval "(list 1 2 (quote sym) \"str\")"))))
  (check "success is tagged ok" (eq? (car datum) 'ok))
  (check "success carries the result" (equal? (cadr datum) '(1 2 sym "str"))))

;; --- Error replies carry key, args, message and bounded backtrace ----------

(let ((datum (round-trip (minde-ipc-eval "(error \"boom\" 42)"))))
  (check "error is tagged error" (eq? (car datum) 'error))
  (check "error preserves five fields and appends metadata" (= (length datum) 6))
  (check "error key is a symbol" (symbol? (cadr datum)))
  (check "error message is a non-empty string"
         (and (string? (list-ref datum 3))
              (not (string-null? (list-ref datum 3)))))
  (check "error message mentions the condition"
         (string-contains (list-ref datum 3) "boom"))
  (check "error backtrace is a string" (string? (list-ref datum 4)))
  (check "error backtrace is bounded"
         (<= (string-length (list-ref datum 4)) %ipc-backtrace-max-chars)))

(define (error-metadata datum) (list-ref datum 5))
(define (check-stage description datum stage started? completed?)
  (check description
         (and (eq? (car datum) 'error)
              (eq? (assq-ref (error-metadata datum) 'stage) stage)
              (eq? (assq-ref (error-metadata datum) 'execution-started?) started?)
              (eq? (assq-ref (error-metadata datum) 'execution-completed?) completed?)
              (eq? (assq-ref (error-metadata datum) 'effects-may-have-occurred?)
                   started?))))

;; A read error (more than one datum) is also reported, not crashed on.
(let ((datum (round-trip (minde-ipc-eval "1 2"))))
  (check "surplus input is an error" (eq? (car datum) 'error))
  (check "surplus-input message present" (string? (list-ref datum 3)))
  (check-stage "surplus input is rejected before execution" datum 'validation #f #f))

(for-each
 (lambda (source)
   (check-stage "malformed or absent datum fails validation"
                (round-trip (minde-ipc-eval source)) 'validation #f #f))
 '("" "; comment only" "(" "\"unterminated"))

;; Parsing the entire request must precede any evaluation.
(define %side-effects 0)
(check-stage "surplus datum prevents all execution"
             (round-trip (minde-ipc-eval "(set! %side-effects 1) 2"))
             'validation #f #f)
(check "validation did not mutate state" (= %side-effects 0))
(with-fluids ((read-eval? #t))
  (check-stage "IPC disables reader evaluation even when enabled by configuration"
               (round-trip (minde-ipc-eval
                            "#.(begin (set! %side-effects 99) 1) 2"))
               'validation #f #f))
(check "reader evaluation did not mutate state" (= %side-effects 0))

;; Both original regressions: text that resembles an opaque printer token is
;; ordinary data, and a successful mutation with no return value stays success.
(check "literal opaque marker in text is data"
       (equal? (round-trip (minde-ipc-eval "\"literal #<text> — λ 🪟\""))
               '(ok "literal #<text> — λ 🪟")))
(check "unspecified mutation result is explicit success"
       (equal? (round-trip (minde-ipc-eval "(set! %side-effects 2)"))
               '(ok (unspecified))))
(check "successful unspecified mutation occurred" (= %side-effects 2))
(check "nested unspecified values normalize too"
       (equal? (round-trip (minde-ipc-eval "(list (if #f #t))"))
               '(ok ((unspecified)))))

(let ((datum (round-trip
              (minde-ipc-eval
               "(begin (set! %side-effects 3) (current-output-port))"))))
  (check "mutation occurred before unsupported return" (= %side-effects 3))
  (check-stage "encoding error reports completed evaluation and possible effects"
               datum 'result-encoding #t #t))
(let ((datum (round-trip
              (minde-ipc-eval
               "(begin (set! %side-effects 4) (error \"after mutation\"))"))))
  (check "mutation occurred before execution exception" (= %side-effects 4))
  (check-stage "execution error reports possible effects"
               datum 'execution #t #f))

;; --- Writable-data guarantee ----------------------------------------------

;; A value that prints as #<...> must never leak into an ok reply.
(let ((datum (round-trip (minde-ipc-eval "(current-output-port)"))))
  (check "unreadable result becomes an error" (eq? (car datum) 'error))
  (check "unreadable result is flagged" (eq? (cadr datum) 'unreadable-result)))

;; The guarantee covers error replies too: throw ARGS carrying a live object
;; (here a wrong-type-arg irritant that prints as #<procedure ...>) must be
;; sanitized, keeping the whole error datum re-readable.
(let ((datum (round-trip (minde-ipc-eval "(car car)"))))
  (check "error with live irritant is still an error" (eq? (car datum) 'error))
  ;; The datum must survive a second write/read cycle unchanged: live objects
  ;; were replaced by strings, so nothing prints as a raw #<...> token.
  (check "error datum with live irritant round-trips"
         (equal? datum
                 (call-with-input-string
                  (call-with-output-string (lambda (p) (write datum p)))
                  read))))

;; ipc-ok-reply classifies readable vs unreadable values directly.
(check "plain data is ok" (eq? (car (ipc-ok-reply '(a 1 "b"))) 'ok))
(check "procedure is rejected" (eq? (car (ipc-ok-reply car)) 'error))

;; Opaque values must be rejected by type, without running their custom printer.
(define-record-type <dangerous-printer>
  (make-dangerous-printer) dangerous-printer?)
(define %printer-ran? #f)
(set-record-type-printer!
 <dangerous-printer>
 (lambda (_ port)
   (set! %printer-ran? #t)
   (display "misleading-readable-data" port)))
(check "custom printer cannot disguise an unsupported object"
       (eq? (cadr (ipc-ok-reply (make-dangerous-printer))) 'unreadable-result))
(check "custom printer was not invoked" (not %printer-ran?))

(let ((pair (list 'loop)) (vector (make-vector 1)))
  (set-cdr! pair pair)
  (vector-set! vector 0 vector)
  (for-each
   (lambda (value)
     (let ((datum (round-trip (ipc-ok-reply-string value))))
       (check "cyclic compound result is rejected" (eq? (cadr datum) 'cyclic-result))
       (check-stage "cycle is a result-encoding failure" datum 'result-encoding #t #t)))
   (list pair vector))
  (check "cyclic exception irritants remain readable"
         (eq? (car (round-trip (minde-ipc-eval
                               "(let ((p (list 1))) (set-cdr! p p) (throw 'oops p))")))
              'error)))

(let* ((shared (list 'same 'subtree))
       (value (list shared shared)))
  (check "shared acyclic values are accepted" (equal? (ipc-ok-reply value) (list 'ok value))))

;; Enforce traversal and output budgets without cutting wire syntax in half.
(for-each
 (lambda (value)
   (let ((datum (round-trip (ipc-ok-reply-string value))))
     (check "oversized data has a stable error code" (eq? (cadr datum) 'result-too-large))
     (check-stage "oversized result reports encoding stage" datum 'result-encoding #t #t)))
 (list (make-string (+ 1 %ipc-reply-max-chars) #\x)
       (make-string (quotient %ipc-reply-max-chars 2) #\")
       (make-vector %ipc-data-max-nodes #f)
       (make-list %ipc-data-max-nodes #f)
       (expt 2 %ipc-reply-max-chars)))
(let ((deep (let loop ((n (+ 2 %ipc-data-max-depth)))
              (if (zero? n) 'leaf (list (loop (- n 1)))))))
  (check "excessive nesting is rejected"
         (eq? (cadr (round-trip (ipc-ok-reply-string deep))) 'result-too-deep)))
(let ((datum (round-trip
              (minde-ipc-eval
               "(throw 'huge #f (make-string 10000 #\\x) '())"))))
  (check "error message is bounded"
         (<= (string-length (list-ref datum 3)) %ipc-message-max-chars)))
(let ((datum (round-trip
              (minde-ipc-eval
               "(throw 'huge #f \"large irritants\" (list (make-string 262144 #\\\")))"))))
  (check-stage "oversized error retains execution metadata" datum 'execution #t #f))

(let ((saved-log wm-log))
  (dynamic-wind
    (lambda () (set! wm-log (lambda (_) (error "logger failed"))))
    (lambda ()
      (check-stage "logger failure cannot obscure the evaluation error"
                   (round-trip (minde-ipc-eval "(error \"original error\")"))
                   'execution #t #f))
    (lambda () (set! wm-log saved-log))))

;; Every ok reply round-trips through write/read and equals the input.
(for-each
 (lambda (value)
   (let* ((reply (ipc-ok-reply value))
          (text (call-with-output-string (lambda (p) (write reply p))))
          (back (call-with-input-string text read)))
     (check (format #f "round-trips: ~s" value)
            (and (eq? (car back) 'ok) (equal? (cadr back) value)))))
 (list '() 'sym "" "string" 42 -1.5 1/3 1+2i #t #f #\λ #:keyword
       '#(1 "vector") #vu8(0 1 255) '(1 (2 3) "x") '((a . 1) (b . 2))))

;; --- Catalog surface: metadata queries return writable data ----------------
;; Iterate the built-in command catalog and verify that introspecting each
;; command through the real IPC reply path yields ok, re-readable data.

(register-builtin-command-schemas!)
(let ((names (command-names)))
  (check "catalog is non-empty" (> (length names) 0))
  (for-each
   (lambda (name)
     (let* ((expr (format #f "(map (lambda (c) (list (command-summary c) (command-category c) (command-arguments c))) (list (command-ref '~a)))" name))
            (datum (round-trip (minde-ipc-eval expr))))
       (check (format #f "catalog metadata ok for ~a" name)
              (eq? (car datum) 'ok))))
   names))

(if (zero? failures)
    (format #t "ipc-reply-test: all checks passed~%")
    (begin
      (format #t "ipc-reply-test: ~a failure(s)~%" failures)
      (exit 1)))
