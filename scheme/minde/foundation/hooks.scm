;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Dependency-free named hook registries.

(define-module (minde foundation hooks)
  #:use-module (srfi srfi-1)
  #:export (make-hook-registry event-hook-procedures
            add-hook! remove-hook! run-hook!))

(define (make-hook-registry) (cons 'hook-registry '()))

(define (event-hook-procedures registry name)
  (or (assq-ref (cdr registry) name) '()))

(define (add-hook! registry name procedure)
  (set-cdr! registry
            (assq-set! (cdr registry) name
                       (cons procedure (event-hook-procedures registry name)))))

(define (remove-hook! registry name procedure)
  (set-cdr! registry
            (assq-set! (cdr registry) name
                       (delq procedure (event-hook-procedures registry name)))))

(define (run-hook! registry name on-error . arguments)
  (for-each
   (lambda (procedure)
     (catch #t
       (lambda () (apply procedure arguments))
       (lambda error (apply on-error error))))
   (event-hook-procedures registry name)))
