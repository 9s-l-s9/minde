;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Small deterministic fixtures shared by property-style Scheme tests.

(define-module (minde test-fixtures)
  #:export (check
            check-equal
            check-error
            deterministic-integers
            finish-tests))

(define %failures 0)

(define (check name value)
  (unless value
    (set! %failures (+ %failures 1))
    (format #t "FAIL - ~a~%" name)))

(define (check-equal name actual expected)
  (unless (equal? actual expected)
    (set! %failures (+ %failures 1))
    (format #t "FAIL - ~a: expected ~s, got ~s~%"
            name expected actual)))

(define (check-error name thunk)
  (check name
         (catch #t
           (lambda () (thunk) #f)
           (lambda _ #t))))

;; A fixed LCG makes failures reproducible without adding SRFI or Guile-QuickCheck
;; dependencies.  It is deliberately not suitable for security-sensitive use.
(define* (deterministic-integers count #:optional (seed 1729))
  (let loop ((remaining count) (state seed) (values '()))
    (if (zero? remaining)
        (reverse values)
        (let ((next (modulo (+ (* state 1103515245) 12345) 2147483648)))
          (loop (- remaining 1) next (cons next values))))))

(define (finish-tests suite)
  (if (zero? %failures)
      (format #t "all ~a tests passed~%" suite)
      (begin
        (format #t "~a ~a test(s) failed~%" suite %failures)
        (exit 1))))
