;;; init.scm -- minde policy layer.
;;;
;;; Loaded by the Rust side (src/guile/mod.rs) once at startup. Redefine
;;; anything here live from the REPL (see below) to change behavior without
;;; restarting the compositor.

(use-modules (ice-9 hash-table)
             (srfi srfi-1)
             (system repl server))

;; scheme/frames.scm lives next to this file; add it to the load path so
;; `(use-modules (minde frames))` finds it regardless of cwd.
;; current-filename can be #f when this file is re-(load)ed at runtime
;; (Print R); the module dir is then already on the load path anyway
;; (MINDE_SCHEME_DIR, plus the initial load added it).
(let ((f (current-filename)))
  (when (string? f)
    (add-to-load-path (dirname f))))

(use-modules (minde frames)
             (minde groups)
             (minde input)
             (minde layouts)
             (minde hooks))

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

;; keymap object -> (key -> one-line doc), fed by the optional doc
;; argument of bind-prefix-key!; the ? which-key echo and describe-key
;; read it.
(define %binding-docs (make-hash-table))

(define (set-binding-doc! keymap key doc)
  (let ((tbl (or (hash-ref %binding-docs keymap)
                 (let ((t (make-hash-table)))
                   (hash-set! %binding-docs keymap t)
                   t))))
    (hash-set! tbl key doc)))

(define (binding-doc keymap key)
  (let ((tbl (hash-ref %binding-docs keymap)))
    (and tbl (hash-ref tbl key))))

(define* (bind-prefix-key! key thunk #:optional doc)
  "Bind KEY (e.g. \"s\", \"S\", \"c\") to THUNK, run when KEY is pressed
right after the prefix key (C-t). THUNK may also be a keymap made with
`make-keymap` (StumpWM-style nested map): the next keypress is then looked
up in it. DOC, when given, is a one-line description shown by the ?
which-key echo and describe-key."
  (hash-set! %prefix-bindings key thunk)
  (when doc (set-binding-doc! %prefix-bindings key doc)))

(define (keymap-help-string km)
  "The which-key text for KM: one 'key  doc' line per documented binding,
then one line listing the undocumented keys."
  (let ((documented '()) (bare '()))
    (hash-for-each
     (lambda (k v)
       (let ((d (binding-doc km k)))
         (if d
             (set! documented (cons (cons k d) documented))
             (set! bare (cons k bare)))))
     km)
    (string-join
     (append
      (map (lambda (p) (format #f "~a  ~a" (car p) (cdr p)))
           (sort documented (lambda (a b) (string<? (car a) (car b)))))
      (if (null? bare)
          '()
          (list (string-append "also: " (string-join (sort bare string<?) " ")))))
     "\n")))

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

;; Prefix indicator (StumpWM's pointer-box equivalent): the focus border
;; turns red while a prefix/submap is armed, yellow when idle.
(define %border-color-normal "#d79921")
(define %border-color-prefix "#cc241d")
(define %border-color-resize "#458588")

(define (set-border-color! hex)
  ;; Tolerate a binary older than this config (subr not registered yet).
  (let ((mod (resolve-module '(guile-user) #:ensure #f)))
    (let ((var (and mod (module-variable mod 'wm-border-color))))
      (when var ((variable-ref var) hex)))))

(define (set-key-state! s)
  (set! %key-state s)
  (set-border-color! (cond
                      ((eq? s 'normal) %border-color-normal)
                      ((eq? s %resize-map) %border-color-resize)
                      (else %border-color-prefix))))

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

;; StumpWM-style modifier-prefixed key specs for keymap bindings:
;; "M-Left", "C-0", "S-Right", "s-x" (C-=ctrl M-=alt S-=shift s-=super).
;; Prefixes are generated in this fixed order, so bind "C-M-x", never
;; "M-C-x". Shifted letters already arrive as their uppercase keysym
;; ("W"), so letter bindings don't need "S-".
(define (key-spec mods-bitmask keysym-name)
  (string-append (if (logtest mods-bitmask 4) "C-" "")
                 (if (logtest mods-bitmask 8) "M-" "")
                 (if (logtest mods-bitmask 1) "S-" "")
                 (if (logtest mods-bitmask 64) "s-" "")
                 keysym-name))

;; When set, the next key press is described instead of dispatched
;; (StumpWM describe-key); see the first branch of wm-handle-key.
(define %describe-next-key #f)

(define (describe-key!)
  (echo "describe: press a key...")
  (set! %describe-next-key
        (lambda (mods name)
          (let* ((spec (key-spec mods name))
                 (binding (or (hash-ref %prefix-bindings spec)
                              (hash-ref %prefix-bindings name)))
                 (shown (if (hash-ref %prefix-bindings spec) spec name)))
            (echo (cond
                   ((hash-table? binding)
                    (format #f "~a is a submap (prefix map)" shown))
                   (binding
                    (format #f "~a runs: ~a" shown
                            (or (binding-doc %prefix-bindings shown) "a binding")))
                   (else
                    (format #f "~a is not bound in the prefix map" spec))))))))

(define (wm-handle-key mods-bitmask keysym keysym-name . rest)
  "Called from Rust on every key press (4th argument: the UTF-8 text the
key produces under the active keymap, empty for Return/BackSpace/...).
Returns #t if the key was consumed (and should not be forwarded to the
focused client), #f otherwise."
  (define utf8 (if (pair? rest) (car rest) ""))
  (cond
   ;; describe-key armed: report on the next real key instead of
   ;; dispatching it.
   ((and %describe-next-key (not (modifier-keysym? keysym-name)))
    (let ((report %describe-next-key))
      (set! %describe-next-key #f)
      (report mods-bitmask keysym-name))
    #t)
   ;; A native input prompt is open: it owns the keyboard.
   ((input-active?)
    (if (modifier-keysym? keysym-name)
        #t
        (input-handle-key! mods-bitmask keysym-name utf8)))
   (else (dispatch-key mods-bitmask keysym-name))))

(define (dispatch-key mods-bitmask keysym-name)
  (if (hash-table? %key-state)
      (cond
       ;; Pressing the prefix key's keysym again while awaiting a key
       ;; (e.g. C-t t) is StumpWM's "send this key literally" escape:
       ;; reset to normal state and forward the keypress to the focused
       ;; client instead of consuming it. Matched by keysym name only
       ;; (not modifiers) since the second press may or may not carry the
       ;; same modifier as the prefix itself. Only at the top level --
       ;; inside a nested keymap the prefix keysym is just another key.
       ;; An explicit prefix-map binding for the prefix keysym wins over
       ;; the literal forward (e.g. Print Print below).
       ((and (eq? %key-state %prefix-bindings)
             (string=? keysym-name %prefix-key)
             (not (hash-ref %prefix-bindings keysym-name)))
        (set-key-state! 'normal)
        #f)
       ((modifier-keysym? keysym-name)
        #f) ; stay put; let the client see the modifier
       ;; ? echoes the armed keymap's bindings (which-key) and stays
       ;; armed, so you can look and then still press the key.
       ((string=? keysym-name "question")
        (let ((km %key-state))
          (set-key-state! km)
          (echo (keymap-help-string km)))
        #t)
       (else
        ;; Modifier-prefixed spec ("M-Left") wins over the bare name, so
        ;; e.g. arrows and M-arrows can coexist in one keymap.
        (let ((binding (or (and (positive? mods-bitmask)
                                (hash-ref %key-state (key-spec mods-bitmask keysym-name)))
                           (hash-ref %key-state keysym-name))))
          (set-key-state! 'normal)
          (cond
           ;; A nested keymap: keep waiting, now in the submap.
           ((hash-table? binding)
            (set-key-state! binding))
           (binding
            (let ((result (run-binding! binding mods-bitmask keysym-name)))
              (cond
               ;; A binding may return a keymap to stay armed in it --
               ;; that's how the iresize mode (%resize-map) keeps
               ;; accepting arrow presses without re-hitting the prefix.
               ((hash-table? result)
                (set-key-state! result))
               ;; The prefix keysym's own binding keeps the prefix
               ;; armed, so hammering Print cycles window after window
               ;; (Print Print Print ... = repeated Print-o). Any other
               ;; key resolves the prefix as usual.
               ((string=? keysym-name %prefix-key)
                (set-key-state! %prefix-bindings)))))
           (else
            ;; Unbound key: swallow it and echo, like StumpWM.
            (echo (format #f "~a is not bound" keysym-name))))
          #t)))
      (if (and (= mods-bitmask %prefix-mods) (string=? keysym-name %prefix-key))
          (begin
            (set-key-state! %prefix-bindings)
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
;; r: StumpWM exec -- native prompt with PATH completion (TAB). An
;; external launcher (fuzzel/bemenu) is no longer needed but still works
;; as a layer-shell client if you prefer one.
(bind-prefix-key! "r" (lambda () (run-prompt!)))
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
;; o: cycle only within the current frame's own window stack. Print Print
;; (prefix key twice) does the same -- nobody sends a literal Print to a
;; client anyway; rebind or unbind "Print" here to restore the StumpWM
;; literal-forward behavior.
(bind-prefix-key! "o" (lambda () (focus-next-window-in-frame!)))
(bind-prefix-key! "Print" (lambda () (focus-next-window-in-frame!)))
(bind-prefix-key! "k" (lambda () (close-current-window!)))
(bind-prefix-key! "d" (lambda () (close-current-window!))) ; StumpWM delete-window
(bind-prefix-key! "g" (lambda () (gnext!)))
;; G: new group -- prompts for a name; empty input auto-names (roman).
(bind-prefix-key! "G"
  (lambda ()
    (read-one-line "new group: "
      (lambda (name)
        (let ((g (if (string-null? name)
                     (gnew-auto!)
                     (gnew! (string-append " " name " ")))))
          (echo (string-append "new group:" (group-name g)))))
      #:history 'group)))
;; y: StumpWM `info` -- echo the current window and group.
(bind-prefix-key! "y" (lambda ()
                        (let ((id (current-frame-window)))
                          (echo (if id
                                    (format #f "~a  [~a]" (window-title id)
                                            (string-trim-both (current-group-name)))
                                    "no window")))))
(bind-prefix-key! "m" (lambda () (move-window-to-next-group!)))
(bind-prefix-key! "Q" (lambda () (wm-quit)))

;; ---------------------------------------------------------------------
;; Window numbers + navigation (StumpWM parity, conflict-free keys)
;; ---------------------------------------------------------------------

;; 0-9: jump to window by number; C-0..C-9: pull it here instead.
(for-each
 (lambda (n)
   (let ((k (number->string n)))
     (bind-prefix-key! k (lambda () (select-window-by-number! n)))
     (bind-prefix-key! (string-append "C-" k)
                       (lambda () (pull-window-by-number! n)))))
 (iota 10))

;; Arrows: directional frame focus; M-arrows move the window along;
;; S-arrows swap windows with the neighbor frame.
(for-each
 (lambda (pair)
   (let ((name (car pair)) (dir (cdr pair)))
     (bind-prefix-key! name (lambda () (move-focus! dir)))
     (bind-prefix-key! (string-append "M-" name) (lambda () (move-window! dir)))
     (bind-prefix-key! (string-append "S-" name) (lambda () (exchange-windows! dir)))))
 '(("Left" . left) ("Right" . right) ("Up" . up) ("Down" . down)))

;; Tab: toggle to the previously focused window (emacs C-x b RET);
;; S-Tab (keysym ISO_Left_Tab): toggle to the previous frame; u: toggle
;; to the previous group (StumpWM gother).
(bind-prefix-key! "Tab" (lambda () (other-window!)))
(bind-prefix-key! "ISO_Left_Tab" (lambda () (other-frame!)))
(bind-prefix-key! "u" (lambda () (gother!)))

;; W: StumpWM `windows` -- echo the numbered window list.
(bind-prefix-key! "W" (lambda () (echo (echo-windows-string))))

;; C: StumpWM `only` -- collapse all splits, keep every window;
;; Delete: fclear -- show this frame empty (its windows stay, hidden).
(bind-prefix-key! "C" (lambda () (only!)))
(bind-prefix-key! "Delete" (lambda () (fclear!)))

;; Reverse cycling, mirroring f/p/n/o forwards.
(bind-prefix-key! "C-f" (lambda () (focus-prev-window!)))
(bind-prefix-key! "C-p" (lambda () (pull-hidden-previous!)))
(bind-prefix-key! "C-n" (lambda () (focus-prev-frame!)))
(bind-prefix-key! "C-o" (lambda () (focus-prev-window-in-frame!)))

;; ---------------------------------------------------------------------
;; Help, eval prompt, message history, marks, group management
;; (StumpWM parity sprint 2)
;; ---------------------------------------------------------------------

;; While the prefix (or a submap) is armed, ? echoes its bindings and
;; stays armed -- docs for the existing map so that's actually useful:
(for-each
 (lambda (p) (set-binding-doc! %prefix-bindings (car p) (cdr p)))
 '(("Return" . "terminal") ("r" . "run program (prompt)")
   ("b" . "browser") ("e" . "lem") ("E" . "emacsclient")
   ("v" . "vsplit") ("h" . "hsplit") ("c" . "remove split")
   ("n" . "next frame") ("f" . "next window (group)")
   ("p" . "pull hidden window") ("o" . "next window in frame")
   ("k" . "close window") ("d" . "close window")
   ("g" . "next group") ("G" . "new group (prompt)")
   ("m" . "move window to next group") ("y" . "window info")
   ("l" . "windowlist (prompt)") ("a" . "ask AI (prompt)")
   ("T" . "add TODO (prompt)") ("w" . "voice dictate")
   ("i" . "eww widgets") ("A" . "agents submap") ("P" . "misc submap")
   ("R" . "reload init.scm") ("L" . "lock screen") ("Q" . "quit (asks)")
   ("s" . "resize mode") ("F" . "apply layout (prompt)")
   ("Tab" . "last window") ("ISO_Left_Tab" . "last frame")
   ("u" . "last group") ("W" . "window list echo")
   ("C" . "only: collapse splits") ("Delete" . "fclear: empty frame")))

;; F1: describe-key -- press it, then any key, get told what it does.
(bind-prefix-key! "F1" (lambda () (describe-key!)) "describe next key")

;; colon: eval a Scheme expression in the compositor (StumpWM's colon /
;; eval-line rolled into one -- our commands ARE Scheme).
(define (eval-prompt!)
  (read-one-line "eval: "
    (lambda (s)
      (unless (string-null? s)
        (catch #t
          (lambda ()
            (echo (format #f "~s" (eval (with-input-from-string s read)
                                        (interaction-environment)))))
          (lambda (key . args)
            (echo (format #f "error: ~a ~s" key args))))))
    #:history 'eval))
(bind-prefix-key! "colon" (lambda () (eval-prompt!)) "eval scheme (prompt)")

;; C-m: re-show the last message (StumpWM lastmsg).
(bind-prefix-key! "C-m"
  (lambda () (echo (or (last-message) "no messages")))
  "last message again")

;; Q now asks before quitting (StumpWM quit-confirm).
(bind-prefix-key! "Q"
  (lambda ()
    (read-one-line "quit minde? (yes/n) "
      (lambda (a) (when (member a '("y" "yes")) (wm-quit)))))
  "quit (asks)")

;; Marks: tag windows, then pull them all here at once. (Not on "t":
;; that must stay free as the literal-forward escape for the default
;; C-t prefix.)
(bind-prefix-key! "x" (lambda () (mark-window-toggle!)) "mark/unmark window")
(bind-prefix-key! "M-x" (lambda () (pull-marked!)) "pull marked windows here")
(bind-prefix-key! "C-x" (lambda () (clear-marks!)) "clear marks")

;; Group management: M-g select by name, M-m move-and-follow; grename /
;; gkill live in the P submap (see below).
(define (gselect!)
  (read-one-line "group: "
    (lambda (name)
      (unless (string-null? name)
        (switch-to-group! (string-append " " (string-trim-both name) " "))))
    #:completions (lambda () (map string-trim-both (group-names)))
    #:history 'group))
(bind-prefix-key! "M-g" (lambda () (gselect!)) "switch group (prompt)")
(bind-prefix-key! "M-m" (lambda () (gmove-and-follow!)) "move window + follow")

;; ---------------------------------------------------------------------
;; Timers, fullscreen, force kill, banish, urgency, clipboard
;; (StumpWM parity sprint 3)
;; ---------------------------------------------------------------------

;; One-shot timers: Rust arms a calloop timer and calls (wm-on-timer
;; token) back on the main thread; the token->thunk table lives here.
(define %timers (make-hash-table))
(define %next-timer-token 0)

(define (wm-run-after ms thunk)
  "Runs THUNK after MS milliseconds on the compositor's main thread
(StumpWM run-with-timer, one-shot)."
  (let ((token %next-timer-token))
    (set! %next-timer-token (+ token 1))
    (hash-set! %timers token thunk)
    ;; Tolerate a binary older than this config (subr not registered).
    (let* ((mod (resolve-module '(guile-user) #:ensure #f))
           (var (and mod (module-variable mod 'wm-run-after-ms))))
      (if var
          ((variable-ref var) ms token)
          (hash-remove! %timers token)))))

(define (wm-on-timer token)
  (let ((thunk (hash-ref %timers token)))
    (hash-remove! %timers token)
    (when thunk
      (catch #t
        thunk
        (lambda (key . args)
          (wm-log (format #f "error in timer: ~a ~a" key args)))))))

;; Fullscreen / force kill / banish (see (minde frames)).
(bind-prefix-key! "M-f" (lambda () (fullscreen!)) "fullscreen toggle")
(bind-prefix-key! "K" (lambda () (kill-current-window!)) "kill window (force)")
(bind-prefix-key! "B" (lambda () (banish!)) "banish pointer")

;; Frame indicator (StumpWM curframe): echo the frame geometry and pulse
;; the focus border bright for a moment.
(define (flash-current-frame!)
  (let ((r (current-frame-rect)))
    (echo (format #f "current frame: ~ax~a at ~a,~a"
                  (caddr r) (cadddr r) (car r) (cadr r))))
  (set-border-color! "#fabd2f")
  ;; set-key-state! re-derives the correct color for whatever state the
  ;; key machine is in by then.
  (wm-run-after 400 (lambda () (set-key-state! %key-state))))
(bind-prefix-key! "C-c" (lambda () (flash-current-frame!)) "flash current frame")

;; Urgency: Rust calls (wm-on-urgent id) on xdg-activation requests.
(define (wm-on-urgent id) (add-urgent-window! id))
(bind-prefix-key! "C-u" (lambda () (next-urgent!)) "jump to urgent window")

;; Clipboard. Paste is asynchronous: wm-request-paste (fired from the
;; prompt's C-y/C-v) makes Rust read the selection and call (wm-on-paste
;; text) when it has arrived; route it into the active prompt.
(define (wm-on-paste text)
  (if (and (string? text) (string-null? text))
      (when (input-active?) (echo "clipboard empty"))
      (input-paste! text)))

(define (set-clipboard! text)
  "Owns the clipboard selection with TEXT (StumpWM putsel)."
  (wm-set-clipboard text))

(define (copy-last-message!)
  (let ((m (last-message)))
    (if m
        (begin (set-clipboard! m) (echo "copied last message"))
        (echo "no messages"))))
(bind-prefix-key! "M-c" (lambda () (copy-last-message!)) "copy last message")

;; ---------------------------------------------------------------------
;; iresize: Print s enters an interactive resize mode (StumpWM iresize).
;; Arrows/hjkl move the nearest split divider by %resize-step pixels,
;; b/= balances all frames, Return/Escape leaves the mode. The mode works
;; by its bindings returning %resize-map, which dispatch-key re-arms
;; (border turns blue while armed).
;; ---------------------------------------------------------------------

(define %resize-step 30)

;; Guarded like set-border-color!: tolerate a binary/test without the
;; message subrs.
(define (call-if-bound name . args)
  (let* ((mod (resolve-module '(guile-user) #:ensure #f))
         (var (and mod (module-variable mod name))))
    (when var (apply (variable-ref var) args))))

(define %resize-help "Resize: arrows/hjkl move divider, b balance, RET/ESC done")

(define (resize-and-stay dir)
  (lambda ()
    (resize-frame! dir %resize-step)
    (call-if-bound 'wm-message %resize-help 0)
    %resize-map))

(define (exit-resize!)
  (call-if-bound 'wm-clear-message)
  #t)

(define %resize-map
  (make-keymap
   "Left"  (resize-and-stay 'left)
   "Right" (resize-and-stay 'right)
   "Up"    (resize-and-stay 'up)
   "Down"  (resize-and-stay 'down)
   "h"     (resize-and-stay 'left)
   "l"     (resize-and-stay 'right)
   "k"     (resize-and-stay 'up)
   "j"     (resize-and-stay 'down)
   "b"     (lambda () (balance-frames!)
                      (call-if-bound 'wm-message %resize-help 0)
                      %resize-map)
   "equal" (lambda () (balance-frames!)
                      (call-if-bound 'wm-message %resize-help 0)
                      %resize-map)
   "Return" (lambda () (exit-resize!))
   "Escape" (lambda () (exit-resize!))))

(bind-prefix-key! "s"
  (lambda ()
    (call-if-bound 'wm-message %resize-help 0)
    %resize-map))

;; ---------------------------------------------------------------------
;; Gaps: space between frames (inner) and against the screen edge
;; (outer). Off by default; uncomment to taste.
;; ---------------------------------------------------------------------

;; (set-gaps! 8 8)

;; ---------------------------------------------------------------------
;; Layouts: named frame-tree presets ((minde layouts)). Print F
;; prompts (TAB completes) and applies; Print P s saves the live tree
;; under a name to ~/.config/minde/layouts.scm, reloaded at startup.
;; Spec grammar: 'leaf | (hsplit ratio a b) | (vsplit ratio a b).
;; ---------------------------------------------------------------------

;; Main window on the left 2/3, a stack column on the right.
(define-layout! "main-side" '(hsplit 2/3 leaf leaf))
;; Main + right column split in two.
(define-layout! "main-stack" '(hsplit 2/3 leaf (vsplit 1/2 leaf leaf)))
;; Four equal quarters.
(define-layout! "grid4" '(vsplit 1/2 (hsplit 1/2 leaf leaf) (hsplit 1/2 leaf leaf)))
;; One full-screen frame.
(define-layout! "full" 'leaf)

;; Layouts saved with Print P s in earlier sessions.
(load-layouts!)

(define (layout-prompt!)
  (read-one-line "layout: "
    (lambda (name)
      (unless (string-null? name)
        (apply-layout! name)))
    #:completions layout-names
    #:history 'layout))

(bind-prefix-key! "F" (lambda () (layout-prompt!)))

;; ---------------------------------------------------------------------
;; Placement rules: route windows to a group/frame by app-id or title
;; substring when they map (StumpWM define-frame-preference). Examples:
;; ---------------------------------------------------------------------

;; (add-placement-rule! "zen" #:group "II" #:frame 0)
;; (add-placement-rule! "emacs" #:frame 1 #:follow? #t)

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

;; Random wallpaper via swaybg. Picks only files whose magic bytes say
;; real PNG/JPEG: mixed folders often hold webp-named-.png, READMEs,
;; videos... and one bad pick makes swaybg exit with no wallpaper at all.
(define %wallpaper-cmd
  (string-append
   "img=$(for f in ~/Projects/images/*; do "
   "case \"$(head -c 4 \"$f\" 2>/dev/null | od -An -tx1 | tr -d ' \\n')\" in "
   "89504e47|ffd8ff*) echo \"$f\";; esac; done | shuf -n1); "
   "[ -n \"$img\" ] && swaybg -m fill -i \"$img\""))

;; P: misc submap, like StumpWM's *misc-map*. Wayland substitutions:
;; swaybg for feh (wallpaper), wl-paste/wl-copy for xsel (clipboard).
(bind-prefix-key! "P"
  (make-keymap
   ;; s: save the current frame tree as a named layout (Print F applies).
   "s" (lambda ()
         (read-one-line "save layout as: "
           (lambda (name)
             (unless (string-null? name)
               (save-layout! name)))
           #:completions layout-names
           #:history 'layout))
   ;; r: rename the current group; k: delete it (windows move on).
   "r" (lambda ()
         (read-one-line "rename group: "
           (lambda (name) (grename! name))
           #:history 'group))
   "k" (lambda () (gkill!))
   "w" (lambda ()
         (wm-spawn (string-append "pkill swaybg; " %wallpaper-cmd)))
   "a" (lambda ()
         (wm-spawn "sel=$(wl-paste); [ -n \"$sel\" ] && ASK_AI_SYSTEM='You are a copy editor. Improve grammar, clarity and flow. Keep the meaning and the original language. Output only the revised text.' ~/Projects/System/scripts/ask-ai.scm \"$sel\" | wl-copy && minde-msg 'clipboard updated'"))))

;; Prompt-driven workflows, all through the native read-one-line prompt
;; ((minde input) module -- StumpWM's input.lisp semantics: emacs
;; editing keys, TAB prefix completion, C-p/C-n history, C-g/ESC abort).

;; POSIX single-quote escaping for splicing prompt input into wm-spawn
;; shell strings (port of StumpWM's sh-quote).
(define (sh-quote s)
  (string-append "'" (string-join (string-split s #\') "'\\''") "'"))

;; Executable names on PATH, cached after the first scan (completion for
;; the run prompt).
(define %path-commands #f)
(define (path-commands)
  (unless %path-commands
    (set! %path-commands
          (sort
           (delete-duplicates
            (append-map
             (lambda (dir)
               (catch #t
                 (lambda ()
                   (let ((d (opendir dir)))
                     (let loop ((acc '()))
                       (let ((e (readdir d)))
                         (if (eof-object? e)
                             (begin (closedir d) acc)
                             (loop (if (member e '("." "..")) acc (cons e acc))))))))
                 (lambda _ '())))
             (string-split (or (getenv "PATH") "") #\:)))
           string<?)))
  %path-commands)

(define (run-prompt!)
  (read-one-line "run: "
    (lambda (cmd) (unless (string-null? cmd) (wm-spawn cmd)))
    #:completions path-commands
    #:history 'run))

;; a: ask-ai -- prompt, ask, echo the answer (sticky for 30s).
(bind-prefix-key! "a"
  (lambda ()
    (read-one-line "Ask AI: "
      (lambda (q)
        (unless (string-null? q)
          (echo "thinking...")
          (wm-spawn (string-append
                     "minde-msg -t 30000 \"$(~/Projects/System/scripts/ask-ai.scm "
                     (sh-quote q) ")\""))))
      #:history 'ask-ai)))

;; T: add-todo -- prompt, append to the org file, confirm.
(bind-prefix-key! "T"
  (lambda ()
    (read-one-line "TODO: "
      (lambda (todo)
        (unless (string-null? todo)
          (wm-spawn (string-append
                     "~/Projects/System/scripts/add-todo.scm " (sh-quote todo)
                     " ~/Projects/WorkingMemory/wm.org && minde-msg "
                     (sh-quote (string-append "added: " todo))))))
      #:history 'todo)))

;; l: windowlist -- title prompt with completion; exact title wins, else
;; first prefix match.
(define (windowlist!)
  (let ((pairs (map (lambda (p)
                      (cons (car p)
                            (format #f "~a: ~a"
                                    (or (window-number (car p)) "?") (cdr p))))
                    (window-ids-with-titles))))
    (if (null? pairs)
        (echo "no windows")
        (read-one-line "window: "
          (lambda (sel)
            (let ((hit (or (find (lambda (p) (string=? (cdr p) sel)) pairs)
                           (find (lambda (p) (string-prefix? sel (cdr p))) pairs))))
              (if hit
                  (focus-window-by-id! (car hit))
                  (echo (string-append "no window: " sel)))))
          #:completions (map cdr pairs)
          #:history 'window))))

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
        (catch #t
          (lambda ()
            ;; 'absolute keeps (current-filename) meaningful inside the
            ;; loaded file (its add-to-load-path needs it; see the same
            ;; fluid in tests/keys-test.scm).
            (with-fluids ((%file-port-name-canonicalization 'absolute))
              (load path))
            (wm-log (string-append "reloaded " path))
            (echo (string-append "reloaded " path)))
          (lambda (key . args)
            (wm-log (format #f "reload FAILED: ~a ~s" key args))
            (echo (format #f "reload FAILED: ~a ~s" key args))))
        (echo (string-append "no init file at " path)))))

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
