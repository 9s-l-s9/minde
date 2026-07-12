;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; session.scm -- session management: screen lock, suspend, logout.
;;;
;;; StumpWM (X11) had nothing to port here -- X11 never standardized a
;;; session-lock protocol, so StumpWM users always reached for an
;;; external locker themselves. Wayland has one (ext-session-lock-v1,
;;; implemented on the Rust side); this module still delegates the
;;; actual locking UI to an external program (default swaylock),
;;; StumpWM-style "run a command, don't reinvent one", but coordinates
;;; with the Rust side's session-lock state so suspend! can wait for the
;;; lock to actually be up.
;;;
;;; Interface contract with the Rust side: it calls two 0-arg events,
;;; `wm-on-session-lock` and `wm-on-session-unlock`, defined and
;;; exported from (minde groups) (see the comment there) rather than
;;; here, because that's where this codebase's other Rust-facing 0/N-arg
;;; event entry points already live. Both run named hooks ('session-lock
;;; / 'session-unlock) via (minde hooks); suspend! below adds a
;;; one-shot 'session-lock hook of its own to know when it's safe to
;;; actually suspend, instead of racing the locker's own startup time.
;;;
;;; Same load-time constraint as frames.scm: nothing here may call a
;;; wm-* Rust subr, or wm-run-after (a Scheme wrapper defined by
;;; init.scm itself, looked up the same dynamic way -- see rust-call
;;; below), at module load time.

(define-module (minde session)
  ;; Non-declarative: the config variables below are documented (README,
  ;; "Session management") as user-overridable via (set! %lock-command
  ;; ...) from init.scm. Guile 3's default declarative modules let the
  ;; compiler constant-fold a module's own top-level references, so a
  ;; compiled build would read the ORIGINAL value while an external set!
  ;; only mutated the exported copy -- suspend!/lock-screen! would ignore
  ;; the user's override. #:declarative? #f keeps those cross-references
  ;; going through the live variable, honoring the set! contract.
  #:declarative? #f
  #:use-module (minde frames)
  #:use-module (minde hooks)
  #:use-module (minde ui prompt)
  #:export (logout!
            lock-screen!
            suspend!
            %lock-command
            %suspend-command
            %lock-on-suspend?
            %lock-timeout-ms))

;; ---------------------------------------------------------------------
;; User-configurable defaults (StumpWM-style plain variables -- override
;; after loading, e.g. (set! %lock-command "hyprlock") from init.scm).
;; ---------------------------------------------------------------------

(define %lock-command "swaylock -f")
;; Guix systems run elogind rather than full systemd, but elogind speaks
;; the same logind D-Bus API, so loginctl still works unmodified.
(define %suspend-command "loginctl suspend")
(define %lock-on-suspend? #t)
;; How long suspend! waits for wm-on-session-lock to confirm the locker
;; actually came up before giving up. swaylock's own startup is close to
;; instant; this is generous headroom for a cold cache or loaded system.
(define %lock-timeout-ms 5000)

;; ---------------------------------------------------------------------
;; Rust subrs (and init.scm's own wm-run-after), looked up dynamically at
;; call time -- see (minde frames)'s rust-call comment for why: a
;; module compiled with define-module can't #:use-module something that
;; doesn't exist yet at its own compile time, and these are registered
;; (or, for wm-run-after, defined) only once the compositor/init.scm
;; actually starts. A missing definition (old binary, test stubs) is a
;; no-op, same policy as everywhere else in this codebase.
;; ---------------------------------------------------------------------

(define (rust-call name . args)
  (let* ((mod (resolve-module '(guile-user) #:ensure #f))
         (var (and mod (module-variable mod name))))
    (if var
        (apply (variable-ref var) args)
        (begin
          (format #t "minde: ~a unbound, ignoring call~%" name)
          #f))))

(define (spawn! cmd) (rust-call 'wm-spawn cmd))
(define (run-after ms thunk) (rust-call 'wm-run-after ms thunk))

;; ---------------------------------------------------------------------
;; Best-effort "is this installed" hint. wm-spawn can't itself report a
;; missing binary -- it just enqueues a fork on the compositor's main
;; thread (see src/guile/mod.rs) and returns whether *that enqueue*
;; succeeded, not whether the program later executed. A PATH search is a
;; cheap, synchronous, purely-Scheme proxy that catches the common case
;; of a bare locker name.
;; ---------------------------------------------------------------------

(define (command-program cmd)
  "Returns CMD's first whitespace-delimited token (its program name), or
#f for an empty/whitespace-only command."
  (let ((parts (string-tokenize cmd)))
    (and (pair? parts) (car parts))))

(define (program-missing? prog)
  "Best-effort PATH search for PROG. Returns #f (assume present) for an
explicit path or when PATH can't be read, rather than false-alarming."
  (and prog (not (string-null? prog))
       (not (string-index prog #\/))
       (not (search-path (parse-path (or (getenv "PATH") "")) prog))))

(define (warn-if-missing! cmd)
  (let ((prog (command-program cmd)))
    (when (program-missing? prog)
      (echo (format #f "warning: ~a not found on PATH" prog)))))

;; ---------------------------------------------------------------------
;; Logout
;; ---------------------------------------------------------------------

(define (logout!)
  "Ends the compositor after an explicit yes/no confirmation (StumpWM
quit-confirm style). Ending the compositor ends the whole Wayland
session (back to the login screen), so this must never fire from a
single keystroke -- it only calls the Rust wm-quit subr once the prompt
is answered \"y\" or \"yes\"."
  (read-one-line "log out (ends the session)? (yes/n) "
    (lambda (answer)
      (when (member answer '("y" "yes"))
        (rust-call 'wm-quit)))))

;; ---------------------------------------------------------------------
;; Lock
;; ---------------------------------------------------------------------

(define (lock-screen!)
  "Locks the session by spawning %lock-command (default \"swaylock
-f\"). Warns (best-effort, see warn-if-missing!) when the configured
locker isn't on PATH; either way this never throws -- worst case is an
inert wm-spawn, same as pressing a launcher key for a missing program."
  (warn-if-missing! %lock-command)
  (spawn! %lock-command)
  (echo "locking..."))

;; ---------------------------------------------------------------------
;; Suspend: locks first, then waits for confirmation that the lock
;; surface is actually up (via the 'session-lock hook that (minde
;; groups)'s wm-on-session-lock runs) before suspending, so the machine
;; always wakes locked. Spawning the locker and suspending in the same
;; breath would race swaylock's own startup -- it forks and talks to the
;; compositor before the lock surface exists -- and suspending mid-race
;; would wake the machine unlocked, defeating the entire point of this
;; command.
;;
;; If the lock never confirms within %lock-timeout-ms, suspend! gives up
;; and does NOT suspend (fails closed): silently suspending anyway on an
;; unconfirmed lock would be a worse security bug -- an unlocked machine
;; that looks asleep -- than a suspend that has to be retried by hand.
;; ---------------------------------------------------------------------

(define %suspend-armed? #f)

(define (on-locked-for-suspend!)
  (when %suspend-armed?
    (set! %suspend-armed? #f)
    (remove-event-hook! 'session-lock on-locked-for-suspend!)
    (spawn! %suspend-command)))

(define (on-lock-timeout!)
  (when %suspend-armed?
    (set! %suspend-armed? #f)
    (remove-event-hook! 'session-lock on-locked-for-suspend!)
    (echo "suspend cancelled: lock did not confirm in time")))

(define (suspend!)
  "Suspends the machine (default \"loginctl suspend\", elogind-compatible).
When %lock-on-suspend? is true (the default), locks first and waits for
confirmation before suspending -- see the comment above. When false,
suspends immediately without locking."
  (cond
   ((not %lock-on-suspend?) (spawn! %suspend-command))
   ;; Already locked (locker running before suspend! was called): the
   ;; 'session-lock hook only fires on a real unlocked->locked edge, so
   ;; waiting for it here would just time out. wm-session-locked? is a
   ;; boolean subr; an unbound one (old binary, test stubs) is #f via
   ;; rust-call, which safely falls through to the lock-and-wait path.
   ((rust-call 'wm-session-locked?) (spawn! %suspend-command))
   (%suspend-armed? (echo "suspend already pending"))
   (else
    (set! %suspend-armed? #t)
    (add-event-hook! 'session-lock on-locked-for-suspend!)
    (lock-screen!)
    (run-after %lock-timeout-ms on-lock-timeout!))))
