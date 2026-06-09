;;; init.scm -- minde policy layer.
;;;
;;; Loaded by the Rust side (src/guile/mod.rs) once at startup. Redefine
;;; anything here live from the REPL (see below) to change behavior without
;;; restarting the compositor.

(use-modules (ice-9 hash-table)
             (system repl server))

;; scheme/frames.scm lives next to this file; add it to the load path so
;; `(use-modules (minde frames))` finds it regardless of cwd.
(add-to-load-path (dirname (current-filename)))

(use-modules (minde frames)
             (minde groups))

;; ---------------------------------------------------------------------
;; Keybinding table
;; ---------------------------------------------------------------------

;; Global (non-prefixed) bindings: (mods . keysym-name) -> thunk. Kept
;; deliberately small -- StumpWM style is that almost everything goes
;; through the C-t prefix.
(define %keybindings (make-hash-table))

;; Prefix-key bindings: keysym-name -> thunk. Looked up with no modifier
;; requirement beyond having entered prefix state (matching StumpWM, where
;; e.g. C-t c means "prefix, then plain c").
(define %prefix-bindings (make-hash-table))

;; X11-style modifier bitmask, mirrored on the Rust side: shift=1 ctrl=4
;; alt=8 super=64.
(define (mod-symbol->bit sym)
  (case sym
    ((shift) 1)
    ((ctrl control) 4)
    ((alt) 8)
    ((super logo) 64)
    (else (error "unknown modifier" sym))))

(define (mods->bitmask mods)
  "MODS is a list of symbols such as '(super) or '(ctrl shift)."
  (apply + (map mod-symbol->bit mods)))

(define (bind-key! mods key thunk)
  "Bind KEY (an xkb keysym name string, e.g. \"Return\", \"q\", \"d\") with
modifier list MODS (e.g. '(super)) to THUNK, a zero-argument procedure run
when the binding fires. Global binding, always active."
  (hash-set! %keybindings (cons (mods->bitmask mods) key) thunk))

(define (bind-prefix-key! key thunk)
  "Bind KEY (e.g. \"s\", \"S\", \"c\") to THUNK, run when KEY is pressed
right after the prefix key (C-t)."
  (hash-set! %prefix-bindings key thunk))

;; The prefix key itself: C-t by default, StumpWM-style configurable.
;; e.g. (set-prefix-key! '() "Print") to mirror a StumpWM
;; (set-prefix-key (kbd "Print")) setup.
(define %prefix-mods (mods->bitmask '(ctrl)))
(define %prefix-key "t")

(define (set-prefix-key! mods key)
  "Set the prefix to KEY (an xkb keysym name string) with modifier list
MODS (possibly '())."
  (set! %prefix-mods (mods->bitmask mods))
  (set! %prefix-key key))

;; wm-handle-key is a tiny two-state machine: 'normal (the usual case) and
;; 'prefix (right after C-t, waiting for the next key).
(define %key-state 'normal)

(define (run-binding! thunk mods keysym-name)
  ;; Errors from a binding are logged, not fatal -- one bad keybinding
  ;; shouldn't take down the compositor.
  (catch #t
    thunk
    (lambda (key . args)
      (wm-log (format #f "error in keybinding ~a ~a: ~a ~a" mods keysym-name key args)))))

;; Bare modifier presses (e.g. the Shift_L press that precedes typing "S")
;; must not be interpreted as the key following the prefix.
(define %modifier-keysyms
  '("Shift_L" "Shift_R" "Control_L" "Control_R" "Alt_L" "Alt_R"
    "Super_L" "Super_R" "Meta_L" "Meta_R" "ISO_Level3_Shift" "Caps_Lock"))

(define (modifier-keysym? name)
  (member name %modifier-keysyms))

(define (wm-handle-key mods-bitmask keysym keysym-name)
  "Called from Rust on every key press. Returns #t if the key was consumed
(and should not be forwarded to the focused client), #f otherwise."
  (case %key-state
    ((prefix)
     (cond
      ;; Pressing the prefix key's keysym again while already in prefix
      ;; state (e.g. C-t t) is StumpWM's "send this key literally"
      ;; escape: reset to normal state and forward the keypress to the
      ;; focused client instead of consuming it. Matched by keysym name
      ;; only (not modifiers) since the second press may or may not carry
      ;; the same modifier as the prefix itself.
      ((string=? keysym-name %prefix-key)
       (set! %key-state 'normal)
       #f)
      ((modifier-keysym? keysym-name)
       #f) ; stay in prefix state; let the client see the modifier
      (else
       (set! %key-state 'normal)
       (let ((thunk (hash-ref %prefix-bindings keysym-name)))
         (if thunk
             (run-binding! thunk mods-bitmask keysym-name)
             ;; Unbound prefix key: swallow it (StumpWM does the same,
             ;; typically with a "not bound" echo) rather than forwarding a
             ;; keypress the user didn't intend for the client.
             (wm-log (format #f "C-t ~a: not bound" keysym-name)))
         #t))))
    (else
     (if (and (= mods-bitmask %prefix-mods) (string=? keysym-name %prefix-key))
         (begin
           (set! %key-state 'prefix)
           #t)
         (let ((thunk (hash-ref %keybindings (cons mods-bitmask keysym-name))))
           (if thunk
               (begin
                 (run-binding! thunk mods-bitmask keysym-name)
                 #t)
               #f))))))

;; ---------------------------------------------------------------------
;; Default bindings
;; ---------------------------------------------------------------------

;; Global: super+q quits outright (kept as an escape hatch outside the
;; prefix, mirroring many StumpWM configs' emergency exit).
(bind-key! '(super) "q"
           (lambda () (wm-quit)))

;; Prefix bindings, mirroring the user's StumpWM root map:
;;   Return -- terminal          r -- run prompt (fuzzel)
;;   b -- browser                E -- emacs
;;   v -- vsplit (stacked)       h -- hsplit (side by side)
;;   c -- remove split           n -- next frame
;;   f -- next window in frame   k/d -- close window
;;   e -- lem editor
;;   p -- pull window from other frame into this one
;;   g -- next group             G -- new group (auto-named)
;;   m -- move window to next group
;;   Q -- quit compositor
;; (Emacs note: emacsclient needs the user's Guix Home shepherd emacs
;; daemon; plain emacs is the fallback.)
(bind-prefix-key! "Return" (lambda () (wm-spawn "alacritty || foot || xterm")))
(bind-prefix-key! "r" (lambda () (wm-spawn "fuzzel || bemenu-run")))
;; Prebuilt Mozilla binaries sometimes fall back to X11 (which minde
;; cannot display -- no Xwayland); force the Wayland backend.
(bind-prefix-key! "b" (lambda () (wm-spawn "MOZ_ENABLE_WAYLAND=1 zen || chromium --ozone-platform-hint=auto")))
(bind-prefix-key! "e" (lambda () (wm-spawn "lem -i sdl2")))
;; NOTE: needs a pgtk emacs (Guix package emacs-pgtk) -- an X11-only emacs
;; build cannot open a frame here at all. emacsclient additionally needs
;; the daemon running inside this session.
(bind-prefix-key! "E" (lambda () (wm-spawn "emacsclient -c -a emacs")))
(bind-prefix-key! "v" (lambda () (split-frame-vertical!)))
(bind-prefix-key! "h" (lambda () (split-frame-horizontal!)))
(bind-prefix-key! "c" (lambda () (remove-split!)))
(bind-prefix-key! "n" (lambda () (focus-next-frame!)))
(bind-prefix-key! "f" (lambda () (focus-next-window-in-frame!)))
(bind-prefix-key! "p" (lambda () (pull-window-from-other-frame!)))
(bind-prefix-key! "k" (lambda () (close-current-window!)))
(bind-prefix-key! "d" (lambda () (close-current-window!))) ; StumpWM delete-window
(bind-prefix-key! "g" (lambda () (gnext!)))
(bind-prefix-key! "G" (lambda () (gnew-auto!)))
(bind-prefix-key! "m" (lambda () (move-window-to-next-group!)))
(bind-prefix-key! "Q" (lambda () (wm-quit)))

;; Uncomment to use the Print key as prefix, like the user's StumpWM setup:
;; (set-prefix-key! '() "Print")

;; ---------------------------------------------------------------------
;; Standalone-session extras
;; ---------------------------------------------------------------------

;; Hardware/media keys work without the prefix. All of these just spawn
;; shell commands and are harmless no-ops if the tool isn't installed.
(bind-key! '() "XF86MonBrightnessUp"
           (lambda () (wm-spawn "brightnessctl set +5%")))
(bind-key! '() "XF86MonBrightnessDown"
           (lambda () (wm-spawn "brightnessctl set 5%-")))
(bind-key! '() "XF86AudioRaiseVolume"
           (lambda () (wm-spawn "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+")))
(bind-key! '() "XF86AudioLowerVolume"
           (lambda () (wm-spawn "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-")))
(bind-key! '() "XF86AudioMute"
           (lambda () (wm-spawn "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle")))

;; C-t R: reload this init file in place -- edit
;; ~/.config/minde/init.scm, hit C-t R, and the new bindings are live.
;; Errors during the reload are logged and leave the old state in place
;; for whatever was already defined.
(define (init-file-path)
  (or (getenv "MINDE_INIT")
      (string-append (getenv "HOME") "/.config/minde/init.scm")))

(define (reload-init!)
  (let ((path (init-file-path)))
    (if (file-exists? path)
        (begin
          (load path)
          (wm-log (string-append "reloaded " path)))
        (wm-log (string-append "no init file at " path)))))

(bind-prefix-key! "R" (lambda () (reload-init!)))

;; Screen lock (needs swaylock installed).
(bind-prefix-key! "L" (lambda () (wm-spawn "swaylock -c 282828")))

;; Programs to start once, when the compositor is up (first output ready).
;; Rust calls (wm-on-startup) if it is bound -- both nested and on the TTY.
(define %autostart
  '(;; "swaybg -m fill -i ~/Projects/images/wallpaper.png"
    ;; "foot --server"
    ))

(define (wm-on-startup)
  (for-each wm-spawn %autostart)
  (wm-log (format #f "autostart: ~a programs" (length %autostart))))

;; ---------------------------------------------------------------------
;; REPL server
;; ---------------------------------------------------------------------
;;
;; NOTE: spawn-server runs the REPL in its own thread inside Guile. State
;; mutated from a REPL connection (e.g. redefining %keybindings or
;; wm-handle-key) is therefore not synchronized with the compositor's main
;; thread/event loop -- fine for interactively poking at bindings, but be
;; aware of races if you mutate shared mutable state from both places at
;; once.

(define %repl-socket-path
  (string-append (or (getenv "XDG_RUNTIME_DIR") "/tmp")
                 "/minde-repl.sock"))

;; A failure to start the REPL server (e.g. unwritable runtime dir) should
;; not prevent the rest of the init file / compositor from working.
;; Guarded via an environment variable (not a define) so that reloading
;; this file with C-t R doesn't spawn a second server: top-level defines
;; are re-evaluated on load, but the process environment persists.
(unless (getenv "MINDE_REPL_STARTED")
  (catch #t
    (lambda ()
      ;; Remove a stale socket left over from a previous run, if any.
      (when (file-exists? %repl-socket-path)
        (delete-file %repl-socket-path))
      (spawn-server (make-unix-domain-server-socket #:path %repl-socket-path))
      (setenv "MINDE_REPL_STARTED" "1")
      (wm-log (string-append "REPL server listening at " %repl-socket-path)))
    (lambda (key . args)
      (wm-log (format #f "could not start REPL server at ~a: ~a ~a"
                       %repl-socket-path key args)))))

(wm-log "minde scheme layer loaded")
