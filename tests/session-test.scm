;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; session-test.scm -- Guile-only unit test of logout!/lock-screen!/
;;; suspend! ((minde session)) and the wm-on-session-lock/unlock
;;; Rust-facing events defined in (minde groups).
;;;
;;; Run with:
;;;   guile -L scheme tests/session-test.scm
;;;
;;; Injects the four callbacks required by the module's explicit runtime
;;; boundary. The real Rust side is not built while this runs; lock state and
;;; timer delivery are controlled by the fakes below.

(use-modules (srfi srfi-1))

(define %spawned '())          ; commands passed to wm-spawn, newest first
(define %quit? #f)
(define %log-lines '())
(define %messages '())         ; echoed text, newest first
(define %timers '())           ; (ms . thunk), newest first, from wm-run-after

(define (wm-spawn cmd) (set! %spawned (cons cmd %spawned)) #t)
(define (wm-quit) (set! %quit? #t) #t)
(define (wm-log msg) (set! %log-lines (cons msg %log-lines)) #t)
(define (wm-message text . _) (set! %messages (cons text %messages)) #t)
;; Stands in for init.scm's own (define (wm-run-after ms thunk) ...) wrapper.
(define (wm-run-after ms thunk) (set! %timers (cons (cons ms thunk) %timers)) #t)
;; Privacy/input locking starts immediately; suspend readiness waits for
;; presentation confirmation. Keep both fakes distinct to exercise that gap.
(define %immediate-locked? #f)
(define %lock-confirmed? #f)
(define (wm-session-locked?) %immediate-locked?)
(define (wm-session-lock-confirmed?) %lock-confirmed?)

;; Now it's safe to load the modules under test.
(use-modules (minde compositor frames))
(use-modules (minde hooks))
(use-modules (minde groups))
(use-modules (minde ui prompt))
(use-modules (minde session))

;; Security-sensitive operations must not silently no-op before the runtime
;; boundary is registered.
(define %unconfigured-runtime-rejected?
  (catch #t
    (lambda () (lock-screen!) #f)
    (lambda _ #t)))

((@@ (minde session) configure-session-runtime!)
 #:spawn wm-spawn
 #:quit wm-quit
 #:run-after wm-run-after
 #:session-locked? wm-session-lock-confirmed?)

;; ---------------------------------------------------------------------
;; Tiny assertion helpers
;; ---------------------------------------------------------------------

(define %failures 0)

(define (check name got expected)
  (if (equal? got expected)
      (format #t "ok - ~a~%" name)
      (begin
        (set! %failures (+ %failures 1))
        (format #t "FAIL - ~a: expected ~s, got ~s~%" name expected got))))

(define (spawned? cmd) (and (member cmd %spawned) #t))
(define (echoed-containing? substring)
  (and (any (lambda (m) (string-contains m substring)) %messages) #t))

(check "unconfigured session runtime is rejected"
       %unconfigured-runtime-rejected? #t)

;; ---------------------------------------------------------------------
;; logout!: prompts, only "y"/"yes" actually quits, and it's the exact
;; same procedure the Print Q and portable "s q" bindings call, so
;; testing it here covers both.
;; ---------------------------------------------------------------------

(logout!)
(check "logout! opens a confirmation prompt" (input-active?) #t)
(input-handle-key! 0 "n" "n")
(input-handle-key! 0 "Return" "")
(check "answering n does not quit" %quit? #f)

(logout!)
(input-handle-key! 0 "y" "y")
(input-handle-key! 0 "Return" "")
(check "answering y quits" %quit? #t)
(set! %quit? #f)

;; ---------------------------------------------------------------------
;; lock-screen!: spawns %lock-command, warns (best-effort) when it isn't
;; on PATH, but never errors either way.
;; ---------------------------------------------------------------------

(set! %lock-command "true") ; near-universally on PATH
(set! %spawned '())
(set! %messages '())
(lock-screen!)
(check "lock-screen! spawns %lock-command" (spawned? "true") #t)
(check "lock-screen! does not warn about a present program"
       (echoed-containing? "not found on PATH") #f)

(set! %lock-command "definitely-not-a-real-minde-binary-xyz")
(set! %spawned '())
(set! %messages '())
(lock-screen!)
(check "lock-screen! still spawns a missing program (never errors)"
       (spawned? "definitely-not-a-real-minde-binary-xyz") #t)
(check "lock-screen! warns about a missing program"
       (echoed-containing? "not found on PATH") #t)

(set! %lock-command "true") ; restore for the suspend! tests below

;; ---------------------------------------------------------------------
;; suspend!: default %lock-on-suspend? #t locks first and waits for
;; wm-on-session-lock before spawning %suspend-command.
;; ---------------------------------------------------------------------

(set! %suspend-command "false")

(set! %spawned '())
(set! %timers '())
(suspend!)
(check "suspend! locks first" (spawned? "true") #t)
(check "suspend! does not suspend before the lock confirms"
       (spawned? "false") #f)
(check "suspend! armed a timeout fallback" (= (length %timers) 1) #t)

(wm-on-session-lock)
(check "suspend! suspends once the lock is confirmed" (spawned? "false") #t)

(set! %spawned '())
(wm-on-session-lock) ; a later relock must not re-trigger a stale suspend
(check "a later session-lock event does not re-trigger suspend"
       (spawned? "false") #f)

;; ---------------------------------------------------------------------
;; suspend!: fails closed when the lock never confirms in time.
;; ---------------------------------------------------------------------

(set! %spawned '())
(set! %timers '())
(set! %messages '())
(suspend!)
(check "suspend! locked again" (spawned? "true") #t)
(for-each (lambda (timer) ((cdr timer))) %timers) ; fire the armed timeout
(check "suspend! does not suspend when the lock never confirms"
       (spawned? "false") #f)
(check "suspend! echoes a cancellation message on timeout"
       (echoed-containing? "cancelled") #t)

(set! %spawned '())
(wm-on-session-lock) ; a confirmation arriving after the timeout is too late
(check "a late lock confirmation after timeout does not suspend"
       (spawned? "false") #f)

;; ---------------------------------------------------------------------
;; suspend!: %lock-on-suspend? #f suspends immediately, no lock.
;; ---------------------------------------------------------------------

(set! %lock-on-suspend? #f)
(set! %spawned '())
(suspend!)
(check "suspend! skips locking when %lock-on-suspend? is #f"
       (spawned? "false") #t)
(check "suspend! does not lock when %lock-on-suspend? is #f"
       (spawned? "true") #f)
(set! %lock-on-suspend? #t)

;; ---------------------------------------------------------------------
;; suspend!: a second call while one is already pending is a no-op echo,
;; not a second race with a duplicated hook.
;; ---------------------------------------------------------------------

(set! %spawned '())
(set! %timers '())
(set! %messages '())
(suspend!)
(suspend!)
(check "a second suspend! while pending does not lock twice"
       (length (filter (lambda (c) (string=? c "true")) %spawned)) 1)
(check "a second suspend! while pending echoes a hint"
       (echoed-containing? "already pending") #t)
(wm-on-session-lock)
(check "the pending suspend still completes exactly once"
       (length (filter (lambda (c) (string=? c "false")) %spawned)) 1)

;; A completed attempt's timer can arrive during a later attempt (e.g. a
;; quick resume and re-suspend). It must only cancel the attempt that armed it.
(set! %timers '())
(suspend!)
(define first-suspend-timeout (cdar %timers))
(wm-on-session-lock)
(wm-on-session-unlock)
(set! %spawned '())
(set! %messages '())
(suspend!)
(first-suspend-timeout)
(check "an earlier successful suspend's timeout does not cancel a new attempt"
       (echoed-containing? "cancelled") #f)
(wm-on-session-lock)
(check "the new suspend still completes after a stale timeout"
       (length (filter (lambda (c) (string=? c "false")) %spawned)) 1)
((cdar %timers))
(check "a timeout after confirmation does not suspend again"
       (length (filter (lambda (c) (string=? c "false")) %spawned)) 1)

(set! %timers '())
(set! %spawned '())
(set! %messages '())
(suspend!)
(first-suspend-timeout)
(check "an ignored stale timeout leaves the current attempt armed"
       (echoed-containing? "cancelled") #f)
((cdar %timers))
(check "the current attempt's timeout still cancels after a stale timeout"
       (echoed-containing? "cancelled") #t)
(check "the current timeout never runs the suspend command"
       (spawned? "false") #f)
(wm-on-session-lock)
(check "confirmation after the current timeout does not suspend"
       (spawned? "false") #f)

;; ---------------------------------------------------------------------
;; An accepted lock protects input/privacy before its first locked frame is
;; presented. That immediate guard must not take suspend!'s confirmed shortcut.
;; ---------------------------------------------------------------------

(set! %immediate-locked? #t)
(set! %lock-confirmed? #f)
(set! %spawned '())
(set! %timers '())
(suspend!)
(check "an immediately locked but unconfirmed session waits before suspend"
       (spawned? "false") #f)
(check "an unconfirmed session arms its confirmation timeout"
       (length %timers) 1)
(set! %lock-confirmed? #t)
(wm-on-session-lock)
(check "presentation confirmation completes the waiting suspend once"
       (length (filter (lambda (c) (string=? c "false")) %spawned)) 1)
(wm-on-session-lock)
(check "repeated confirmation does not complete the same suspend twice"
       (length (filter (lambda (c) (string=? c "false")) %spawned)) 1)
(set! %immediate-locked? #f)
(set! %lock-confirmed? #f)

;; ---------------------------------------------------------------------
;; Once locked presentation is confirmed, another suspend! needs neither
;; another locker nor a confirmation hook. Its injected callback reports
;; confirmed safety, rather than the earlier input/privacy guard.
;; ---------------------------------------------------------------------

(set! %immediate-locked? #t)
(set! %lock-confirmed? #t)
(set! %spawned '())
(set! %timers '())
(suspend!)
(check "suspend! while already locked suspends immediately"
       (spawned? "false") #t)
(check "suspend! while already locked does not respawn the locker"
       (spawned? "true") #f)
(check "suspend! while already locked arms no timeout" %timers '())
(set! %immediate-locked? #f)
(set! %lock-confirmed? #f)

;; ---------------------------------------------------------------------
;; wm-on-session-lock / wm-on-session-unlock ((minde groups)) run
;; user-extensible 'session-lock / 'session-unlock hooks.
;; ---------------------------------------------------------------------

(define %lock-hook-fired 0)
(define %unlock-hook-fired 0)
(add-event-hook! 'session-lock
  (lambda () (set! %lock-hook-fired (+ %lock-hook-fired 1))))
(add-event-hook! 'session-unlock
  (lambda () (set! %unlock-hook-fired (+ %unlock-hook-fired 1))))
(wm-on-session-lock)
(wm-on-session-unlock)
(check "wm-on-session-lock runs the session-lock hook"
       (>= %lock-hook-fired 1) #t)
(check "wm-on-session-unlock runs the session-unlock hook"
       (>= %unlock-hook-fired 1) #t)

(if (zero? %failures)
    (format #t "all session tests passed~%")
    (exit 1))
