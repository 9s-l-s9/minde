# 02 -- A writing session that undoes itself

**Request.**  "I need to write for the next 90 minutes.  Give me just
Emacs and one terminal.  If Signal or Slack gets urgent, do not show me.
Keep a list and tell me afterwards.  Then put everything back."

Nobody would configure this permanently.  It is a one-off program with
a beginning, a body and an end.

## Session

### Today's API  (exists, except where marked)

```scheme
(define before (dump-desktop))
(define missed '())

(create-group! "writing")
(apply-layout-spec! '(hsplit 0.65 leaf leaf))
(for-each (lambda (id)
            (when (member (window-app-id id) '("emacs" "foot"))
              (pull-window-by-id! id)))
          (all-window-ids))

;; Swallow urgency from chat apps, remember who wanted attention.
(define (quiet id)
  (when (member (window-app-id id) '("Signal" "Slack"))
    (set! missed (cons (window-title id) missed))
    (clear-urgent! id)))
(add-event-hook! 'urgent-window quiet)

;; 90 minutes later: undo everything, then report.
(wm-run-after (* 90 60 1000)
  (lambda ()
    (remove-event-hook! 'urgent-window quiet)
    (delete-current-group!)
    (restore-frames! before)                          ; sketch: restore from a dump-desktop value
    (echo (if (null? missed)
              "Session over. Nothing was waiting."
              (string-append "Session over. Missed: "
                             (string-join (delete-duplicates missed) ", "))))))
```

Failure mode of this version: if `restore-frames!` throws, the hook is
already gone and the group deleted.  That is what plan:D5 fixes.

### With PLAN.md items D1, D4, D5  (plan)

```scheme
(with-session 'writing #:timeout-ms (* 90 60 1000)
  (create-group! "writing")
  (apply-layout-spec! '(hsplit 0.65 leaf leaf))
  (for-each pull-window-by-id!
            (map (lambda (w) (assq-ref w 'id))
                 (windows #:app-id '("emacs" "foot"))))       ; plan:A2
  (add-event-hook! 'urgent-window quiet))                    ; owned by 'writing
;; snapshot, retract! and restore-desktop! happen on end, error or timeout
```

### Two days later: "Can I have it as a key, and ask me how long?"  (exists)

```scheme
(register-command! 'focus
  (lambda ()
    (read-one-line "Minutes: "
      (lambda (answer)
        (start-focus-session! (string->number answer)))))
  #:summary "Distraction-free session")
(bind-prefix-key! "f" (lambda () (invoke-command 'focus)) "focus session")
```

The definitions are copied to the config and reloaded.  `C-t f` asks
"Minutes:" in the compositor's own prompt.  The feature grew out of a
conversation and was tried live before being kept.

## Against a dispatcher API

- The layout must be coaxed from the dynamic tiler after windows move;
  there is no snapshot to restore from.
- Swallowing urgency needs a daemon on the event socket; the compositor
  has no "clear urgency" command, so the daemon fakes it.
- The timer is a `sleep` in that daemon.  If it dies, nothing restores.
- "Ask me how long" means shelling out to a launcher.
- Making it permanent is a shell script plus a `bind` line; the
  compositor knows the key exists, not what it does, so help cannot
  describe it.

## Why regular users care

People do not want to configure a window manager.  They want to say
what they need today.  Situational, temporary, self-undoing behaviour is
what agents will be asked for most, and what a fixed vocabulary handles
worst.
