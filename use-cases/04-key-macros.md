# 04 -- One-off key macros

Small programs bound to a key for the duration of a task, then removed.
The recurring shape: a prompt in the compositor, an action against a
chosen window or a spawned process, state kept in a closure.

## A. Type a prompted string into the spreadsheet  (exists)

**Request.**  "During this meeting, when I press C-t then x, ask me for
a note and put it into the Calc window without me switching to it."

(A separate Print-prefixed map is possible with `make-keymap` and
`bind-key!`; the examples use the existing C-t prefix to stay short.)

```scheme
(define calc-id
  (find (lambda (id) (equal? (window-app-id id) "libreoffice-calc"))
        (all-window-ids)))

(register-command! 'meeting-note
  (lambda ()
    (let ((was (focused-window-id)))
      (read-one-line "Note: "
        (lambda (text)
          (focus-window-by-id! calc-id)
          (window-send-string text)
          (send-key "Return")
          (focus-window-by-id! was)))))
  #:summary "Type a note into Calc")
(bind-prefix-key! "x" (lambda () (invoke-command 'meeting-note)) "meeting note")
```

The prompt runs in the compositor, so focus and pointer are exactly
where they were when it finishes.  With plan:E1 the focus dance
disappears: `(window-send-string text #:window calc-id)`.  Anything
cell-aware ("next empty row") needs the application to answer, which
injection cannot do; see PLAN.md G1 (AT-SPI bridge).

After the meeting:

```scheme
(hash-remove! %prefix-bindings "x")   ; sketch: no public unbind yet; init.scm's table
```

With plan:D1/D2 the whole thing is `(with-owner 'meeting ...)` and
`(retract! 'meeting)`.

## B. Toggle a voice recording  (exists)

**Request.**  "C-t then r starts recording my microphone; pressing it
again stops and tells me the file name."

```scheme
(define recording #f)
(register-command! 'toggle-recording
  (lambda ()
    (if recording
        (begin
          (spawn! "pkill -INT -f 'arecord.*minde-rec'")
          (echo (string-append "Saved " recording))
          (set! recording #f))
        (let ((path (string-append (getenv "HOME") "/rec/minde-rec-"
                                   (number->string (current-time)) ".wav")))
          (spawn! (string-append "arecord -f cd " path))
          (echo "Recording...")
          (set! recording path))))
  #:summary "Start/stop microphone recording")
(bind-prefix-key! "r" (lambda () (invoke-command 'toggle-recording)) "toggle recording")
```

Limit: `spawn!` reports that the fork was enqueued, not that `arecord`
ran.  A persistent indicator needs a hook or an overlay (plan:H1).

## C. Screenshot the focused window to the clipboard  (exists, async)

```scheme
(register-command! 'shot-window
  (lambda ()
    (let ((path "/tmp/minde-shot.png"))
      (wm-screenshot path (focused-window-id))
      (wm-run-after 500 (lambda () (spawn! (string-append "wl-copy < " path))))))
  #:summary "Screenshot focused window")
(bind-prefix-key! "w" (lambda () (invoke-command 'shot-window)) "screenshot window")
```

With plan:B1 the `wm-run-after` guess becomes a synchronous call with a
timeout.

## Against a dispatcher API

Each of these is a shell script plus a `bind` line.  Two things are
lost: the prompt (a launcher has to be shelled out and focus moves),
and the fact that the compositor knows what the key does, so the
contextual help and the generated key map can describe it.
