# 01 -- Firefox Picture-in-Picture hijacks a frame

**Request.**  "When I pop a video out in Firefox, the little player grabs
my frame and pushes the browser somewhere I cannot see.  Fix it."

The agent does not know in advance which property distinguishes the
player from the browser window.  It finds out by observing the running
system, tries a fix live, then keeps only the one line that matters.

## Session

### 1. Look around  (exists)

```scheme
(map (lambda (id) (list id (window-app-id id) (window-title id) (window-floating? id)))
     (all-window-ids))
;; => ((7 "firefox" "Minde PR 412 - Mozilla Firefox" #f) (9 "foot" "foot" #f))
```

Nothing wrong yet: the player is not open.

### 2. Plant a probe  (exists)

```scheme
(define probe-log '())
(define (probe id)
  (set! probe-log
        (cons (list id (window-app-id id) (window-title id) (window-geometry id))
              probe-log)))
(add-event-hook! 'new-window probe)
```

Agent to user: "Pop out a video now."

### 3. Read the evidence  (exists)

```scheme
probe-log
;; => ((12 "firefox" "Picture-in-Picture" (0 0 480 270)))
```

The app-id is the same as the browser's; the title and the small size
are what distinguish it.  With plan:A1 the probe would also see
`parent`, `kind` and `min-size`, and the guess would not be needed.

### 4. Try a fix live  (exists)

```scheme
(define (pip-fix id)
  (when (equal? (window-title id) "Picture-in-Picture")
    (float-window! id)
    (toggle-always-on-top! id)))
(add-event-hook! 'new-window pip-fix)
```

Second attempt after seeing it land top-left over the editor:

```scheme
(remove-event-hook! 'new-window pip-fix)
(define (pip-fix id)
  (when (equal? (window-title id) "Picture-in-Picture")
    (float-window! id)
    (set-float-geometry! id '(1400 780 480 270))
    (toggle-always-on-top! id)))
(add-event-hook! 'new-window pip-fix)
```

Note: `add-event-hook!` stores the closure, not the name, so a plain
`define` does not update the registered hook.  Remove and re-add.

### 5. Promote the finding  (exists)

```scheme
(remove-event-hook! 'new-window probe)
(remove-event-hook! 'new-window pip-fix)
(add-placement-rule! "Picture-in-Picture" #:float? #t)
```

Then the same `add-placement-rule!` line goes into
`scheme/default-config.scm`, followed by `mindectl check-config` and
`(reload-configuration!)`.  `(event-hook-procedures 'new-window)` shows
nothing left behind.

## Against a dispatcher API

Hyprland has the ready-made answer
`windowrulev2 = float, title:^(Picture-in-Picture)$` and an experienced
user knows it.  The REPL advantage is smallest when the problem is well
known.  It is largest when the agent must discover which property
distinguishes the window, whether the title is set at map time or a
moment later, or why focus went somewhere unexpected -- the probe in
step 2 can record anything the compositor knows, not only what the
event stream was designed to carry.

## What made it a REPL session

- The probe extended what the compositor reports, for one session.
- Each hypothesis was live code tested against the next real window.
- Data from the investigation (the geometry) fed the fix.
- Diagnosis and fix used the same channel.
