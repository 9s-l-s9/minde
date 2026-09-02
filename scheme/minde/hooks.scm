;;; hooks.scm -- user-extensible event hooks, StumpWM's *xxx-hook* model.
;;;
;;; The compositor modules run hooks at interesting moments; user config
;;; extends them without touching module code:
;;;
;;;   (add-hook! 'focus-window (lambda (id) (wm-log ...)))
;;;
;;; Hooks fired by the bundled modules:
;;;   new-window (id title app-id)   -- window mapped and placed
;;;   destroy-window (id)            -- window unmapped
;;;   focus-window (id)              -- shown window changed (id or #f)
;;;   focus-frame (x y w h)          -- current frame changed
;;;   focus-group (name)             -- group switched
;;;   message (text)                 -- something was echoed
;;;   session-lock ()                -- the session-lock surface came up
;;;   session-unlock ()              -- the session-lock surface went away
;;;
;;; A hook procedure that throws is logged and dropped from that run --
;;; one bad hook must not break the event path (same policy as
;;; run-binding! in init.scm).

(define-module (minde hooks)
  #:use-module ((minde foundation hooks) #:prefix foundation:)
  #:export (add-event-hook!
            remove-event-hook!
            run-event-hook!
            event-hook-procedures))

;; name (symbol) -> list of procedures, most recently added first.
;; (Named with a trailing * to avoid colliding with Guile's own
;; add-hook!/run-hook, which operate on <hook> objects.)
(define %hooks (foundation:make-hook-registry))

(define (event-hook-procedures name)
  "Returns the compositor event-hook procedures registered for NAME."
  (foundation:event-hook-procedures %hooks name))

(define (add-event-hook! name proc)
  "Registers PROC to run when hook NAME fires."
  (foundation:add-hook! %hooks name proc))

(define (remove-event-hook! name proc)
  "Removes PROC from compositor event hook NAME."
  (foundation:remove-hook! %hooks name proc))

;; Variables looked up in guile-user, cached once found. A variable object
;; stays valid across live redefinition (`define' of an existing top-level
;; name sets the same variable), so the cache never goes stale; a name that
;; is not yet bound (this module loads before event-stream.scm and before
;; the Rust gsubrs in headless tests) is simply retried on the next firing.
(define %guile-user-cache (make-hash-table))

(define (guile-user-variable name)
  "Returns the guile-user variable object for NAME, or #f when unbound."
  (or (hashq-ref %guile-user-cache name)
      (let* ((mod (resolve-module '(guile-user) #:ensure #f))
             (var (and mod (module-variable mod name))))
        (when var (hashq-set! %guile-user-cache name var))
        var)))

(define (run-event-hook! name . args)
  "Runs every procedure registered on NAME with ARGS; errors are logged
via the wm-log subr (when present) and swallowed."
  ;; Mirror every firing to the read-only event push socket before running
  ;; user hooks. The mirror procedure (minde-mirror-event, defined in the
  ;; plain-loaded event-stream.scm) is resolved from guile-user so this module
  ;; stays decoupled and testable in isolation; a missing mirror or a throwing
  ;; one must never break the event path.
  (let ((var (guile-user-variable 'minde-mirror-event)))
    (when var
      (catch #t
        (lambda () ((variable-ref var) name args))
        (lambda _ #f))))
  (apply foundation:run-hook! %hooks name
         (lambda (key . eargs)
           (let ((var (guile-user-variable 'wm-log)))
             (when var
               ((variable-ref var)
                (format #f "error in ~a hook: ~a ~s" name key eargs)))))
         args))
