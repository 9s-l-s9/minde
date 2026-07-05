;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Dependency-free named hook registries.

(define-module (minde foundation hooks)
  #:use-module (srfi srfi-1)
  #:export (make-hook-registry event-hook-procedures
            add-hook! remove-hook! run-hook!))

(define (make-hook-registry)
  "Returns an empty mutable named-hook registry."
  (cons 'hook-registry '()))

(define (event-hook-procedures registry name)
  "Returns the procedures registered for NAME, newest first."
  (or (assq-ref (cdr registry) name) '()))

(define (add-hook! registry name procedure)
  "Adds PROCEDURE to hook NAME in REGISTRY."
  (set-cdr! registry
            (assq-set! (cdr registry) name
                       (cons procedure (event-hook-procedures registry name)))))

(define (remove-hook! registry name procedure)
  "Removes PROCEDURE from hook NAME in REGISTRY."
  (set-cdr! registry
            (assq-set! (cdr registry) name
                       (delq procedure (event-hook-procedures registry name)))))

(define (run-hook! registry name on-error . arguments)
  "Runs hook NAME with ARGUMENTS, forwarding callback failures to ON-ERROR."
  (for-each
   (lambda (procedure)
     (catch #t
       (lambda () (apply procedure arguments))
       (lambda error (apply on-error error))))
   (event-hook-procedures registry name)))
