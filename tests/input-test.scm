;;; input-test.scm -- the native read-one-line prompt engine, driven
;;; through wm-handle-key exactly as Rust drives it (4-arg form).
;;;
;;; Run with: guile -L scheme tests/input-test.scm

(use-modules (srfi srfi-1))

(define %spawned '())
(define %messages '())
(define %cleared 0)
(define %test-repeat-states '())
(define %test-border #f)
(define %test-timers '())
(define (wm-set-key-repeat enabled?) (set! %test-repeat-states (cons enabled? %test-repeat-states)))
(define (wm-border-color color) (set! %test-border color))
(define (wm-run-after-ms ms token) (set! %test-timers (acons ms token %test-timers)))

(define (wm-spawn cmd) (set! %spawned (cons cmd %spawned)) #t)
(define (wm-quit) #t)
(define (wm-session-locked?) #f)
(define (wm-session-lock-confirmed?) #f)
(define (wm-session-lock-abandoned?) #f)
(define (wm-log msg) #t)
(define (wm-place-window id x y w h) #t)
(define (wm-focus-window id) #t)
(define (wm-close-window id) #t)
(define (wm-clear-focus) #t)
(define (wm-focus-rect x y w h) #t)
(define (wm-output-geometry) (list 0 0 1280 720))
(define (wm-message text . _) (set! %messages (cons text %messages)) #t)
(define (wm-clear-message) (set! %cleared (+ %cleared 1)) #t)
(define %paste-requests 0)
(define (wm-request-paste) (set! %paste-requests (+ %paste-requests 1)) #t)
(define %clipboard #f)
(define (wm-set-clipboard text) (set! %clipboard text) #t)

(fluid-set! %file-port-name-canonicalization 'absolute)
(primitive-load (canonicalize-path
                 (string-append (dirname (current-filename)) "/../scheme/init.scm")))
(set-prefix-key! '(ctrl) "t")

(define %failures 0)
(define (check name got expected)
  (if (equal? got expected)
      (format #t "ok - ~a~%" name)
      (begin
        (set! %failures (+ %failures 1))
        (format #t "FAIL - ~a: expected ~s, got ~s~%" name expected got))))
(define (check-true name got) (check name (if got #t #f) #t))

(define ctrl 4)
(define (key name utf8) (wm-handle-key 0 #f name utf8))
(define (ckey name) (wm-handle-key ctrl #f name ""))

;; ---------------------------------------------------------------------
;; A submitted line reaches the callback; typing renders live.
;; ---------------------------------------------------------------------

(define %submitted #f)
(read-one-line "run: " (lambda (s) (set! %submitted s)) #:history 'test)

(check-true "prompt active" (input-active?))
(check "prompt rendered" (car %messages) "run: |")
(check "prompt forwards Shift for client modifier state"
       (wm-handle-key 1 #f "Shift_L" "") #f)
(check-true "modifier leaves prompt active" (input-active?))

(key "f" "f") (key "o" "o") (key "x" "x")
(check "typed text rendered with cursor" (car %messages) "run: fox|")

(key "BackSpace" "")
(key "o" "o") (key "t" "t")
(check "backspace edits" (car %messages) "run: foot|")

;; While the prompt is open, every key is consumed -- prefix included.
(check "keys are consumed by the prompt" (ckey "t") #t)

(key "Return" "")
(check "submit delivered the buffer" %submitted "foot")
(check-true "prompt closed after submit" (not (input-active?)))
(check-true "message cleared" (> %cleared 0))

;; ---------------------------------------------------------------------
;; Editing keys: C-a, C-e, C-k, word motion.
;; ---------------------------------------------------------------------

(read-one-line "edit: " (lambda (s) (set! %submitted s)))
(for-each (lambda (c) (key c c)) '("a" "b" " " "c" "d"))
(ckey "a")
(check "C-a moves to start" (car %messages) "edit: |ab cd")
(ckey "k")
(check "C-k kills to end" (car %messages) "edit: |")
(for-each (lambda (c) (key c c)) '("x" "y"))
(ckey "b")
(check "C-b moves left" (car %messages) "edit: x|y")
(key "Return" "")
(check "edited buffer submitted" %submitted "xy")

;; ---------------------------------------------------------------------
;; Completion: TAB cycles prefix matches.
;; ---------------------------------------------------------------------

(read-one-line "run: " (lambda (s) (set! %submitted s))
               #:completions (list "emacs" "emacsclient" "foot"))
(key "e" "e")
(key "Tab" "")
(check "first TAB completes to first prefix match" (car %messages) "run: emacs|")
(key "Tab" "")
(check "second TAB cycles" (car %messages) "run: emacsclient|")
(key "Return" "")
(check "completed buffer submitted" %submitted "emacsclient")

;; ---------------------------------------------------------------------
;; History: C-p recalls, C-n returns to the live buffer.
;; ---------------------------------------------------------------------

(read-one-line "run: " (lambda (s) (set! %submitted s)) #:history 'test)
(key "Up" "")
(check "C-p/Up recalls last submitted line" (car %messages) "run: foot|")
(key "Down" "")
(check "C-n/Down returns to empty live buffer" (car %messages) "run: |")
(key "Escape" "")
(check-true "escape aborts" (not (input-active?)))

;; ---------------------------------------------------------------------
;; Abort callback fires; nothing spawned by aborted prompts.
;; ---------------------------------------------------------------------

(define %aborted #f)
(read-one-line "x: " (lambda (s) #f) #:on-abort (lambda () (set! %aborted #t)))
(ckey "g")
(check-true "C-g abort callback" %aborted)

;; ---------------------------------------------------------------------
;; Clipboard: C-y/C-v request a paste, handle-paste! inserts at point,
;; M-w copies the buffer.
;; ---------------------------------------------------------------------

(define %submitted2 #f)
(read-one-line "p: " (lambda (s) (set! %submitted2 s)))
(key "a" "a")
(ckey "y")
(check "C-y requested a paste" %paste-requests 1)
(wm-handle-key ctrl #f "v" "")
(check "C-v requested a paste too" %paste-requests 2)
(handle-paste! "XY\nZ")
(check "paste inserted at point, newline collapsed" (car %messages) "p: aXY Z|")
(key "Left" "")
(handle-paste! "-")
(check "paste lands at the cursor" (car %messages) "p: aXY -|Z")
(wm-handle-key 8 #f "w" "")
(check "M-w copied the buffer" %clipboard "aXY -Z")
(key "Return" "")
(check "buffer with pasted text submitted" %submitted2 "aXY -Z")
;; A paste arriving after the prompt closed is a silent no-op.
(handle-paste! "late")
(check-true "late paste ignored" (not (input-active?)))

(select-from-menu '("one" "two") (lambda (_) #t))
(check "menu forwards Control for client modifier state"
       (wm-handle-key 4 #f "Control_L" "") #f)
(check-true "modifier leaves menu active" (menu-active?))
(key "Escape" "")

;; A prefix keysym in a child map is ordinary input. Only a binding found
;; in the top-level prefix map may retain the repeated-prefix behavior.
(let ((saved %prefix-bindings) (ran 0))
  (set! %prefix-bindings (copy-hash-table saved))
  (bind-prefix-key! "F13" (make-keymap "t" (lambda () (set! ran (+ ran 1)))))
  (ckey "t") (key "F13" "") (key "t" "t")
  (check "nested prefix-keysym binding ran once" ran 1)
  (check "nested prefix keysym returns to normal" %key-state 'normal)
  (check "ordinary input after nested binding is forwarded" (key "a" "a") #f)
  (bind-prefix-key! "t" (lambda () (set! ran (+ ran 1))))
  (ckey "t") (key "t" "t") (key "t" "t")
  (check "top-level prefix binding remains repeatable" ran 3)
  (check-true "top-level prefix binding remains armed" (eq? %key-state %prefix-bindings))
  (set-key-state! 'normal)
  (hash-remove! %prefix-bindings "t")
  (ckey "t") (key "m" "m") (key "c" "c")
  (check-true "portable binding enters command mode" %command-mode)
  (check "literal prefix keysym is forwarded" (key "t" "t") #f)
  (check "literal escape clears command-mode flag" %command-mode #f)
  (check "literal escape leaves normal state" %key-state 'normal)
  (check "literal escape announces command-mode exit" (car %messages) "command mode off")
  (bind-prefix-key! "F14" (lambda () (set! ran (+ ran 1))))
  (ckey "t") (key "F14" "")
  (check "later prefix command does not revive command mode" %key-state 'normal)
  (leave-command-mode!)
  (set! %prefix-bindings saved))

;; Drive timer tokens deterministically through the real timer callback.
;; Restoring a frame flash must not take keyboard ownership from a UI.
(for-each
 (lambda (open)
   (set-key-state! 'normal)
   (flash-current-frame!)
   (let ((token (assoc-ref %test-timers 400)))
     (open)
     (check "prompt/menu enables repeat" (car %test-repeat-states) #t)
     (let ((repeat %test-repeat-states) (generation %keymap-generation))
       (handle-timer! token)
       (check "flash restoration does not change repeat" %test-repeat-states repeat)
       (check "flash restoration does not change generation" %keymap-generation generation)
       (check-true "prompt/menu stays active after flash" (or (input-active?) (menu-active?)))
       (check "flash restores the normal border" %test-border %border-color-normal)))
   (key "Escape" ""))
 (list (lambda () (read-one-line "flash: " (lambda (_) #t)))
       (lambda () (select-from-menu '("one" "two") (lambda (_) #t)))))
(set! %which-key-mode #t)
(set-key-state! %prefix-bindings)
(let ((help-token (assoc-ref %test-timers %which-key-delay-ms)))
  (flash-current-frame!)
  (handle-timer! (assoc-ref %test-timers 400))
  (check "flash restores the armed border" %test-border %border-color-prefix)
  (handle-timer! help-token)
  (check "flash preserves delayed which-key help" (car %messages) (keymap-help-string %prefix-bindings)))
(set! %which-key-mode #f)
(set-key-state! %resize-map)
(flash-current-frame!)
(handle-timer! (assoc-ref %test-timers 400))
(check "flash restores the resize border" %test-border %border-color-resize)
(set-key-state! 'normal)

(if (zero? %failures)
    (begin (format #t "all tests passed~%") (exit 0))
    (begin (format #t "~a test(s) FAILED~%" %failures) (exit 1)))
