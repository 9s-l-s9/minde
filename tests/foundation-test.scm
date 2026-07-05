;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (minde foundation geometry)
             (minde foundation tree)
             (minde foundation serialization)
             ((minde foundation hooks) #:prefix hook:)
             (minde foundation keys))

(define failures 0)
(define (check name actual expected)
  (if (equal? actual expected)
      (format #t "ok - ~a~%" name)
      (begin (set! failures (+ failures 1))
             (format #t "FAIL - ~a: expected ~s, got ~s~%"
                     name expected actual))))

(check "rectangle union" (rect-union '((0 0 10 10) (10 0 5 20))) '(0 0 15 20))
(let* ((origin '(0 0 10 10))
       (wide '(10 2 10 6))
       (narrow '(10 8 10 2)))
  (check "directional neighbor prefers overlap"
         (directional-neighbor origin (list narrow wide) 'right) wide))

(let* ((tree (make-split 'horizontal 1/2 (make-leaf 'a) (make-leaf 'b)))
       (encoded (tree->sexp tree))
       (decoded (sexp->tree encoded)))
  (check "tree validates" (tree-valid? tree) #t)
  (check "tree leaf order" (map leaf-value (tree-leaves decoded)) '(a b))
  (check "tree serialization round trip" (tree->sexp decoded) encoded))

(check "datum string round trip"
       (string->datum (datum->string '(version 1 (a . b))))
       '(version 1 (a . b)))

(let ((path (string-append "/tmp/minde-serialization-"
                           (number->string (getpid)) ".scm")))
  (write-versioned-datum-file path 'minde-test 1 '(safe state))
  (check "versioned atomic file round trip"
         (read-versioned-datum-file path 'minde-test 1)
         '(safe state))
  ;; A leftover temporary file models interruption before rename: readers
  ;; continue to observe the last complete target.
  (call-with-output-file (string-append path ".tmp.interrupted")
    (lambda (port) (display "(truncated" port)))
  (check "interrupted write preserves previous state"
         (read-versioned-datum-file path 'minde-test 1)
         '(safe state))
  (check "wrong persistence version rejected"
         (catch #t
           (lambda () (read-versioned-datum-file path 'minde-test 2) #f)
           (lambda _ #t))
         #t)
  (delete-file path)
  (delete-file (string-append path ".tmp.interrupted")))

(let ((registry (hook:make-hook-registry)) (seen '()) (errors 0))
  (hook:add-hook! registry 'event (lambda (value) (set! seen (cons value seen))))
  (hook:add-hook! registry 'event (lambda (_) (error "expected")))
  (hook:run-hook! registry 'event (lambda _ (set! errors (+ errors 1))) 'payload)
  (check "hook survives callback error" seen '(payload))
  (check "hook reports callback error" errors 1))

(let ((registry (make-key-registry)))
  (register-key! registry '(ctrl alt) "x" 'command)
  (check "canonical key notation" (key-notation '(ctrl alt) "x") "C-M-x")
  (check "key registry lookup" (lookup-key registry 12 "x") 'command)
  (check "key collision rejected"
         (catch #t
           (lambda () (register-key! registry 12 "x" 'other) #f)
           (lambda _ #t))
         #t))

(if (zero? failures)
    (format #t "all foundation tests passed~%")
    (exit 1))
