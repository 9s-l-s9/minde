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
;;;
;;; A hook procedure that throws is logged and dropped from that run --
;;; one bad hook must not break the event path (same policy as
;;; run-binding! in init.scm).

(define-module (minde hooks)
  #:use-module (srfi srfi-1)
  #:export (add-hook!*
            remove-hook!*
            run-hook!*
            hook-procedures))

;; name (symbol) -> list of procedures, most recently added first.
;; (Named with a trailing * to avoid colliding with Guile's own
;; add-hook!/run-hook, which operate on <hook> objects.)
(define %hooks '())

(define (hook-procedures name)
  (or (assq-ref %hooks name) '()))

(define (add-hook!* name proc)
  "Registers PROC to run when hook NAME fires."
  (set! %hooks (assq-set! %hooks name (cons proc (hook-procedures name)))))

(define (remove-hook!* name proc)
  (set! %hooks (assq-set! %hooks name (delq proc (hook-procedures name)))))

(define (run-hook!* name . args)
  "Runs every procedure registered on NAME with ARGS; errors are logged
via the wm-log subr (when present) and swallowed."
  (for-each
   (lambda (proc)
     (catch #t
       (lambda () (apply proc args))
       (lambda (key . eargs)
         (let* ((mod (resolve-module '(guile-user) #:ensure #f))
                (var (and mod (module-variable mod 'wm-log))))
           (when var
             ((variable-ref var)
              (format #f "error in ~a hook: ~a ~s" name key eargs)))))))
   (hook-procedures name)))
