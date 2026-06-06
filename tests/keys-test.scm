;;; keys-test.scm -- Guile-only unit test of the wm-handle-key prefix
;;; state machine in scheme/init.scm.
;;;
;;; Run with:
;;;   guile -L scheme tests/keys-test.scm
;;;
;;; Stubs every wm-* Rust subr *before* loading init.scm (via
;;; primitive-load, same as the real Rust side does with
;;; scm_c_primitive_load), so init.scm's own top-level side effects (REPL
;;; server startup, default keybindings) run against fakes instead of
;;; missing subrs.

(use-modules (srfi srfi-1))

(define %spawned '())
(define %quit? #f)
(define %log-lines '())

(define (wm-spawn cmd) (set! %spawned (cons cmd %spawned)) #t)
(define (wm-quit) (set! %quit? #t) #t)
(define (wm-log msg) (set! %log-lines (cons msg %log-lines)) #t)
(define (wm-place-window id x y w h) #t)
(define (wm-focus-window id) #t)
(define (wm-close-window id) #t)
(define (wm-clear-focus) #t)
(define (wm-output-geometry) (list 1280 720))

;; Load init.scm by its full canonical path. init.scm's own top-level code
;; calls (current-filename) (to find scheme/ for add-to-load-path), which
;; expands to #f unless the path Guile records as the loaded port's
;; filename canonicalize-path's cleanly from the current working
;; directory. Because init.scm lives under a %load-path root (-L scheme;
;; init.scm is scheme/init.scm), Guile's default port-name
;; canonicalization records it relative to that root ("init.scm") rather
;; than the absolute path we invoke it with, and that relative name
;; doesn't exist from our cwd -- so current-filename comes back #f and
;; init.scm's own add-to-load-path call crashes on it. Forcing absolute
;; canonicalization keeps the recorded filename as the real path.
(fluid-set! %file-port-name-canonicalization 'absolute)
(primitive-load (canonicalize-path
                 (string-append (dirname (current-filename)) "/../scheme/init.scm")))

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

(define (check-true name got) (check name (if got #t #f) #t))

;; wm-handle-key's internal %key-state isn't exported, but it's inferable
;; from behavior: press the prefix (Ctrl-t) to enter prefix state, then
;; probe subsequent presses.

(define ctrl-bit 4) ; matches mod-symbol->bit's 'ctrl -> 4 in init.scm

;; ---------------------------------------------------------------------
;; normal -> prefix on Ctrl-t
;; ---------------------------------------------------------------------

(check "Ctrl-t enters prefix state (consumed)"
       (wm-handle-key ctrl-bit #f "t")
       #t)

;; ---------------------------------------------------------------------
;; prefix -> prefix-key pressed again (literal forward): returns #f and
;; resets to normal.
;; ---------------------------------------------------------------------

(check "prefix, then t again: forwarded to client (#f)"
       (wm-handle-key ctrl-bit #f "t")
       #f)

;; Now back in normal state: a plain "t" (no prefix mods) should not be
;; treated as a bound prefix action nor re-enter prefix state; it's
;; unbound so it's forwarded.
(check "after literal-forward reset, back to normal: unbound plain t forwarded"
       (wm-handle-key 0 #f "t")
       #f)

;; ---------------------------------------------------------------------
;; prefix -> modifier press stays in prefix (not consumed, but state
;; doesn't reset either -- verified by a following bound key still firing
;; the prefix binding rather than being treated as a fresh normal-state
;; key).
;; ---------------------------------------------------------------------

(wm-handle-key ctrl-bit #f "t") ; re-enter prefix

(check "a bare modifier keysym in prefix state is not consumed"
       (wm-handle-key 4 #f "Control_L")
       #f)

(check "prefix state survives a modifier press: v still runs the vsplit binding"
       (wm-handle-key 0 #f "v")
       #t)

;; ---------------------------------------------------------------------
;; C-t g dispatches to the groups binding (gnext!) without error.
;; ---------------------------------------------------------------------

(wm-handle-key ctrl-bit #f "t")
(check "C-t g is consumed (dispatches to gnext!)"
       (wm-handle-key 0 #f "g")
       #t)

;; ---------------------------------------------------------------------
;; C-t Return spawns a terminal via the prefix binding.
;; ---------------------------------------------------------------------

(wm-handle-key ctrl-bit #f "t")
(wm-handle-key 0 #f "Return")
(check-true "C-t Return spawned something"
            (pair? %spawned))

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
