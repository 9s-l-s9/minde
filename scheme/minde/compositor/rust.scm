;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; rust.scm -- cached lookup of the compositor's top-level procedures.
;;;
;;; The wm-* Rust subrs (and a few init.scm wrappers such as wm-run-after)
;;; are defined at the top level of whichever module loads the policy layer
;;; -- (guile-user) in both the real compositor (scheme/init.scm, loaded via
;;; scm_c_primitive_load) and the unit tests. A module created with
;;; define-module doesn't automatically see another module's top-level
;;; bindings, and #:use-module/#:select requires the binding to already
;;; exist at *compile time* of the importing file, which is too early (the
;;; caller hasn't defined its stubs/Rust hasn't registered its subrs yet
;;; when the modules are compiled). So the names are resolved dynamically
;;; at call time instead, exactly like the Rust side's own `guile::lookup`
;;; does for `wm-handle-key' -- a missing definition is simply a no-op.
;;;
;;; The lookup is cached per symbol. The cached object is the *variable*,
;;; not its value, so redefining a hook live (REPL, Print R) is still seen:
;;; a `define' on an existing top-level name reuses its variable. A name
;;; that was unbound is not cached, so a stub or subr defined later (unit
;;; tests, a newer binary) is found on the next call.

(define-module (minde compositor rust)
  #:export (rust-variable
            rust-bound?
            rust-call
            rust-call-if-bound))

;; symbol -> variable, for names resolved at least once.
(define %variables (make-hash-table))
(define %reported-missing-rust-calls (make-hash-table))

(define (rust-variable name)
  "Returns the (guile-user) variable bound to symbol NAME, or #f."
  (or (hashq-ref %variables name)
      (let* ((mod (resolve-module '(guile-user) #:ensure #f))
             (var (and mod (module-variable mod name))))
        (when var (hashq-set! %variables name var))
        var)))

(define (rust-bound? name)
  "Returns whether NAME is currently bound at the compositor's top level."
  (let ((var (rust-variable name)))
    (and var (variable-bound? var) #t)))

(define (rust-call name . args)
  "Applies the top-level procedure NAME to ARGS; returns #f when unbound.
Unit tests intentionally omit capabilities irrelevant to the behavior under
test, so each missing name is reported once: the signal stays visible
without burying real failures in repeated noise."
  (let ((var (rust-variable name)))
    (if var
        (apply (variable-ref var) args)
        (begin
          (unless (hash-ref %reported-missing-rust-calls name)
            (hash-set! %reported-missing-rust-calls name #t)
            (format #t "minde: ~a unbound, ignoring call~%" name))
          #f))))

(define (rust-call-if-bound name . args)
  "Like rust-call, but a missing NAME is silently ignored (returns #f)."
  (let ((var (rust-variable name)))
    (and var (apply (variable-ref var) args))))
