;;; SPDX-License-Identifier: GPL-3.0-or-later
;;;
;;; The compositor -> policy callback contract. Rust (src/guile/mod.rs,
;;; the `Hook` table) invokes these by looking the names up in the
;;; top-level environment, so a user configuration can override any of them
;;; by redefining the name. This module makes that contract explicit: the
;;; expected arity of every callback, a definer that checks a replacement
;;; against it, and a checker that reports missing or mis-arity definitions
;;; at startup and on every configuration reload instead of leaving Rust to
;;; treat a typo as "no policy".

(define-module (minde compositor callbacks)
  #:use-module (srfi srfi-1)
  #:export (compositor-callbacks
            compositor-callback-arities
            define-compositor-callback!
            check-compositor-callbacks!))

;; name -> number of arguments Rust passes. Keep in sync with the
;; `Hook::new` table and its `.call` sites in src/guile/mod.rs;
;; tests/callbacks-test.scm compares the two.
(define %arities
  '((minde-ipc-eval . 1)
    (publish-status! . 0)
    (wm-handle-key . 4)
    (handle-window-map! . 3)
    (handle-window-title-change! . 3)
    (handle-window-unmap! . 1)
    (handle-heads-change! . 1)
    (handle-output-geometry! . 4)
    (handle-timer! . 1)
    (handle-paste! . 1)
    (handle-window-move! . 5)
    (handle-urgent-window! . 1)
    (handle-foreign-activate! . 1)
    (handle-foreign-fullscreen! . 2)
    (handle-foreign-minimize! . 2)
    (output-configuration-allowed? . 0)
    (handle-output-configured! . 0)
    (handle-input-device-added! . 2)
    (handle-startup! . 0)
    (wm-on-session-lock . 0)
    (wm-on-session-unlock . 0)))

(define (compositor-callback-arities)
  "The (name . argument-count) contract Rust calls into."
  %arities)

(define (policy-module)
  (resolve-module '(guile-user)))

(define (accepts-arity? proc n)
  (let ((arity (procedure-minimum-arity proc)))
    (or (not arity)
        (let ((required (car arity))
              (optional (cadr arity))
              (rest? (caddr arity)))
          (and (<= required n)
               (or rest? (<= n (+ required optional))))))))

(define (callback-status name arity)
  (let ((var (module-variable (policy-module) name)))
    (cond
     ((or (not var) (not (variable-bound? var))) 'unbound)
     ((not (procedure? (variable-ref var))) 'not-a-procedure)
     ((accepts-arity? (variable-ref var) arity) 'ok)
     (else 'arity-mismatch))))

(define (compositor-callbacks)
  "One (name arity status) entry per callback, STATUS being ok, unbound,
not-a-procedure or arity-mismatch; the inspectable form of the contract."
  (map (lambda (entry)
         (list (car entry) (cdr entry)
               (callback-status (car entry) (cdr entry))))
       %arities))

(define (define-compositor-callback! name proc)
  "Installs PROC as the compositor callback NAME after checking that NAME
is part of the contract and PROC accepts the arguments Rust passes. The
checked way for a configuration to replace a hook."
  (let ((arity (assq-ref %arities name)))
    (unless arity
      (error "not a compositor callback" name))
    (unless (procedure? proc)
      (error "compositor callback must be a procedure" name proc))
    (unless (accepts-arity? proc arity)
      (error "compositor callback does not accept the arguments Rust passes"
             name arity))
    (module-define! (policy-module) name proc)
    name))

(define* (check-compositor-callbacks! #:optional (report #f))
  "Returns the callbacks whose status is not ok, as (name arity status)
entries, passing each one to REPORT when given. An unbound callback is a
deliberate option for some names (Rust treats it as a no-op), so this
warns rather than fails; an arity mismatch is always a bug."
  (let ((problems (filter (lambda (entry) (not (eq? (caddr entry) 'ok)))
                          (compositor-callbacks))))
    (when report (for-each report problems))
    problems))
