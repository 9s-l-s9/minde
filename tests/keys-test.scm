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
(define (wm-output-geometry) (list 0 0 1280 720))
(define %messages '())
(define (wm-message text . _) (set! %messages (cons text %messages)) #t)
;; Sprint-3 subrs (recording fakes).
(define %timer-calls '())
(define (wm-run-after-ms ms token)
  (set! %timer-calls (cons (cons ms token) %timer-calls)) #t)
(define %fullscreen-calls '())
(define (wm-set-fullscreen id on) (set! %fullscreen-calls (cons (cons id on) %fullscreen-calls)) #t)
(define %killed '())
(define (wm-kill-window id) (set! %killed (cons id %killed)) #t)
(define %warps '())
(define (wm-warp-pointer x y) (set! %warps (cons (cons x y) %warps)) #t)
(define %paste-requests 0)
(define (wm-request-paste) (set! %paste-requests (+ %paste-requests 1)) #t)
(define %clipboard #f)
(define (wm-set-clipboard text) (set! %clipboard text) #t)
(define (wm-border-color hex) #t)

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

;; init.scm ships whatever prefix the user prefers (currently Print); pin
;; it back to C-t so the state-machine checks below are stable.
(set-prefix-key! '(ctrl) "t")

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
;; Nested keymaps: C-t A enters the agents submap, then c spawns.
;; ---------------------------------------------------------------------

(set! %spawned '())
(wm-handle-key ctrl-bit #f "t")
(check "C-t A is consumed (enters agents submap)"
       (wm-handle-key 1 #f "A")
       #t)
(check "submap key c is consumed"
       (wm-handle-key 0 #f "c")
       #t)
(check-true "submap binding spawned the agent command"
            (and (pair? %spawned)
                 (string-contains (car %spawned) "codex")))
;; And unbound submap keys swallow + reset.
(wm-handle-key ctrl-bit #f "t")
(wm-handle-key 1 #f "A")
(check "unbound submap key consumed" (wm-handle-key 0 #f "x") #t)
(check "state reset to normal after unbound submap key"
       (wm-handle-key 0 #f "x")
       #f)
(check-true "unbound key echoed via wm-message"
            (and (pair? %messages)
                 (string-contains (car %messages) "not bound")))

;; ---------------------------------------------------------------------
;; Prefix keysym bound in the prefix map: repeated presses keep cycling
;; (Print Print Print...), state stays armed.
;; ---------------------------------------------------------------------

(set-prefix-key! '() "Print")
(check "Print arms the prefix" (wm-handle-key 0 #f "Print") #t)
(check "second Print fires its binding (consumed)" (wm-handle-key 0 #f "Print") #t)
(check "third Print fires again without re-arming" (wm-handle-key 0 #f "Print") #t)
;; Still armed: a bound prefix key like v fires and resolves.
(check "prefix still armed afterwards: v runs vsplit" (wm-handle-key 0 #f "v") #t)
(check "then back to normal: plain t forwarded" (wm-handle-key 0 #f "t") #f)
(set-prefix-key! '(ctrl) "t")

;; ---------------------------------------------------------------------
;; iresize mode: C-t s arms the resize keymap; its bindings return the
;; keymap so it stays armed until Return/Escape.
;; ---------------------------------------------------------------------

(wm-handle-key ctrl-bit #f "t")
(check "C-t s enters resize mode (consumed)" (wm-handle-key 0 #f "s") #t)
(check "Left consumed in resize mode" (wm-handle-key 0 #f "Left") #t)
(check "mode stays armed: second Left consumed" (wm-handle-key 0 #f "Left") #t)
(check "b (balance) consumed and stays armed" (wm-handle-key 0 #f "b") #t)
(check "Down still consumed after balance" (wm-handle-key 0 #f "Down") #t)
(check "Return exits resize mode (consumed)" (wm-handle-key 0 #f "Return") #t)
(check "after exit, plain t is forwarded again" (wm-handle-key 0 #f "t") #f)

;; Escape exits too.
(wm-handle-key ctrl-bit #f "t")
(wm-handle-key 0 #f "s")
(check "Escape exits resize mode (consumed)" (wm-handle-key 0 #f "Escape") #t)
(check "after Escape, plain t forwarded" (wm-handle-key 0 #f "t") #f)

;; ---------------------------------------------------------------------
;; Modifier-prefixed keymap specs: C-0 pulls, plain 0 selects; M-Left
;; fires; unprefixed bindings still work under shift (bare fallback).
;; ---------------------------------------------------------------------

(wm-handle-key ctrl-bit #f "t")
(check "plain 0 dispatches (select-by-number)" (wm-handle-key 0 #f "0") #t)
(wm-handle-key ctrl-bit #f "t")
(check "C-0 dispatches (pull-by-number)" (wm-handle-key 4 #f "0") #t)
(wm-handle-key ctrl-bit #f "t")
(check "M-Left dispatches (move-window)" (wm-handle-key 8 #f "Left") #t)
(wm-handle-key ctrl-bit #f "t")
(check "plain Left dispatches (move-focus)" (wm-handle-key 0 #f "Left") #t)
(wm-handle-key ctrl-bit #f "t")
(check "Tab dispatches (other-window)" (wm-handle-key 0 #f "Tab") #t)
;; Shifted letter: arrives as mods=1 name="W"; no "S-W" binding exists,
;; so the bare "W" (echo-windows) must fire via the fallback.
(wm-handle-key ctrl-bit #f "t")
(set! %messages '())
(check "shift+W falls back to the bare W binding" (wm-handle-key 1 #f "W") #t)
(check-true "W echoed the window list" (pair? %messages))

;; ---------------------------------------------------------------------
;; Sprint 2: which-key (?), describe-key, colon eval prompt,
;; quit-confirm, lastmsg, marks keys.
;; ---------------------------------------------------------------------

;; ? echoes the armed keymap and STAYS armed.
(set! %messages '())
(wm-handle-key ctrl-bit #f "t")
(check "? in prefix state is consumed" (wm-handle-key 1 #f "question") #t)
(check-true "? echoed the binding docs" (and (pair? %messages)
                                             (string-contains (car %messages) "vsplit")))
(check "still armed after ?: v runs vsplit" (wm-handle-key 0 #f "v") #t)

;; describe-key: F1 arms it, the next key is described, not dispatched.
(set! %messages '())
(wm-handle-key ctrl-bit #f "t")
(check "F1 is consumed" (wm-handle-key 0 #f "F1") #t)
(check "described key is consumed" (wm-handle-key 0 #f "v") #t)
(check-true "describe-key reported the v binding"
            (string-contains (car %messages) "vsplit"))

;; colon eval prompt: type (+ 1 2), get 3 echoed.
(set! %messages '())
(wm-handle-key ctrl-bit #f "t")
(check "colon opens the eval prompt" (wm-handle-key 0 #f "colon") #t)
(for-each (lambda (c) (wm-handle-key 0 #f (string c) (string c)))
          (string->list "(+ 1 2)"))
(wm-handle-key 0 #f "Return" "")
(check-true "eval echoed the result 3"
            (find (lambda (m) (string-contains m "3")) %messages))

;; quit-confirm: Q prompts; n keeps running, yes quits.
(set! %quit? #f)
(wm-handle-key ctrl-bit #f "t")
(wm-handle-key 0 #f "Q")
(wm-handle-key 0 #f "n" "n")
(wm-handle-key 0 #f "Return" "")
(check "answering n does not quit" %quit? #f)
(wm-handle-key ctrl-bit #f "t")
(wm-handle-key 0 #f "Q")
(for-each (lambda (c) (wm-handle-key 0 #f (string c) (string c)))
          (string->list "yes"))
(wm-handle-key 0 #f "Return" "")
(check "answering yes quits" %quit? #t)

;; lastmsg + marks keys dispatch without error.
(wm-handle-key ctrl-bit #f "t")
(check "C-m (lastmsg) consumed" (wm-handle-key 4 #f "m") #t)
(wm-handle-key ctrl-bit #f "t")
(check "x (mark) consumed" (wm-handle-key 0 #f "x") #t)
(wm-handle-key ctrl-bit #f "t")
(check "M-x (pull-marked) consumed" (wm-handle-key 8 #f "x") #t)

;; ---------------------------------------------------------------------
;; Sprint 3: timers, fullscreen, kill, banish, flash, urgency, clipboard.
;; ---------------------------------------------------------------------

;; wm-run-after stores a thunk that wm-on-timer runs.
(define %fired #f)
(wm-run-after 5 (lambda () (set! %fired #t)))
(check-true "wm-run-after armed the Rust timer" (pair? %timer-calls))
(wm-on-timer (cdar %timer-calls))
(check "wm-on-timer ran the stored thunk" %fired #t)
;; A throwing thunk must not propagate.
(wm-run-after 5 (lambda () (error "boom")))
(check "a throwing timer thunk is caught"
       (begin (wm-on-timer (cdar %timer-calls)) #t)
       #t)

;; Map a window so fullscreen/kill have a target.
(wm-on-window-map 1 "term" "foot")

(wm-handle-key ctrl-bit #f "t")
(check "M-f (fullscreen) consumed" (wm-handle-key 8 #f "f") #t)
(check "fullscreen set on window 1" (car %fullscreen-calls) '(1 . #t))
(wm-handle-key ctrl-bit #f "t")
(wm-handle-key 8 #f "f")
(check "second M-f toggles fullscreen off" (car %fullscreen-calls) '(1 . #f))

(wm-handle-key ctrl-bit #f "t")
(check "K (force kill) consumed" (wm-handle-key 1 #f "K") #t)
(check "kill targeted window 1" %killed '(1))

(wm-handle-key ctrl-bit #f "t")
(check "B (banish) consumed" (wm-handle-key 1 #f "B") #t)
(check-true "banish warped the pointer" (pair? %warps))

(set! %timer-calls '())
(wm-handle-key ctrl-bit #f "t")
(check "C-c (flash frame) consumed" (wm-handle-key 4 #f "c") #t)
(check-true "flash armed a restore timer" (pair? %timer-calls))
(check "flash restore timer runs without error"
       (begin (wm-on-timer (cdar %timer-calls)) #t)
       #t)

;; Urgency: wm-on-urgent echoes and C-u jumps.
(set! %messages '())
(wm-on-urgent 1)
(check-true "urgent window echoed"
            (and (pair? %messages) (string-contains (car %messages) "Urgent")))
(wm-handle-key ctrl-bit #f "t")
(check "C-u (next-urgent) consumed" (wm-handle-key 4 #f "u") #t)
(wm-handle-key ctrl-bit #f "t")
(set! %messages '())
(wm-handle-key 4 #f "u")
(check-true "second C-u reports no urgent windows"
            (and (pair? %messages)
                 (string-contains (car %messages) "No urgent")))

;; Clipboard: paste routes into the open prompt; C-y requests, M-w copies.
(set! %messages '())
(wm-handle-key ctrl-bit #f "t")
(wm-handle-key 0 #f "colon")
(wm-on-paste "(+ 1 2)")
(wm-handle-key 0 #f "Return" "")
(check-true "pasted expression evaluated to 3"
            (find (lambda (m) (string-contains m "3")) %messages))

(wm-handle-key ctrl-bit #f "t")
(wm-handle-key 0 #f "colon")
(for-each (lambda (c) (wm-handle-key 0 #f (string c) (string c)))
          (string->list "abc"))
(check "C-y in the prompt is consumed" (wm-handle-key 4 #f "y" "") #t)
(check "C-y requested a paste" %paste-requests 1)
(check "M-w in the prompt is consumed" (wm-handle-key 8 #f "w" "") #t)
(check "M-w copied the prompt buffer" %clipboard "abc")
(wm-handle-key 0 #f "Escape" "")

;; Head keys: with one head, S/M-S echo "only one head" without error;
;; with a second synthetic head, S switches.
(set! %messages '())
(wm-handle-key ctrl-bit #f "t")
(check "S (snext) consumed" (wm-handle-key 1 #f "S") #t)
(check-true "single head echoed" (and (pair? %messages)
                                      (string-contains (car %messages) "one head")))
(wm-on-heads-changed '((0 0 0 1280 720) (1 1280 0 1280 720)))
(wm-handle-key ctrl-bit #f "t")
(wm-handle-key 1 #f "S")
(check "snext! switched to head 1" (current-head-id) 1)
(wm-handle-key ctrl-bit #f "t")
(check "M-s (sother) consumed" (wm-handle-key 8 #f "s") #t)
(check "sother! back on head 0" (current-head-id) 0)
(wm-on-heads-changed '((0 0 0 1280 720))) ; back to one head

;; M-c: copy the last message.
(echo "hello from the message ring")
(wm-handle-key ctrl-bit #f "t")
(check "M-c (copy last message) consumed" (wm-handle-key 8 #f "c") #t)
(check "M-c put the last message on the clipboard"
       %clipboard "hello from the message ring")

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
