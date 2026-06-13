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
right after the prefix key (C-t). THUNK may also be a keymap made with
`make-keymap` (StumpWM-style nested map): the next keypress is then looked
up in it."
  (hash-set! %prefix-bindings key thunk))

(define (make-keymap . bindings)
  "Builds a nested keymap from KEY THUNK pairs, bindable under a prefix
key: (bind-prefix-key! \"A\" (make-keymap \"c\" thunk1 \"d\" thunk2))."
  (let ((map (make-hash-table)))
    (let loop ((b bindings))
      (unless (null? b)
        (hash-set! map (car b) (cadr b))
        (loop (cddr b))))
    map))

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

;; wm-handle-key is a tiny state machine: 'normal (the usual case) or the
;; keymap currently awaiting a key -- %prefix-bindings right after the
;; prefix key, or a nested keymap (see make-keymap) after its parent key.
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
  (if (hash-table? %key-state)
      (cond
       ;; Pressing the prefix key's keysym again while awaiting a key
       ;; (e.g. C-t t) is StumpWM's "send this key literally" escape:
       ;; reset to normal state and forward the keypress to the focused
       ;; client instead of consuming it. Matched by keysym name only
       ;; (not modifiers) since the second press may or may not carry the
       ;; same modifier as the prefix itself. Only at the top level --
       ;; inside a nested keymap the prefix keysym is just another key.
       ((and (eq? %key-state %prefix-bindings)
             (string=? keysym-name %prefix-key))
        (set! %key-state 'normal)
        #f)
       ((modifier-keysym? keysym-name)
        #f) ; stay put; let the client see the modifier
       (else
        (let ((binding (hash-ref %key-state keysym-name)))
          (set! %key-state 'normal)
          (cond
           ;; A nested keymap: keep waiting, now in the submap.
           ((hash-table? binding)
            (set! %key-state binding))
           (binding
            (run-binding! binding mods-bitmask keysym-name))
           (else
            ;; Unbound key: swallow it and echo, like StumpWM.
            (echo (format #f "~a is not bound" keysym-name))))
          #t)))
      (if (and (= mods-bitmask %prefix-mods) (string=? keysym-name %prefix-key))
          (begin
            (set! %key-state %prefix-bindings)
            #t)
          (let ((thunk (hash-ref %keybindings (cons mods-bitmask keysym-name))))
            (if thunk
                (begin
                  (run-binding! thunk mods-bitmask keysym-name)
                  #t)
                #f)))))

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
;;   f -- next window (group-wide, like cycling emacs buffers)
;;   o -- next window within this frame's stack
;;   p -- pull next hidden window into this frame
;;   k/d -- close window          e -- lem editor
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
;; f: StumpWM `next` -- cycle through ALL of the group's windows,
;; emacs-buffer-style, jumping frames as needed.
(bind-prefix-key! "f" (lambda () (focus-next-window!)))
;; p: StumpWM `pull` -- dig the next hidden window out into this frame.
(bind-prefix-key! "p" (lambda () (pull-hidden-next!)))
;; o: cycle only within the current frame's own window stack.
(bind-prefix-key! "o" (lambda () (focus-next-window-in-frame!)))
(bind-prefix-key! "k" (lambda () (close-current-window!)))
(bind-prefix-key! "d" (lambda () (close-current-window!))) ; StumpWM delete-window
(bind-prefix-key! "g" (lambda () (gnext!)))
(bind-prefix-key! "G" (lambda ()
                        (let ((g (gnew-auto!)))
                          (echo (string-append "new group:" (group-name g))))))
;; y: StumpWM `info` -- echo the current window and group.
(bind-prefix-key! "y" (lambda ()
                        (let ((id (current-frame-window)))
                          (echo (if id
                                    (format #f "~a  [~a]" (window-title id)
                                            (string-trim-both (current-group-name)))
                                    "no window")))))
(bind-prefix-key! "m" (lambda () (move-window-to-next-group!)))
(bind-prefix-key! "Q" (lambda () (wm-quit)))

;; ---- Ported from the user's StumpWM config ----

;; w: voice dictation (script may still assume X tools internally).
(bind-prefix-key! "w" (lambda () (wm-spawn "~/Projects/System/scripts/voice-dictate.scm")))

;; i: eww desktop widgets (eww supports Wayland natively; the X-only
;; windowlower workaround from StumpWM is dropped).
(bind-prefix-key! "i" (lambda () (wm-spawn "eww open --toggle sysinfo")))

;; A: agents submap, like StumpWM's *agents-map*.
(bind-prefix-key! "A"
  (make-keymap
   "c" (lambda () (wm-spawn "alacritty -e ~/Projects/System/scripts/codex-guix.scm"))
   "d" (lambda () (wm-spawn "alacritty -e ~/Projects/System/scripts/claude-guix.scm"))
   "o" (lambda () (wm-spawn "alacritty -e ~/Projects/System/scripts/open-code-guix.scm"))
   "p" (lambda () (wm-spawn "alacritty -e ~/Projects/System/scripts/pi-guix.scm"))))

;; P: misc submap, like StumpWM's *misc-map*. Wayland substitutions:
;; swaybg for feh (wallpaper), wl-paste/wl-copy for xsel (clipboard).
(bind-prefix-key! "P"
  (make-keymap
   "w" (lambda ()
         (wm-spawn "pkill swaybg; swaybg -m fill -i \"$(ls ~/Projects/images/* | shuf -n1)\""))
   "a" (lambda ()
         (wm-spawn "sel=$(wl-paste); [ -n \"$sel\" ] && ASK_AI_SYSTEM='You are a copy editor. Improve grammar, clarity and flow. Keep the meaning and the original language. Output only the revised text.' ~/Projects/System/scripts/ask-ai.scm \"$sel\" | wl-copy && minde-msg 'clipboard updated'"))))

;; Prompt-driven workflows. The pattern: fuzzel --dmenu (an ordinary
;; async Wayland client, so the compositor never blocks) collects input,
;; and minde-msg/minde-cmd (REPL-socket helpers on PATH from the
;; package) push results back into the compositor.

;; a: ask-ai -- prompt, ask, echo the answer (sticky for 30s).
(bind-prefix-key! "a"
  (lambda ()
    (wm-spawn "q=$(: | fuzzel --dmenu -p 'Ask AI: '); [ -n \"$q\" ] && minde-msg -t 30000 \"$(~/Projects/System/scripts/ask-ai.scm \"$q\")\"")))

;; T: add-todo -- prompt, append to the org file, confirm.
(bind-prefix-key! "T"
  (lambda ()
    (wm-spawn "todo=$(: | fuzzel --dmenu -p 'TODO: '); [ -n \"$todo\" ] && ~/Projects/System/scripts/add-todo.scm \"$todo\" ~/Projects/WorkingMemory/wm.org && minde-msg \"added: $todo\"")))

;; l: windowlist -- pick a window with fuzzel, jump to it. The list is
;; written to a file so titles never touch shell quoting.
(define (windowlist!)
  (let ((pairs (window-ids-with-titles))
        (tmp (string-append (or (getenv "XDG_RUNTIME_DIR") "/tmp")
                            "/minde-windowlist")))
    (if (null? pairs)
        (echo "no windows")
        (begin
          (call-with-output-file tmp
            (lambda (p)
              (for-each (lambda (pr) (format p "~a  ~a~%" (car pr) (cdr pr)))
                        pairs)))
          (wm-spawn (string-append
                     "sel=$(fuzzel --dmenu -p 'window: ' < " tmp "); "
                     "id=${sel%% *}; "
                     "[ -n \"$id\" ] && minde-cmd \"((@ (minde frames) focus-window-by-id!) $id)\""))))))

(bind-prefix-key! "l" (lambda () (windowlist!)))

;; Not ported yet (missing infrastructure), from StumpWM:
;;   D/t dashboards -- interactive terminal scripts; run them in a
;;                     terminal yourself, or bind alacritty -e wrappers
;;   F  float-this             -- no floating layer
;;   z  zen (gaps + mode line) -- no gaps/mode-line
;;   S  screenshot map         -- needs wlr-screencopy (grim) support

;; Print as the prefix key, matching the user's StumpWM setup. Comment out
;; to fall back to the C-t default.
(set-prefix-key! '() "Print")

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
