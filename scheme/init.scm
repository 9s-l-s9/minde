;;; init.scm -- minde policy layer.
;;;
;;; Loaded by the Rust side (src/guile/mod.rs) once at startup. Redefine
;;; anything here live from the REPL (see below) to change behavior without
;;; restarting the compositor.

(use-modules (ice-9 hash-table)
             (srfi srfi-1)
             (system vm trace))

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
             (minde ui prompt)
             (minde ui menu)
             (minde layouts)
             (minde hooks)
             (minde commands)
             (minde command-catalog)
             (minde config)
             (minde status)
             (minde session)
             ((minde foundation keys) #:prefix key:))

;; Publish the versioned status document after every policy synchronization.
;; publish-status! suppresses unchanged states and keeps the historical
;; one-line file updated for existing eww configurations.
(set-sync-hook! publish-status!)

(define (call-with-scheme-backtrace context thunk handler)
  "Runs THUNK and logs a Guile backtrace before invoking HANDLER on error."
  (let ((stack #f))
    (catch #t
      (lambda ()
        (with-throw-handler #t thunk
          (lambda _ (set! stack (make-stack #t)))))
      (lambda (key . arguments)
        (let ((details
               (call-with-output-string
                (lambda (port)
                  (format port "~a: ~a ~s~%" context key arguments)
                  (when stack (display-backtrace stack port))))))
          (wm-log details)
          (apply handler key arguments))))))

;; UI engines are packageable state machines. The compositor supplies their
;; rendering/input side effects here; standalone tests inject simple lambdas.
(define (ui-rust-call name . arguments)
  (let* ((module (resolve-module '(guile-user) #:ensure #f))
         (variable (and module (module-variable module name))))
    (and variable (apply (variable-ref variable) arguments))))
(configure-prompt-ui!
 #:show (lambda (text duration) (ui-rust-call 'wm-message text duration))
 #:clear (lambda () (ui-rust-call 'wm-clear-message))
 #:set-key-repeat (lambda (enabled?) (ui-rust-call 'wm-set-key-repeat enabled?))
 #:request-paste (lambda () (ui-rust-call 'wm-request-paste))
 #:set-clipboard (lambda (text) (ui-rust-call 'wm-set-clipboard text)))
(configure-menu-ui!
 #:show (lambda (text duration) (ui-rust-call 'wm-message text duration))
 #:clear (lambda () (ui-rust-call 'wm-clear-message))
 #:set-key-repeat (lambda (enabled?) (ui-rust-call 'wm-set-key-repeat enabled?)))

;; ---------------------------------------------------------------------
;; Keybinding table
;; ---------------------------------------------------------------------

;; Global (non-prefixed) bindings: (mods . keysym-name) -> thunk. Kept
;; deliberately small -- StumpWM style is that almost everything goes
;; through the C-t prefix.
(define %keybindings (make-hash-table))

;; Global binding -> one-line description.  Keeping this beside the live
;; binding table lets the documentation generator enumerate the exact map
;; instead of maintaining a second hand-written key reference.
(define %global-binding-docs (make-hash-table))

;; Prefix-key bindings: keysym-name -> thunk. Looked up with no modifier
;; requirement beyond having entered prefix state (matching StumpWM, where
;; e.g. C-t c means "prefix, then plain c").
(define %prefix-bindings (make-hash-table))

;; X11-style modifier bitmask, mirrored on the Rust side: shift=1 ctrl=4
;; alt=8 super=64.
(define (mod-symbol->bit sym)
  (key:modifier->bit sym))

(define (mods->bitmask mods)
  "MODS is a list of symbols such as '(super) or '(ctrl shift)."
  (key:modifiers->bitmask mods))

(define* (bind-key! mods key thunk #:optional doc)
  "Bind KEY (an xkb keysym name string, e.g. \"Return\", \"q\", \"d\") with
modifier list MODS (e.g. '(super)) to THUNK, a zero-argument procedure run
when the binding fires. Global binding, always active."
  (let ((binding (cons (mods->bitmask mods) key)))
    (hash-set! %keybindings binding thunk)
    (when doc (hash-set! %global-binding-docs binding doc))))

;; keymap object -> (key -> one-line doc), fed by the optional doc
;; argument of bind-prefix-key!; the ? which-key echo and describe-key
;; read it.
(define %binding-docs (make-hash-table))

;; (PARENT KEY CHILD) triples identify nested maps even when a binding wraps
;; entry into the child with a small side effect (the w window listing and f
;; frame-number overlay).  The live objects remain authoritative; this table
;; only preserves their relationship for help/reference traversal.
(define %binding-submaps '())

(define (set-binding-submap! parent key child)
  (set! %binding-submaps
        (cons (list parent key child)
              (filter (lambda (entry)
                        (not (and (eq? parent (car entry))
                                  (string=? key (cadr entry)))))
                      %binding-submaps))))

(define (binding-submap parent key)
  (let ((entry (find (lambda (entry)
                       (and (eq? parent (car entry))
                            (string=? key (cadr entry))))
                     %binding-submaps)))
    (and entry (caddr entry))))

(define (copy-binding-submaps! source target)
  (for-each (lambda (entry)
              (when (eq? source (car entry))
                (set-binding-submap! target (cadr entry) (caddr entry))))
            (list-copy %binding-submaps)))

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
  (when (hash-table? thunk)
    (set-binding-submap! %prefix-bindings key thunk))
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

(define (make-documented-keymap . bindings)
  "Build a keymap from repeating KEY VALUE DOC triples."
  (let ((map (make-hash-table)))
    (let loop ((rest bindings))
      (unless (null? rest)
        (hash-set! map (car rest) (cadr rest))
        (set-binding-doc! map (car rest) (caddr rest))
        (loop (cdddr rest))))
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

;; which-key-mode (StumpWM): when on, an armed keymap that stays armed
;; for %which-key-delay-ms echoes its bindings automatically (the same
;; text ? shows on demand). The generation counter voids stale timers.
(define %which-key-mode #f)
(define %which-key-delay-ms 1000)
(define %keymap-generation 0)

(define (which-key-mode!)
  (set! %which-key-mode (not %which-key-mode))
  (echo (if %which-key-mode "which-key mode on" "which-key mode off")))

(define (set-key-repeat! on)
  ;; Tolerate a binary older than this config (subr not registered yet).
  (let ((mod (resolve-module '(guile-user) #:ensure #f)))
    (let ((var (and mod (module-variable mod 'wm-set-key-repeat))))
      (when var ((variable-ref var) on)))))

(define (set-key-state! s)
  (set! %key-state s)
  (set! %keymap-generation (+ %keymap-generation 1))
  ;; Keys an armed keymap consumes never repeat client-side; ask the
  ;; compositor to re-fire held keys (prompts toggle this themselves in
  ;; input.scm/menu.scm).
  (set-key-repeat! (hash-table? s))
  (when (and %which-key-mode (hash-table? s))
    (let ((gen %keymap-generation) (km s))
      (wm-run-after %which-key-delay-ms
                    (lambda ()
                      (when (and (= gen %keymap-generation)
                                 (eq? %key-state km))
                        (echo (keymap-help-string km)))))))
  (set-border-color! (cond
                      ((eq? s 'normal) %border-color-normal)
                      ((eq? s %resize-map) %border-color-resize)
                      (else %border-color-prefix))))

;; Last error a keybinding raised, for copy-unhandled-error (StumpWM):
;; grab it onto the clipboard instead of retyping it from the log.
(define %last-unhandled-error #f)

(define (run-binding! thunk mods keysym-name)
  ;; Errors from a binding are logged, not fatal -- one bad keybinding
  ;; shouldn't take down the compositor.
  (call-with-scheme-backtrace "keybinding failure"
    thunk
    (lambda (key . args)
      (set! %last-unhandled-error
            (format #f "error in keybinding ~a ~a: ~a ~s" mods keysym-name key args))
      (wm-log %last-unhandled-error))))

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
  (key:key-notation mods-bitmask keysym-name))

;; Command mode (StumpWM command-mode): while on, the prefix map stays
;; armed for EVERY key -- no prefix needed -- until Return/C-g/Escape
;; leaves it. Implemented by re-arming %prefix-bindings after each
;; dispatch (the same mechanism iresize uses for its own map).
(define %command-mode #f)

(define (command-mode!)
  (set! %command-mode #t)
  (set-key-state! %prefix-bindings)
  (echo "command mode: keys act as prefix keys (RET / C-g exits)"))

(define (leave-command-mode!)
  (set! %command-mode #f)
  (set-key-state! 'normal)
  (echo "command mode off"))

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
   ;; A menu is open (select-from-menu): same deal.
   ((menu-active?)
    (if (modifier-keysym? keysym-name)
        #t
        (menu-handle-key! mods-bitmask keysym-name utf8)))
   (else (dispatch-key mods-bitmask keysym-name))))

(define (dispatch-key mods-bitmask keysym-name)
  (if (hash-table? %key-state)
      (cond
       ;; Command mode ends on Return / C-g / Escape, whatever keymap
       ;; is currently armed.
       ((and %command-mode
             (or (string=? keysym-name "Return")
                 (string=? keysym-name "Escape")
                 (equal? (key-spec mods-bitmask keysym-name) "C-g")))
        (leave-command-mode!)
        #t)
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
        ;; e.g. arrows and M-arrows can coexist in one keymap. Shifted
        ;; letters arrive as their uppercase keysym, so a binding like
        ;; "M-G" is physically alt+shift+g: retry without the "S-"
        ;; prefix before falling back to the bare name.
        (let ((binding (or (and (positive? mods-bitmask)
                                (hash-ref %key-state (key-spec mods-bitmask keysym-name)))
                           (and (logtest mods-bitmask 1)
                                (positive? (logand mods-bitmask (lognot 1)))
                                (hash-ref %key-state
                                          (key-spec (logand mods-bitmask (lognot 1))
                                                    keysym-name)))
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
          ;; Command mode: whatever just ran, stay armed in the prefix
          ;; map (unless the binding armed a submap/mode of its own).
          (when (and %command-mode (not (hash-table? %key-state)))
            (set-key-state! %prefix-bindings))
          #t)))
      (if (and (= mods-bitmask %prefix-mods) (string=? keysym-name %prefix-key))
          (begin
            (set-key-state! %prefix-bindings)
            #t)
          (let ((thunk (hash-ref %keybindings (cons mods-bitmask keysym-name))))
            (cond
             (thunk
              (run-binding! thunk mods-bitmask keysym-name)
              #t)
             ;; Per-app remapped keys (StumpWM define-remapped-keys): a
             ;; key headed for the client may translate to a different
             ;; synthesized key for this app-id.
             ((and (not (modifier-keysym? keysym-name))
                   (remap-target (key-spec mods-bitmask keysym-name)))
              => (lambda (target)
                   (send-key target)
                   #t))
             (else #f))))))

;; ---------------------------------------------------------------------
;; Default bindings
;; ---------------------------------------------------------------------

;; Global: super+q quits outright (kept as an escape hatch outside the
;; prefix, mirroring many StumpWM configs' emergency exit).
(bind-key! '(super) "q"
           (lambda () (wm-quit))
           "emergency compositor quit")

;; The terminal is the only application launcher in the portable default.
(define (terminal-command)
  "Return the configured terminal command, with dependency-light fallbacks."
  (or (getenv "MINDE_TERMINAL") "foot || xterm"))

(bind-prefix-key! "Return" (lambda () (wm-spawn (terminal-command))))
;; r: StumpWM exec -- native prompt with PATH completion (TAB). An
;; external launcher (fuzzel/bemenu) is no longer needed but still works
;; as a layer-shell client if you prefer one.
(bind-prefix-key! "r" (lambda () (run-prompt!)))
;; Dynamic groups auto-tile; manual splits are refused there (StumpWM).
(define (guard-manual-tiling thunk)
  (if (dynamic-group?)
      (echo (format #f "~a is a dynamic group"
                    (string-trim-both (current-group-name))))
      (thunk)))

(bind-prefix-key! "v" (lambda () (guard-manual-tiling split-frame-vertical!)))
(bind-prefix-key! "h" (lambda () (guard-manual-tiling split-frame-horizontal!)))
(bind-prefix-key! "c" (lambda () (guard-manual-tiling remove-split!)))
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
(bind-prefix-key! "g" (lambda () (switch-to-next-group!)))
;; G: new group -- prompts for a name; empty input auto-names (roman).
(bind-prefix-key! "G"
  (lambda ()
    (read-one-line "new group: "
      (lambda (name)
        (let ((g (if (string-null? name)
                     (create-auto-named-group!)
                     (create-group! (string-append " " name " ")))))
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
(bind-prefix-key! "u" (lambda () (switch-to-last-group!)))

;; W: StumpWM `windows` -- echo the numbered window list.
(bind-prefix-key! "W" (lambda () (echo (echo-windows-string))))

;; C: StumpWM `only` -- collapse all splits, keep every window;
;; Delete: fclear -- show this frame empty (its windows stay, hidden).
(bind-prefix-key! "C" (lambda () (collapse-to-one-frame!)))
(bind-prefix-key! "Delete" (lambda () (clear-current-frame!)))

;; Reverse cycling, mirroring f/p/n/o forwards.
(bind-prefix-key! "C-f" (lambda () (focus-previous-window!)))
(bind-prefix-key! "C-p" (lambda () (pull-hidden-previous!)))
(bind-prefix-key! "C-n" (lambda () (focus-previous-frame!)))
(bind-prefix-key! "C-o" (lambda () (focus-previous-window-in-frame!)))

;; ---------------------------------------------------------------------
;; Help, eval prompt, message history, marks, group management
;; (StumpWM parity sprint 2)
;; ---------------------------------------------------------------------

;; While the prefix (or a submap) is armed, ? echoes its bindings and
;; stays armed -- docs for the existing map so that's actually useful:
(for-each
 (lambda (p) (set-binding-doc! %prefix-bindings (car p) (cdr p)))
 '(("Return" . "terminal") ("r" . "run program (prompt)")
   ("v" . "vsplit") ("h" . "hsplit") ("c" . "remove split")
   ("n" . "next frame") ("f" . "next window (group)")
   ("p" . "pull hidden window") ("o" . "next window in frame")
   ("k" . "close window") ("d" . "close window")
   ("g" . "next group") ("G" . "new group (prompt)")
   ("m" . "move window to next group") ("y" . "window info")
   ("l" . "windowlist (prompt)") ("P" . "misc submap")
   ("R" . "reload init.scm") ("L" . "lock screen") ("Q" . "quit (asks)")
   ("s" . "resize mode") ("F" . "float/unfloat window")
   ("M-F" . "apply layout (prompt)")
   ("Tab" . "last window") ("ISO_Left_Tab" . "last frame")
   ("u" . "last group") ("W" . "window list echo")
   ("C" . "only: collapse splits") ("Delete" . "fclear: empty frame")
   ("z" . "command mode (RET/C-g exits)")))

;; F1: describe-key -- press it, then any key, get told what it does.
(bind-prefix-key! "F1" (lambda () (describe-key!)) "describe next key")

;; z: command mode -- every key acts as a prefix key until RET/C-g.
(bind-prefix-key! "z" (lambda () (command-mode!)) "command mode (RET/C-g exits)")

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

;; Q now asks before quitting (StumpWM quit-confirm); logout! (minde
;; session) owns the actual prompt-then-wm-quit shape, so this, the
;; portable "s q" binding, and the colon prompt all agree on one
;; implementation.
(bind-prefix-key! "Q" (lambda () (logout!)) "quit (asks)")

;; ---------------------------------------------------------------------
;; Session management (minde session): L locks; M-L suspends (locks
;; first and waits for confirmation -- see the module for why). Logout
;; is deliberately not on its own single keystroke anywhere: Q above
;; and the portable "s q" binding both go through logout!'s confirmation
;; prompt.
;; ---------------------------------------------------------------------
(bind-prefix-key! "L" (lambda () (lock-screen!)) "lock screen")
(bind-prefix-key! "M-L" (lambda () (suspend!)) "suspend (locks first)")

;; Marks: tag windows, then pull them all here at once. (Not on "t":
;; that must stay free as the literal-forward escape for the default
;; C-t prefix.)
(bind-prefix-key! "x" (lambda () (mark-window-toggle!)) "mark/unmark window")
(bind-prefix-key! "M-x" (lambda () (pull-marked!)) "pull marked windows here")
(bind-prefix-key! "C-x" (lambda () (clear-marks!)) "clear marks")

;; Group management: M-g select by name, M-m move-and-follow; grename /
;; gkill live in the P submap (see below).
(define (gselect!)
  (select-from-menu
   (map (lambda (name) (cons (string-trim-both name) name)) (group-names))
   switch-to-group!
   #:prompt "groups:"))
(bind-prefix-key! "M-g" (lambda () (gselect!)) "switch group (prompt)")
(bind-prefix-key! "M-m" (lambda () (move-current-window-to-next-group-and-follow!)) "move window + follow")

;; ---------------------------------------------------------------------
;; Timers, fullscreen, force kill, banish, urgency, clipboard
;; (StumpWM parity sprint 3)
;; ---------------------------------------------------------------------

;; One-shot timers: Rust arms a calloop timer and calls (handle-timer!
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

(define (handle-timer! token)
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
(bind-prefix-key! "B" (lambda () (move-pointer-to-corner!)) "banish pointer")

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

;; Urgency: Rust calls (handle-urgent-window! id) on xdg-activation requests.
(define (handle-urgent-window! id) (add-urgent-window! id))
(bind-prefix-key! "C-u" (lambda () (next-urgent!)) "jump to urgent window")

;; Foreign-toplevel management (wlr-foreign-toplevel-management): external
;; bars/switchers request actions on windows by their compositor id. Rust
;; (src/handlers/foreign_toplevel.rs) looks these up by plain top-level
;; name, as with the other event entry points, so they live here as thin
;; glue over the exported group/frame operations rather than as public
;; module API. `close` is handled entirely in Rust (a shell close).
(define (handle-foreign-activate! id)
  "Rust: a taskbar/switcher asked to activate window ID. Switches to the
group holding it (if any) and focuses it; a no-op if it has since vanished."
  (let ((group (find (lambda (name) (group-has-window? name id)) (group-names))))
    (when group
      (unless (string=? group (current-group-name))
        (switch-to-group! group))
      (focus-window-by-id! id))))

(define (handle-foreign-fullscreen! id on)
  "Rust: a taskbar asked to (un)fullscreen window ID. Activates the window,
then drives the single-fullscreen model (fullscreen-window / fullscreen!)
so at most one window is fullscreen and the frame layout re-syncs on exit."
  (handle-foreign-activate! id)
  (let ((current (fullscreen-window)))
    (cond
     ;; Want it fullscreen and it isn't: clear any other, then set this one.
     ((and on (not (equal? current id)))
      (when current (fullscreen!)) ; clears the currently-fullscreen window
      (fullscreen!))               ; focused window is now ID -> fullscreen it
     ;; Want it un-fullscreened and it currently is: toggle off.
     ((and (not on) (equal? current id))
      (fullscreen!)))))

(define (handle-foreign-minimize! id on)
  "Rust: a taskbar asked to (un)minimize window ID. minde's tiling model
has no minimized state and never advertises one, so this is intentionally a
no-op; defined so the request is acknowledged rather than reaching an
unbound-variable path."
  (values id on))

;; Clipboard. Paste is asynchronous: wm-request-paste (fired from the
;; prompt's C-y/C-v) makes Rust read the selection and call (handle-paste!
;; text) when it has arrived; route it into the active prompt.
(define (handle-paste! text)
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
;; Multi-head (multi-monitor): each group has a frame tree per head;
;; S cycles heads, M-s toggles to the last one. Directional focus
;; (prefix arrows) also crosses monitor edges. Users who prefer one big
;; tree spanning all monitors can (set-head-mode! 'span).
;;
;; These two are development-only: install-portable-keymap! rebuilds the
;; map from scratch, so the release default binds heads under "o" instead.
;; ---------------------------------------------------------------------

(bind-prefix-key! "S" (lambda () (focus-next-head!)) "next head (monitor)")
(bind-prefix-key! "M-s" (lambda () (focus-last-head!)) "last head (monitor)")

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

(define (enter-resize-mode!)
  (call-if-bound 'wm-message %resize-help 0)
  %resize-map)

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

;; F floats/unfloats (the user's old StumpWM float-this binding); the
;; layout prompt moved to M-F.
(bind-prefix-key! "F"
  (lambda ()
    (float-this!)
    ;; Floating in/out of a dynamic group changes the tiled set.
    (when (dynamic-group?) (retile-dynamic!)))
  "float/unfloat window")
(bind-prefix-key! "M-F" (lambda () (layout-prompt!)) "apply layout (prompt)")

;; ---------------------------------------------------------------------
;; Placement rules: route windows to a group/frame by app-id or title
;; substring when they map (StumpWM define-frame-preference). Examples:
;; ---------------------------------------------------------------------

;; (add-placement-rule! "zen" #:group "II" #:frame 0)
;; (add-placement-rule! "emacs" #:frame 1 #:follow? #t)

;; Miscellaneous built-in operations. Personal launchers and desktop policy
;; belong in a user configuration layered over this portable file.
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
           (lambda (name) (rename-current-group! name))
           #:history 'group))
   "k" (lambda () (delete-current-group!))
   ;; n: rename the current window (StumpWM title).
   "n" (lambda ()
         (read-one-line "window name: "
           (lambda (name) (rename-window! name))))
   ;; p: re-apply placement rules to existing windows.
   "p" (lambda () (place-existing-windows!))
   ;; f: unfloat every float of this group; t: always-on-top toggle;
   ;; y: always-show (sticky, follows every group switch).
   "f" (lambda () (flatten-floats!))
   "t" (lambda () (toggle-always-on-top!))
   "y" (lambda () (toggle-always-show!))
   ;; i: window properties echo; d: date; V: version; M: modifiers.
   "i" (lambda () (show-window-properties!))
   "d" (lambda () (echo-date!))
   "V" (lambda () (version!))
   "M" (lambda () (modifiers!))
   ;; u: unmaximize toggle; g: gravity of the unmaximized window.
   "u" (lambda () (unmaximize!))
   "g" (lambda ()
         (read-one-line "gravity: "
           (lambda (s)
             (unless (string-null? s)
               (set-window-gravity! (string->symbol s))))
           #:completions %gravity-names))
   ;; R/F: remember/forget a persistent placement rule for this window.
   "R" (lambda () (remember!))
   "F" (lambda () (forget!))
   ;; D/O: dump / restore the whole desktop (groups + frame layouts).
   "D" (lambda ()
         (read-one-line "dump desktop to: "
           (lambda (path)
             (unless (string-null? path) (dump-desktop-to-file path)))
           #:initial (string-append (getenv "HOME") "/.config/minde/desktop.scm")))
   "O" (lambda ()
         (read-one-line "restore desktop from: "
           (lambda (path)
             (unless (string-null? path) (restore-from-file path)))
           #:initial (string-append (getenv "HOME") "/.config/minde/desktop.scm")))))

;; Prompt-driven workflows, all through the native read-one-line prompt
;; ((minde ui prompt) module -- StumpWM's input.lisp semantics: emacs
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

;; l: windowlist -- navigable menu (C-n/C-p/digits/Return; typing
;; filters by title), StumpWM select-window style.
(define (windowlist!)
  (let ((items (map (lambda (p)
                      (cons (format #f "~a~a ~a"
                                    (or (window-number (car p)) "?")
                                    (cond ((equal? (car p) (focused-window-id)) "*")
                                          ((window-floating? (car p)) "~")
                                          (else " "))
                                    (cdr p))
                            (car p)))
                    (window-ids-with-titles))))
    (if (null? items)
        (echo "no windows")
        (select-from-menu items focus-window-by-id! #:prompt "windows:"))))

(bind-prefix-key! "l" (lambda () (windowlist!)))

;; ---------------------------------------------------------------------
;; Group & window management parity (StumpWM sprint 8)
;; ---------------------------------------------------------------------

;; Windowlist variants: current frame only, by class, pull instead of
;; jump. All reuse the select-from-menu machinery.
(define (window-label id)
  (format #f "~a~a ~a"
          (or (window-number id) "?")
          (cond ((equal? id (focused-window-id)) "*")
                ((window-floating? id) "~")
                (else " "))
          (window-title id)))

(define (frame-windowlist!)
  (let ((items (map (lambda (id) (cons (window-label id) id))
                    (current-frame-window-ids))))
    (if (null? items)
        (echo "no windows in this frame")
        (select-from-menu items focus-window-by-id! #:prompt "frame windows:"))))

(define (echo-frame-windows!)
  (let ((ids (current-frame-window-ids)))
    (echo (if (null? ids)
              "no windows in this frame"
              (string-join (map window-label ids) "\n")))))

(define (windowlist-by-class!)
  (let* ((ids (sort (all-window-ids)
                    (lambda (a b)
                      (string<? (or (window-app-id a) "?")
                                (or (window-app-id b) "?")))))
         (items (map (lambda (id)
                       (cons (format #f "~a | ~a"
                                     (or (window-app-id id) "?")
                                     (window-title id))
                             id))
                     ids)))
    (if (null? items)
        (echo "no windows")
        (select-from-menu items focus-window-by-id! #:prompt "windows by class:"))))

(define (pull-from-windowlist!)
  (let ((items (map (lambda (id) (cons (window-label id) id))
                    (all-window-ids))))
    (if (null? items)
        (echo "no windows")
        (select-from-menu items pull-window-by-id! #:prompt "pull window:"))))

(define (select-floating-window!)
  (let ((items (map (lambda (id) (cons (window-title id) id))
                    (group-floats (current-group)))))
    (if (null? items)
        (echo "no floating windows")
        (select-from-menu items focus-window-by-id! #:prompt "floats:"))))

;; Exact title match wins over the first substring match, like StumpWM's
;; select-window-by-name.
(define (select-window-by-name!)
  (read-one-line "select window: "
    (lambda (name)
      (unless (string-null? name)
        (let* ((ids (all-window-ids))
               (id (or (find (lambda (i) (string=? (window-title i) name)) ids)
                       (find (lambda (i) (string-contains (window-title i) name))
                             ids))))
          (if id
              (focus-window-by-id! id)
              (echo (format #f "no window matching ~s" name))))))
    #:completions (lambda () (map window-title (all-window-ids)))
    #:history 'window))

;; Group prompts over the menu, targeting any group but the current one.
(define (other-group-menu prompt action)
  (let ((items (filter-map
                (lambda (name)
                  (and (not (string=? name (current-group-name)))
                       (cons (string-trim-both name) name)))
                (group-names))))
    (if (null? items)
        (echo "no other groups")
        (select-from-menu items action #:prompt prompt))))

(define (gmerge-prompt!) (other-group-menu "merge group here:" merge-group-into-current!))
(define (gmove-marked-prompt!)
  (other-group-menu "move marked to:" move-marked-windows-to-group!))

(define (gnewbg-prompt!)
  (read-one-line "new background group: "
    (lambda (name)
      (unless (string-null? name)
        (create-group-in-background! (string-append " " name " "))
        (echo (groups-echo-string))))
    #:history 'group))

(define (gnewbg-float-prompt!)
  (read-one-line "new background float group: "
    (lambda (name)
      (unless (string-null? name)
        (create-floating-group-in-background! (string-append " " name " "))
        (echo (groups-echo-string))))
    #:history 'group))

;; Misc echoes.
(define (echo-date!)
  (echo (strftime "%a %d %b %Y %H:%M:%S" (localtime (current-time)))))

(define (version!)
  (echo "minde 0.1.0"))

(define (modifiers!)
  (echo (format #f "key spec prefixes: C- control, M- alt, S- shift, s- super~%prefix key: ~a" %prefix-key)))

(define (redisplay!)
  "Re-places every window of the active group (StumpWM redisplay /
refresh)."
  (sync-frames!)
  (echo "redisplayed"))

;; Bindings: windowlist variants at the top level, group management in
;; the M-G submap, window odds and ends in the P submap.
(bind-prefix-key! "C-l" (lambda () (frame-windowlist!)) "frame windowlist (menu)")
(bind-prefix-key! "M-l" (lambda () (windowlist-by-class!)) "windowlist by class (menu)")
(bind-prefix-key! "M-p" (lambda () (pull-from-windowlist!)) "pull from windowlist (menu)")
(bind-prefix-key! "M-w" (lambda () (select-window-by-name!)) "select window by name (prompt)")
(bind-prefix-key! "M-t" (lambda () (select-floating-window!)) "select floating window (menu)")
(bind-prefix-key! "C-r" (lambda () (redisplay!)) "redisplay windows")

;; Dynamic groups (StumpWM sprint 11): prompts and per-group commands.
(define (gnew-dynamic-prompt!)
  (read-one-line "new dynamic group: "
    (lambda (name)
      (unless (string-null? name)
        (create-dynamic-group! (string-append " " name " "))))
    #:history 'group))

(define (gnewbg-dynamic-prompt!)
  (read-one-line "new background dynamic group: "
    (lambda (name)
      (unless (string-null? name)
        (create-dynamic-group-in-background! (string-append " " name " "))
        (echo (groups-echo-string))))
    #:history 'group))

(define (change-layout-prompt!)
  (read-one-line "master layout (left/right/top/bottom): "
    (lambda (s)
      (unless (string-null? s)
        (change-layout! (string->symbol s))))
    #:completions '("left" "right" "top" "bottom")))

(define (change-split-ratio-prompt!)
  (read-one-line "master ratio (0.1-0.9): "
    (lambda (s)
      (let ((r (string->number s)))
        (if r
            (change-split-ratio! (inexact->exact r))
            (echo "not a number"))))))

(bind-prefix-key! "M-G"
  (make-keymap
   "n" (lambda () (gnewbg-prompt!))
   "N" (lambda () (gnewbg-float-prompt!))
   "m" (lambda () (gmerge-prompt!))
   "M" (lambda () (gmove-marked-prompt!))
   "o" (lambda () (delete-other-groups!))
   "g" (lambda () (echo (groups-echo-string)))
   "f" (lambda () (shift-current-window-to-next-group!))
   "b" (lambda () (shift-current-window-to-previous-group!))
   "k" (lambda () (kill-windows-current-group!))
   "K" (lambda () (kill-windows-other!))
   "d" (lambda () (gnew-dynamic-prompt!))
   "D" (lambda () (gnewbg-dynamic-prompt!))
   "r" (lambda () (rotate-windows! 'forward))
   "C-r" (lambda () (rotate-windows! 'backward))
   "s" (lambda () (rotate-stack! 'forward))
   "x" (lambda () (exchange-with-master!))
   "l" (lambda () (change-layout-prompt!))
   "S" (lambda () (change-split-ratio-prompt!))
   "t" (lambda () (retile-dynamic-group!)))
  "groups submap")
(for-each
 (lambda (p) (set-binding-doc! (hash-ref %prefix-bindings "M-G") (car p) (cdr p)))
 '(("n" . "new group in background (prompt)")
   ("N" . "new float group in background (prompt)")
   ("m" . "merge a group here (menu)")
   ("M" . "move marked windows to a group (menu)")
   ("o" . "kill all other groups")
   ("g" . "group list echo")
   ("f" . "next group, taking the window")
   ("b" . "previous group, taking the window")
   ("k" . "close all windows in this group")
   ("K" . "close all windows in other groups")
   ("d" . "new dynamic group (prompt)")
   ("D" . "new dynamic group in background (prompt)")
   ("r" . "rotate windows through master")
   ("C-r" . "rotate windows backward")
   ("s" . "rotate the stack")
   ("x" . "exchange window with master")
   ("l" . "master layout (prompt)")
   ("S" . "master split ratio (prompt)")
   ("t" . "retile")))

;; ---------------------------------------------------------------------
;; Frames & placement parity (StumpWM sprint 9): fselect, expose,
;; sibling, uniform splits, rules persistence, desktop dump/restore
;; ---------------------------------------------------------------------

;; fselect: number labels appear in every frame; a digit jumps there.
;; Works via the same return-a-keymap mechanism as iresize.
(define %fselect-map
  (let ((km (make-keymap)))
    (for-each
     (lambda (n)
       (hash-set! km (number->string n)
                  (lambda ()
                    (clear-frame-overlays!)
                    (focus-frame-by-index! n))))
     (iota 10))
    (for-each
     (lambda (k) (hash-set! km k (lambda () (clear-frame-overlays!) #t)))
     '("Escape" "Return" "C-g"))
    km))

(define (fselect!)
  (show-frame-overlays!)
  %fselect-map)

;; expose: grid of one window per frame, digit picks, layout restored.
(define %expose-map
  (let ((km (make-keymap)))
    (for-each
     (lambda (n)
       (hash-set! km (number->string n) (lambda () (expose-pick! n))))
     (iota 10))
    (for-each
     (lambda (k) (hash-set! km k (lambda () (expose-pick! #f) #t)))
     '("Escape" "Return" "C-g"))
    km))

(define (expose-mode!)
  (if (expose-enter!) %expose-map #t))

(bind-prefix-key! "j" (lambda () (fselect!)) "fselect: jump to frame by number")
(bind-prefix-key! "M-e" (lambda () (expose-mode!)) "expose: pick window from grid")
(bind-prefix-key! "M-o" (lambda () (focus-sibling-frame!)) "sibling frame")

;; Uniform splits (StumpWM hsplit/vsplit-uniformly): split the current
;; frame into N equal parts.
(define (split-uniformly-prompt! orientation)
  (read-one-line (if (eq? orientation 'horizontal)
                     "hsplit into n frames: "
                     "vsplit into n frames: ")
    (lambda (s)
      (let ((n (string->number s)))
        (if (and n (exact-integer? n) (> n 1) (<= n 9))
            (if (eq? orientation 'horizontal)
                (hsplit-equally! n)
                (vsplit-equally! n))
            (echo "need a count between 2 and 9"))))))

(bind-prefix-key! "M-h"
  (lambda () (guard-manual-tiling
              (lambda () (split-uniformly-prompt! 'horizontal))))
  "hsplit uniformly (prompt)")
(bind-prefix-key! "M-v"
  (lambda () (guard-manual-tiling
              (lambda () (split-uniformly-prompt! 'vertical))))
  "vsplit uniformly (prompt)")

;; Gravity names for the P g prompt.
(define %gravity-names
  '("center" "top" "bottom" "left" "right"
    "top-left" "top-right" "bottom-left" "bottom-right"))

;; Placement rules saved by remember!/forget! in earlier sessions.
(load-placement-rules!)

;; ---------------------------------------------------------------------
;; Keys & help parity (StumpWM parity sprint 10)
;; ---------------------------------------------------------------------

;; send-raw-key: capture the next key press and synthesize it into the
;; focused window (for keys the wm would otherwise act on).
(define (send-raw-key!)
  (echo "send key: press a key...")
  (set! %describe-next-key
        (lambda (mods name) (send-key (key-spec mods name)))))
(bind-prefix-key! "C-q" (lambda () (send-raw-key!)) "send next key to window")

;; Help family: describe-command / where-is work over the binding docs
;; (%binding-docs); describe-function / describe-variable introspect the
;; live Guile modules.
(define (prefix-doc-pairs)
  (let ((tbl (hash-ref %binding-docs %prefix-bindings)))
    (if tbl (hash-map->list cons tbl) '())))

(define (commands!)
  "Echoes every registered canonical command."
  (echo
   (string-join
    (map (lambda (name)
           (let ((command (command-ref name)))
             (format #f "~a  ~a" name (command-summary command))))
         (command-names))
    "\n")))

(define (describe-command!)
  (read-one-line "describe command: "
    (lambda (s)
      (let ((hit (find (lambda (p) (string=? (cdr p) s)) (prefix-doc-pairs))))
        (if hit
            (echo (format #f "~a -- bound to ~a ~a" (cdr hit) %prefix-key (car hit)))
            (echo (format #f "no command matching ~s" s)))))
    #:completions (lambda () (sort (map cdr (prefix-doc-pairs)) string<?))))

(define (where-is!)
  (read-one-line "where is (doc substring): "
    (lambda (s)
      (let ((hits (filter (lambda (p)
                            (string-contains (string-downcase (cdr p))
                                             (string-downcase s)))
                          (prefix-doc-pairs))))
        (if (null? hits)
            (echo (format #f "nothing matching ~s" s))
            (echo (string-join
                   (map (lambda (p)
                          (format #f "~a ~a  ~a" %prefix-key (car p) (cdr p)))
                        (sort hits (lambda (a b) (string<? (car a) (car b)))))
                   "\n")))))))

(define %describe-modules
  '((guile-user) (minde frames) (minde groups) (minde ui prompt)
    (minde ui menu) (minde layouts) (minde hooks)))

(define (lookup-symbol-value name)
  "Two values: found? and the value of NAME across the wm's modules."
  (let ((sym (string->symbol name)))
    (let loop ((mods %describe-modules))
      (if (null? mods)
          (values #f #f)
          (let* ((mod (resolve-module (car mods) #:ensure #f))
                 (var (and mod (module-variable mod sym))))
            (if (and var (variable-bound? var))
                (values #t (variable-ref var))
                (loop (cdr mods))))))))

(define (truncate-for-echo s)
  (if (> (string-length s) 400) (string-append (substring s 0 400) "...") s))

(define (describe-function!)
  (read-one-line "describe function: "
    (lambda (s)
      (call-with-values (lambda () (lookup-symbol-value s))
        (lambda (found? v)
          (cond
           ((not found?) (echo (format #f "~a is unbound" s)))
           ((not (procedure? v))
            (echo (format #f "~a is not a procedure: ~s" s v)))
           (else
            (echo (format #f "~a: ~a" s
                          (or (procedure-documentation v) "no docstring"))))))))))

(define (describe-variable!)
  (read-one-line "describe variable: "
    (lambda (s)
      (call-with-values (lambda () (lookup-symbol-value s))
        (lambda (found? v)
          (if found?
              (echo (truncate-for-echo (format #f "~a = ~s" s v)))
              (echo (format #f "~a is unbound" s))))))))

(bind-prefix-key! "F2" (lambda () (describe-command!)) "describe command (prompt)")
(bind-prefix-key! "F3" (lambda () (commands!)) "list all commands")
(bind-prefix-key! "F4" (lambda () (describe-function!)) "describe function (prompt)")
(bind-prefix-key! "F5" (lambda () (where-is!)) "where-is (doc search)")
(bind-prefix-key! "F6" (lambda () (describe-variable!)) "describe variable (prompt)")

;; copy-unhandled-error: the last keybinding error onto the clipboard.
(define (copy-unhandled-error!)
  (if %last-unhandled-error
      (begin
        (wm-set-clipboard %last-unhandled-error)
        (echo "unhandled error copied to clipboard"))
      (echo "no unhandled error recorded")))

;; StumpWM load-module: pull a Guile module into the wm session at
;; runtime, e.g. (load-module! "ice-9 format"). add-to-load-path first
;; if it lives outside the default %load-path.
(define (load-module! name)
  (catch #t
    (lambda ()
      (module-use! (current-module)
                   (resolve-interface
                    (map string->symbol (string-split name #\space))))
      (echo (format #f "loaded module (~a)" name)))
    (lambda (key . args)
      (echo (format #f "load-module failed: ~a ~s" key args)))))

;; Colon/REPL-callable, unbound by default (like StumpWM): which-key-mode!,
;; define-remapped-keys!, toggle-remapped-keys!, unbind-remapped-keys!,
;; send-key / meta / send-escape, ratrelwarp, load-module!,
;; copy-unhandled-error!, reload-configuration!.

;; Not ported yet (missing infrastructure), from StumpWM:
;;   D/t dashboards -- interactive terminal scripts; run them in a
;;                     terminal yourself, or bind alacritty -e wrappers
;;   (command-mode is Print z)
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
           (lambda () (wm-spawn "brightnessctl set +5%"))
           "increase brightness 5%")
(bind-key! '() "XF86MonBrightnessDown"
           (lambda () (wm-spawn "brightnessctl set 5%-"))
           "decrease brightness 5%")
(bind-key! '() "XF86AudioRaiseVolume"
           (lambda () (wm-spawn "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+"))
           "increase volume 5%")
(bind-key! '() "XF86AudioLowerVolume"
           (lambda () (wm-spawn "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"))
           "decrease volume 5%")
(bind-key! '() "XF86AudioMute"
           (lambda () (wm-spawn "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"))
           "toggle audio mute")

;; ---------------------------------------------------------------------
;; Portable prefix map
;; ---------------------------------------------------------------------

(define (command-palette!)
  "Select and invoke a registered command that takes no arguments."
  (let ((items
         (filter-map
          (lambda (name)
            (let ((command (command-ref name)))
              (and (null? (command-arguments command))
                   (cons (format #f "~a — ~a" name (command-summary command))
                         name))))
          (command-names))))
    (select-from-menu items invoke-command #:prompt "command: ")))

(define (bind-portable-key! key procedure doc)
  (when (hash-ref %prefix-bindings key)
    (error "duplicate portable key binding" key))
  (bind-prefix-key! key procedure doc))

(define (install-portable-keymap!)
  "Replace the accumulated development bindings with the release default."
  (set! %prefix-bindings (make-hash-table))
  (set! %binding-docs (make-hash-table))
  (set! %binding-submaps '())
  (for-each
   (lambda (entry)
     (let ((key (car entry)) (direction (cadr entry)))
       (bind-portable-key! key (lambda () (move-focus! direction)) "focus frame")))
   '(("h" left) ("j" down) ("k" up) ("l" right)))
  (bind-portable-key! "n" focus-next-window! "next window")
  (bind-portable-key! "p" pull-hidden-next! "pull next hidden window")
  (for-each
   (lambda (number)
     (let ((key (number->string number)))
       (bind-portable-key! key (lambda () (select-window-by-number! number)) "select numbered window")
       ))
   (iota 10))
  (bind-portable-key! "Return" (lambda () (wm-spawn (terminal-command))) "terminal")
  (bind-portable-key! "r" run-prompt! "run command")
  (bind-portable-key! "Space" command-palette! "command palette")
  (bind-portable-key! "colon" eval-prompt! "evaluate Scheme")
  (let* ((pull-map
          (make-documented-keymap
           "n" pull-hidden-next! "pull next hidden window"
           "p" pull-hidden-previous! "pull previous hidden window"))
         (window-map
          (make-documented-keymap
           "n" focus-next-window! "next window"
           "p" focus-previous-window! "previous window"
           "c" close-current-window! "close window"
           "f" float-this! "float/unfloat window"
           "w" windowlist! "window list"
           "u" pull-map "pull hidden window"
           "0" (lambda () (pull-window-by-number! 0)) "pull window 0"
           "1" (lambda () (pull-window-by-number! 1)) "pull window 1"
           "2" (lambda () (pull-window-by-number! 2)) "pull window 2"
           "3" (lambda () (pull-window-by-number! 3)) "pull window 3"
           "4" (lambda () (pull-window-by-number! 4)) "pull window 4"
           "5" (lambda () (pull-window-by-number! 5)) "pull window 5"
           "6" (lambda () (pull-window-by-number! 6)) "pull window 6"
           "7" (lambda () (pull-window-by-number! 7)) "pull window 7"
           "8" (lambda () (pull-window-by-number! 8)) "pull window 8"
           "9" (lambda () (pull-window-by-number! 9)) "pull window 9")))
    (for-each
     (lambda (entry)
       (let ((key (car entry)) (direction (cadr entry)))
         (hash-set! window-map key (lambda () (move-window! direction)))
         (set-binding-doc! window-map key "move window")))
     '(("h" left) ("j" down) ("k" up) ("l" right)))
    (bind-portable-key! "w" window-map "window commands"))
  (let ((window-map (hash-ref %prefix-bindings "w")))
    (bind-prefix-key!
     "w"
     (lambda ()
       (echo (echo-windows-string))
       window-map)
     "window commands")
    (set-binding-submap! %prefix-bindings "w" window-map))
  (let ((exchange-map
         (make-documented-keymap
          "h" (lambda () (exchange-windows! 'left)) "exchange left"
          "j" (lambda () (exchange-windows! 'down)) "exchange down"
          "k" (lambda () (exchange-windows! 'up)) "exchange up"
          "l" (lambda () (exchange-windows! 'right)) "exchange right")))
    (bind-portable-key! "x" exchange-map "exchange windows")
    (bind-portable-key! "f"
         (make-documented-keymap
          "h" (lambda () (guard-manual-tiling split-frame-horizontal!)) "split horizontally"
          "v" (lambda () (guard-manual-tiling split-frame-vertical!)) "split vertically"
          "c" (lambda () (guard-manual-tiling remove-split!)) "remove split"
          "o" collapse-to-one-frame! "collapse to one frame"
          "e" clear-current-frame! "empty frame"
          "0" (lambda () (focus-frame-by-index! 0)) "select frame 0"
          "1" (lambda () (focus-frame-by-index! 1)) "select frame 1"
          "2" (lambda () (focus-frame-by-index! 2)) "select frame 2"
          "3" (lambda () (focus-frame-by-index! 3)) "select frame 3"
          "4" (lambda () (focus-frame-by-index! 4)) "select frame 4"
          "5" (lambda () (focus-frame-by-index! 5)) "select frame 5"
          "6" (lambda () (focus-frame-by-index! 6)) "select frame 6"
          "7" (lambda () (focus-frame-by-index! 7)) "select frame 7"
          "8" (lambda () (focus-frame-by-index! 8)) "select frame 8"
          "9" (lambda () (focus-frame-by-index! 9)) "select frame 9")
         "frame commands"))
  (let ((frame-map (hash-ref %prefix-bindings "f")))
    (bind-prefix-key!
     "f"
     (lambda ()
       (show-frame-overlays!)
       (wm-run-after 2000 clear-frame-overlays!)
       frame-map)
     "frame commands")
    (set-binding-submap! %prefix-bindings "f" frame-map))
  (bind-portable-key! "g"
         (make-documented-keymap
          "n" switch-to-next-group! "next group"
          "p" switch-to-previous-group! "previous group"
          "o" switch-to-last-group! "last group"
          "s" gselect! "select group"
          "m" move-current-window-to-next-group-and-follow! "move window and follow")
         "group commands")
  (bind-portable-key! "o"
         (make-documented-keymap
          "n" focus-next-head! "next head (monitor)"
          "p" focus-previous-head! "previous head (monitor)"
          "o" focus-last-head! "last head (monitor)"
          "s" (lambda () (set-head-mode! 'span)) "span heads (one tree over all monitors)"
          "h" (lambda () (set-head-mode! 'per-head)) "per-head mode (a tree per monitor)")
         "output (monitor) commands")
  (bind-portable-key! "m"
         (make-documented-keymap
          "l" layout-prompt! "select layout"
          "r" enter-resize-mode! "resize mode"
          "c" command-mode! "command mode")
         "layout and mode commands")
  (bind-portable-key! "s"
    (make-documented-keymap
     "r" (lambda () (reload-configuration!)) "reload configuration"
     "l" (lambda () (lock-screen!)) "lock screen"
     ;; z: sleep -- zzz, and free of "s" (the submap key itself, which
     ;; would be confusing to reuse) and "l" (lock) and "r" (reload).
     "z" (lambda () (suspend!)) "suspend (locks first)"
     "q" (lambda () (logout!)) "log out (asks)")
    "session commands"))

(unless (or (getenv "MINDE_FULL_KEYMAP")
            (getenv "MINDE_E2E_LEGACY_KEYMAP"))
  (install-portable-keymap!))

;; Declarative configuration is parsed and validated without evaluation. A
;; candidate binding table is built completely before these globals are
;; swapped, so a malformed reload cannot partially redefine the session.
(define %configuration-base-bindings #f)
(define %configuration-base-docs #f)

(define (copy-hash-table table)
  (let ((copy (make-hash-table)))
    (hash-for-each (lambda (key value) (hash-set! copy key value)) table)
    copy))

(define (configuration-file-path)
  (or (getenv "MINDE_CONFIG")
      (let ((directory (or (getenv "MINDE_SCHEME_DIR")
                           (let ((file (current-filename)))
                             (and (string? file) (dirname file)))
                           "scheme")))
        (string-append directory "/default-config.scm"))))

(define (capture-configuration-base!)
  (unless %configuration-base-bindings
    (set! %configuration-base-bindings (copy-hash-table %prefix-bindings))
    (set! %configuration-base-docs
          (let ((docs (hash-ref %binding-docs %prefix-bindings)))
            (if docs (copy-hash-table docs) (make-hash-table))))))

(define (register-configuration-layer!)
  "Makes the bindings currently installed by a user init layer the atomic
reload baseline. Call once after adding imperative user bindings."
  (set! %configuration-base-bindings (copy-hash-table %prefix-bindings))
  (set! %configuration-base-docs
        (let ((docs (hash-ref %binding-docs %prefix-bindings)))
          (if docs (copy-hash-table docs) (make-hash-table))))
  #t)

(define (apply-configuration! config)
  (capture-configuration-base!)
  (let ((candidate (copy-hash-table %configuration-base-bindings))
        (candidate-docs (copy-hash-table %configuration-base-docs)))
    (for-each
     (lambda (binding)
       (let* ((key (car binding))
              (command (command-ref (cadr binding))))
         (unless (null? (command-arguments command))
           (error "key binding command requires arguments" (command-name command)))
         (hash-set! candidate key (command-procedure command))
         (hash-set! candidate-docs key (command-summary command))))
     (configuration-bindings config))
    ;; No validation or allocation that can reasonably fail remains after
    ;; this point: publish the complete candidate as one short commit step.
    (copy-binding-submaps! %prefix-bindings candidate)
    (set! %prefix-bindings candidate)
    (hash-set! %binding-docs candidate candidate-docs)
    (set-prefix-key! (configuration-prefix-modifiers config)
                     (configuration-prefix-key config))))

(define (reload-configuration!)
  (let ((path (configuration-file-path)))
    (call-with-scheme-backtrace "configuration reload failure"
      (lambda ()
        (let ((candidate (validate-configuration-file path)))
          (apply-configuration! candidate)
          (wm-log (string-append "reloaded configuration " path))
          (echo (string-append "reloaded configuration " path))
          #t))
      (lambda (key . arguments)
        (let ((message (format #f "configuration reload FAILED: ~a ~s"
                               key arguments)))
          (wm-log message)
          (echo message)
          #f)))))

(clear-command-registry!)
(register-builtin-command! 'switch-to-next-group! switch-to-next-group!)
(register-builtin-command! 'switch-to-previous-group! switch-to-previous-group!)
(register-builtin-command! 'switch-to-last-group! switch-to-last-group!)
(register-builtin-command! 'focus-next-frame! focus-next-frame!)
(register-builtin-command! 'focus-previous-frame! focus-previous-frame!)
(register-builtin-command! 'focus-next-window! focus-next-window!)
(register-builtin-command! 'focus-previous-window! focus-previous-window!)
(register-builtin-command! 'pull-hidden-next! pull-hidden-next!)
(register-builtin-command! 'split-frame-horizontal! split-frame-horizontal!)
(register-builtin-command! 'split-frame-vertical! split-frame-vertical!)
(register-builtin-command! 'remove-split! remove-split!)
(register-builtin-command! 'clear-current-frame! clear-current-frame!)
(register-builtin-command! 'collapse-to-one-frame! collapse-to-one-frame!)
(register-builtin-command! 'focus-next-head! focus-next-head!)
(register-builtin-command! 'focus-last-head! focus-last-head!)
(register-builtin-command! 'reload-configuration! reload-configuration!)

(define (refresh-command-help!)
  ;; Fill key help from registry summaries wherever a binding is a command.
  (define (visit keymap)
    (hash-for-each
     (lambda (key value)
       (if (procedure? value)
           (let ((command
                  (find (lambda (name)
                          (eq? value (command-procedure (command-ref name))))
                        (command-names))))
             (when command
               (set-binding-doc! keymap key
                                 (command-summary (command-ref command)))))
           (when (hash-table? value) (visit value))))
     keymap))
  (visit %prefix-bindings))

(refresh-command-help!)

(reload-configuration!)

;; Rust calls this after the first output is ready. The portable default has no
;; autostart policy; user configurations may replace this procedure.
(define (handle-startup!)
  (wm-log "portable configuration: no autostart programs"))

;; ---------------------------------------------------------------------
;; Main-thread IPC and opt-in unsafe REPL
;; ---------------------------------------------------------------------
;;
;; Rust calls this from its calloop-owned Unix socket source. Reading,
;; evaluation and every resulting compositor mutation therefore happen on
;; the event-loop thread. The returned string is a one-datum response.
(define (minde-ipc-eval source)
  (call-with-output-string
    (lambda (port)
      (call-with-scheme-backtrace "IPC evaluation failure"
        (lambda ()
          (let ((datum
                 (call-with-input-string source
                   (lambda (input)
                     (let ((value (read input)))
                       (unless (eof-object? (read input))
                         (error "IPC accepts exactly one datum"))
                       value)))))
            (write (list 'ok (eval datum (interaction-environment))) port)))
        (lambda (key . arguments)
          (write (list 'error key arguments) port))))))

;; The Guile REPL server owns another thread and can race policy mutation.
;; Keep it solely as an explicit development escape hatch.
(when (getenv "MINDE_UNSAFE_REPL")
  (use-modules (system repl server)))

(define %repl-socket-path
  (string-append (or (getenv "XDG_RUNTIME_DIR") "/tmp")
                 "/minde-repl.sock"))

;; A failure to start the REPL server (e.g. unwritable runtime dir) should
;; not prevent the rest of the init file / compositor from working.
;; Guarded via an environment variable (not a define) so that reloading
;; this file with C-t R doesn't spawn a second server: top-level defines
;; are re-evaluated on load, but the process environment persists.
(when (and (getenv "MINDE_UNSAFE_REPL")
           (not (getenv "MINDE_REPL_STARTED")))
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
