;;; init.scm -- minde policy layer.
;;;
;;; Loaded by the Rust side (src/guile/mod.rs) once at startup. Redefine
;;; anything here live from the REPL (see below) to change behavior without
;;; restarting the compositor.

(use-modules (ice-9 hash-table)
             (srfi srfi-1))

;; The minde modules live below this file; add the directory to the load
;; path so public facades and private compositor modules resolve regardless of
;; cwd.
;; current-filename can be #f when this file is re-(load)ed at runtime
;; through a personal reload command; the module dir is already on the load path
;; (MINDE_SCHEME_DIR, plus the initial load added it).
(let ((f (current-filename)))
  (when (string? f)
    (add-to-load-path (dirname f))))

(use-modules (minde compositor callbacks)
             (minde compositor frames)
             (minde groups)
             (minde ui prompt)
             (minde ui menu)
             (minde layouts)
             (minde hooks)
             (minde commands)
             (minde control)
             (minde command-catalog)
             (minde config)
             (minde status)
             (minde session)
             (minde compositor rust)
             ((minde foundation keys) #:prefix key:))

;; Publish the versioned status document after every policy synchronization.
;; publish-status! suppresses unchanged states and keeps the historical
;; one-line file updated for existing eww configurations.
(set-sync-hook! (lambda ()
                  (note-desktop-change!)
                  (publish-status!)))

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
  (apply rust-call-if-bound name arguments))
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
(define (mods->bitmask mods)
  "MODS is a list of symbols such as '(super) or '(ctrl shift)."
  (key:modifiers->bitmask mods))

(define* (bind-key! mods key thunk #:optional doc)
  "Bind KEY (an xkb keysym name string, e.g. \"Return\", \"q\", \"d\") with
modifier list MODS (e.g. '(super)) to THUNK, a zero-argument procedure run
when the binding fires, or a nested keymap. Global binding, always active."
  (let ((binding (cons (mods->bitmask mods) key)))
    (hash-set! %keybindings binding thunk)
    (set-binding-submap! %keybindings binding (and (hash-table? thunk) thunk))
    (if doc (hash-set! %global-binding-docs binding doc)
        (hash-remove! %global-binding-docs binding))))

;; keymap object -> (key -> one-line doc), fed by the optional doc
;; argument of bind-prefix-key!; the ? which-key echo and describe-key
;; read it.
(define %binding-docs (make-weak-key-hash-table))

;; Parent map -> (key -> child map), including wrappers that show a window
;; list or frame labels on entry. Weak parents let replaced configuration
;; trees and their metadata be collected after a reload.
(define %binding-submaps (make-weak-key-hash-table))

(define (set-binding-submap! parent key child)
  (let ((children (hash-ref %binding-submaps parent)))
    (cond (child
           (unless children
             (set! children (make-hash-table))
             (hash-set! %binding-submaps parent children))
           (hash-set! children key child))
          (children (hash-remove! children key)))))

(define (binding-submap parent key)
  (let ((children (hash-ref %binding-submaps parent)))
    (and children (hash-ref children key))))

(define (copy-binding-submaps! source target)
  (let ((children (hash-ref %binding-submaps source)))
    (when children
      (hash-for-each
       (lambda (key child)
         (when (and (hash-ref source key)
                    (eq? (hash-ref source key) (hash-ref target key)))
           (set-binding-submap! target key child)))
       children))))

(define (set-binding-doc! keymap key doc)
  (if doc
      (let ((tbl (or (hash-ref %binding-docs keymap)
                     (let ((t (make-hash-table)))
                       (hash-set! %binding-docs keymap t)
                       t))))
        (hash-set! tbl key doc))
      (let ((tbl (hash-ref %binding-docs keymap)))
        (when tbl (hash-remove! tbl key)))))

(define (binding-doc keymap key)
  (let ((tbl (hash-ref %binding-docs keymap)))
    (and tbl (hash-ref tbl key))))

(define* (bind-prefix-key! key thunk #:optional doc)
  "Bind KEY (e.g. \"s\", \"S\", \"c\") to THUNK, run when KEY is pressed
right after the prefix key (C-t). THUNK may also be a keymap made with
`make-keymap` (StumpWM-style nested map): the next keypress is then looked
up in it. DOC, when given, is a one-line description shown by the ?
which-key echo and describe-key. Omitting DOC removes any previous description."
  (hash-set! %prefix-bindings key thunk)
  (set-binding-submap! %prefix-bindings key (and (hash-table? thunk) thunk))
  (set-binding-doc! %prefix-bindings key doc))

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
(define %keymap-cancel-actions (make-weak-key-hash-table))

(define (set-keymap-cancel! keymap action)
  "Declares ACTION to clean up when KEYMAP is left. Nested maps sharing the
same action belong to one modal operation and may transition without cleanup."
  (hashq-set! %keymap-cancel-actions keymap action))

(define (keymap-cancel-action state)
  (and (hash-table? state) (hashq-ref %keymap-cancel-actions state)))

(define (finish-keymap-state! previous next)
  (let ((cancel (keymap-cancel-action previous)))
    (when (and cancel (not (eq? cancel (keymap-cancel-action next))))
      (run-binding! cancel 0 "cancel"))))

(define (cancel-keymap!)
  (set! %command-mode #f)
  (set! %describe-next-key #f)
  (when (hash-table? %key-state) (set-key-state! 'normal)))

(set-frame-selection-cancel! cancel-keymap!)

;; Prefix indicator (StumpWM's pointer-box equivalent): the focus border
;; turns red while a prefix/submap is armed, yellow when idle.
(define %border-color-normal "#d79921")
(define %border-color-prefix "#cc241d")
(define %border-color-resize "#458588")

(define (set-border-color! hex)
  ;; Tolerate a binary older than this config (subr not registered yet).
  (rust-call-if-bound 'wm-border-color hex))

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
  (rust-call-if-bound 'wm-set-key-repeat on))

(define (refresh-key-border!)
  (set-border-color! (cond
                      ((eq? %key-state 'normal) %border-color-normal)
                      ((eq? %key-state %resize-map) %border-color-resize)
                      (else %border-color-prefix))))

(define* (set-key-state! s #:key (cancel? #t))
  (let ((previous %key-state))
    (set! %key-state s)
    (when cancel? (finish-keymap-state! previous s)))
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
  (refresh-key-border!))

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
   ;; Prompts and menus consume text, but modifiers must reach Smithay's
   ;; forwarding path so clients and input-method grabs receive their state.
   ((input-active?)
    (if (modifier-keysym? keysym-name)
        #f
        (input-handle-key! mods-bitmask keysym-name utf8)))
   ;; A menu is open (select-from-menu): same deal.
   ((menu-active?)
    (if (modifier-keysym? keysym-name)
        #f
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
                 ;; C-g without allocating the spec: ctrl held, no other
                 ;; notated modifier (key-spec ignores lock bits too).
                 (and (string=? keysym-name "g")
                      (logtest mods-bitmask 4)
                      (not (logtest mods-bitmask (logior 1 8 64))))))
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
       ;; the literal forward (e.g. Print Print in the optional legacy map).
       ((and (eq? %key-state %prefix-bindings)
             (string=? keysym-name %prefix-key)
             (not (hash-ref %prefix-bindings keysym-name)))
        (if %command-mode (leave-command-mode!) (set-key-state! 'normal))
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
        (let ((previous %key-state)
              (top-level? (eq? %key-state %prefix-bindings))
              (binding (or (and (positive? mods-bitmask)
                                (hash-ref %key-state (key-spec mods-bitmask keysym-name)))
                           (and (logtest mods-bitmask 1)
                                (positive? (logand mods-bitmask (lognot 1)))
                                (hash-ref %key-state
                                          (key-spec (logand mods-bitmask (lognot 1))
                                                    keysym-name)))
                           (hash-ref %key-state keysym-name))))
          ;; A pick must inspect its transient layout before cleanup. Run the
          ;; old map's cancel action after dispatch, unless a child continues it.
          (set-key-state! 'normal #:cancel? #f)
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
               ;; A top-level binding for the prefix keysym keeps the prefix
               ;; armed, so hammering Print cycles window after window
               ;; (Print Print Print ... = repeated Print-o). Any other
               ;; key resolves the prefix as usual.
               ((and top-level? (string=? keysym-name %prefix-key))
                (set-key-state! %prefix-bindings)))))
           (else
            ;; Unbound key: swallow it and echo, like StumpWM.
            (echo (format #f "~a is not bound" keysym-name))))
          ;; Command mode: whatever just ran, stay armed in the prefix
          ;; map (unless the binding armed a submap/mode of its own).
          (when (and %command-mode (not (hash-table? %key-state)))
            (set-key-state! %prefix-bindings))
          (finish-keymap-state! previous %key-state)
          #t)))
      (if (and (= mods-bitmask %prefix-mods) (string=? keysym-name %prefix-key))
          (begin
            (set-key-state! %prefix-bindings)
            #t)
          (let ((thunk (hash-ref %keybindings (cons mods-bitmask keysym-name))))
            (cond
             (thunk
              (let ((result (if (hash-table? thunk) thunk
                                (run-binding! thunk mods-bitmask keysym-name))))
                (when (hash-table? result) (set-key-state! result)))
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
;; Dynamic groups auto-tile; manual splits are refused there (StumpWM).
(define (guard-manual-tiling thunk)
  (if (dynamic-group?)
      (echo (format #f "~a is a dynamic group"
                    (string-trim-both (current-group-name))))
      (thunk)))

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

;; Group management: M-g select by name, M-m move-and-follow; grename /
;; gkill live in the P submap (see below).
(define (gselect!)
  (select-from-menu
   (map (lambda (name) (cons (string-trim-both name) name)) (group-names))
   switch-to-group!
   #:prompt "groups:"))

;; ---------------------------------------------------------------------
;; Timers, fullscreen, force kill, banish, urgency, clipboard
;; (StumpWM parity)
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
    (let ((var (rust-variable 'wm-run-after-ms)))
      (if var
          ((variable-ref var) ms token)
          (begin (hash-remove! %timers token) #f)))))

(define (handle-timer! token)
  (let ((thunk (hash-ref %timers token)))
    (hash-remove! %timers token)
    (when thunk
      (catch #t
        thunk
        (lambda (key . args)
          (wm-log (format #f "error in timer: ~a ~a" key args)))))))

;; Session operations are security-sensitive and therefore use an explicit,
;; validated capability boundary instead of resolving wm-* names dynamically.
;; This belongs after wm-run-after is defined, while still completing during
;; init.scm loading before any keybinding can invoke lock/suspend/logout.
((@@ (minde session) configure-session-runtime!)
 #:spawn wm-spawn
 #:quit wm-quit
 #:run-after wm-run-after
 #:session-locked? wm-session-lock-confirmed?)

;; ---------------------------------------------------------------------
;; libinput device configuration (per-device tap-to-click, natural
;; scrolling, acceleration profile, click method).
;;
;; `wm-input-devices` and the low-level `wm-configure-input-rule!` are Rust
;; primitives (src/guile/mod.rs); they are not part of any frozen public
;; module. Under the nested winit backend there is no libinput context, so
;; `wm-input-devices` returns '() and rules are stored but configure
;; nothing. On the udev/DRM backend a stored rule is applied to every
;; matching device as it arrives (hotplug) and re-applied immediately to
;; devices already present. Settings a device does not support are logged
;; and skipped, never fatal.
;; ---------------------------------------------------------------------

(define (%guile-user-var name)
  (rust-variable name))

(define (%input-tristate v)
  ;; #t -> 1 (enable), #f -> 0 (disable), 'unset (or anything else) -> -1.
  (cond ((eq? v #t) 1) ((eq? v #f) 0) (else -1)))

(define (%input-choice v allowed setting)
  ;; A member of ALLOWED becomes its string form; 'unset leaves the setting
  ;; unchanged (empty string); anything else is reported and ignored.
  (cond
   ((eq? v 'unset) "")
   ((and (symbol? v) (memq v allowed)) (symbol->string v))
   (else
    (wm-log (format #f "wm-configure-input!: unknown ~a ~s" setting v))
    "")))

(define (%input-output v)
  (cond ((eq? v 'unset) #f)
        ((eq? v 'auto) "")
        ((and (string? v) (not (string-null? v))) v)
        (else (error "wm-configure-input!: map-to-output expects an output name or 'auto" v))))

(define* (wm-configure-input! match #:key (tap-to-click 'unset)
                              (natural-scroll 'unset)
                              (accel-profile 'unset)
                              (click-method 'unset)
                              (map-to-output 'unset))
  "Store a libinput configuration rule and apply it to matching devices.

MATCH is #t (every device) or a string matched as a substring of the
device name (as reported by `wm-input-devices'). Keyword settings, each
left unchanged when omitted:
  #:tap-to-click   #t or #f
  #:natural-scroll #t or #f
  #:accel-profile  'flat or 'adaptive
  #:click-method   'button-areas or 'clickfinger
  #:map-to-output  an output name or 'auto (restore automatic association)
A later rule with the same MATCH replaces the earlier one. On the udev
backend the rule applies to matching devices now and on hotplug; under
winit it is stored but configures nothing. Returns #t when stored."
  (let ((var (%guile-user-var 'wm-configure-input-rule!)))
    (and var
         ((variable-ref var)
          (if (eq? match #t) "" match)
          (%input-tristate tap-to-click)
          (%input-tristate natural-scroll)
          (%input-choice accel-profile '(flat adaptive) "accel-profile")
          (%input-choice click-method '(button-areas clickfinger) "click-method")
          (%input-output map-to-output)))))

;; Frame indicator (StumpWM curframe): echo the frame geometry and pulse
;; the focus border bright for a moment.
(define (flash-current-frame!)
  (let ((r (current-frame-rect)))
    (echo (format #f "current frame: ~ax~a at ~a,~a"
                  (caddr r) (cadddr r) (car r) (cadr r))))
  (set-border-color! "#fabd2f")
  ;; Restore the current state's color without changing keyboard ownership
  ;; or invalidating a pending which-key timer.
  (wm-run-after 400 refresh-key-border!))

;; Urgency: Rust calls (handle-urgent-window! id) on xdg-activation requests.
(define (handle-urgent-window! id) (add-urgent-window! id))

(define (handle-pointer-focus! id)
  "Rust: a click, touch tap or stylus tip selected window ID. Update the
frame/head or floating-window selection through the normal focus path.
Personal configuration may replace this hook to disable click-to-focus."
  (focus-window-by-id! id))

;; Foreign-toplevel management (wlr-foreign-toplevel-management): external
;; bars/switchers request actions on windows by their compositor id. Rust
;; (src/handlers/foreign_toplevel.rs) looks these up by plain top-level
;; name, as with the other event entry points. Activation and application
;; fullscreen policy come from (minde groups); taskbar fullscreen combines
;; both operations here. `close` is handled entirely in Rust (a shell close).
(define (handle-foreign-fullscreen! id on)
  "Rust: a taskbar asked to (un)fullscreen window ID. Activates the window,
then drives the group's fullscreen model (fullscreen-window / fullscreen!)
so at most one window per group is fullscreen and the layout re-syncs on exit."
  (and (handle-foreign-activate! id)
       (handle-window-fullscreen! id on)))

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

;; ---------------------------------------------------------------------
;; Interactive resize mode (StumpWM iresize), entered with C-t m r.
;; Arrows/hjkl move the nearest split divider by %resize-step pixels,
;; b/= balances all frames, Return/Escape leaves the mode. The mode works
;; by its bindings returning %resize-map, which dispatch-key re-arms
;; (border turns blue while armed).
;; ---------------------------------------------------------------------

(define %resize-step 30)

;; Guarded like set-border-color!: tolerate a binary/test without the
;; message subrs.
(define (call-if-bound name . args)
  (apply rust-call-if-bound name args))

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

;; ---------------------------------------------------------------------
;; Gaps: space between frames (inner) and against the screen edge
;; (outer). Off by default; uncomment to taste.
;; ---------------------------------------------------------------------

;; (set-gaps! 8 8)

;; ---------------------------------------------------------------------
;; Layouts: named frame-tree presets ((minde layouts)). C-t m l prompts
;; and applies; save-layout-prompt! saves the live tree under a name to
;; ~/.config/minde/layouts.scm, reloaded at startup.
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

;; Load layouts saved in earlier sessions.
(load-layouts!)

(define (layout-prompt!)
  (read-one-line "layout: "
    (lambda (name)
      (unless (string-null? name)
        (apply-layout! name)))
    #:completions layout-names
    #:history 'layout))

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

;; ---------------------------------------------------------------------
;; Group & window management parity (StumpWM)
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
  (echo (rust-call 'wm-version)))

(define (modifiers!)
  (echo (format #f "key spec prefixes: C- control, M- alt, S- shift, s- super~%prefix key: ~a" %prefix-key)))

(define (redisplay!)
  "Re-places every window of the active group (StumpWM redisplay /
refresh)."
  (sync-frames!)
  (echo "redisplayed"))

;; Dynamic groups (StumpWM parity): prompts and per-group commands.
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

;; ---------------------------------------------------------------------
;; Frames & placement parity (StumpWM): fselect, expose,
;; sibling, uniform splits, rules persistence, desktop dump/restore
;; ---------------------------------------------------------------------

;; Equal-width labels let large selections use a key trie without ambiguity
;; between a complete pick and the start of a longer label.
(define (make-selection-keymap labels pick cancel)
  (define (new-map)
    (let ((km (make-keymap)))
      (set-keymap-cancel! km cancel)
      (for-each (lambda (key) (hash-set! km key (lambda () #t)))
                '("Escape" "Return" "C-g"))
      km))
  (let ((root (new-map)))
    (for-each
     (lambda (label n)
       (let loop ((km root) (keys (string->list label)))
         (let ((key (string (car keys))))
           (if (null? (cdr keys))
               (hash-set! km key (lambda () (pick n)))
               (let ((child (or (hash-ref km key) (new-map))))
                 (hash-set! km key child)
                 (loop child (cdr keys)))))))
     labels (iota (length labels)))
    root))

(define (fselect!)
  (cancel-frame-selection!)
  (let* ((labels (frame-selection-labels (length (frame-leaves (current-tree)))))
         (generation (show-frame-overlays! labels)))
    (make-selection-keymap labels focus-frame-by-index!
                           (lambda () (clear-frame-overlays! generation)))))

(define (expose-mode!)
  (cancel-frame-selection!)
  (let ((count (expose-enter!)))
    (if count
        (make-selection-keymap (frame-selection-labels count) expose-pick!
                               (lambda () (expose-pick! #f)))
        #t)))

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

;; Gravity names for the P g prompt.
(define %gravity-names
  '("center" "top" "bottom" "left" "right"
    "top-left" "top-right" "bottom-left" "bottom-right"))

;; Placement rules saved by remember!/forget! in earlier sessions.
(load-placement-rules!)

;; Utility windows that would otherwise hijack the current frame float
;; instead (#:float? rules match on retitle too, since Wayland clients
;; set the title only after mapping). After load-placement-rules!, which
;; replaces the rule list wholesale.
(add-placement-rule! "Picture-in-Picture" #:float? #t)

;; ---------------------------------------------------------------------
;; Keys & help parity (StumpWM parity)
;; ---------------------------------------------------------------------

;; send-raw-key: capture the next key press and synthesize it into the
;; focused window (for keys the wm would otherwise act on).
(define (send-raw-key!)
  (echo "send key: press a key...")
  (set! %describe-next-key
        (lambda (mods name) (send-key (key-spec mods name)))))

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
  '((guile-user) (minde frames) (minde compositor frames)
    (minde groups) (minde ui prompt)
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
    (select-from-menu items
      (lambda (name)
        (let ((result (invoke-command name)))
          (when (hash-table? result) (set-key-state! result))))
      #:prompt "command: ")))

(define (show-windows!) (echo (echo-windows-string)))
(define (show-frame-labels!)
  (let ((generation (show-frame-overlays!)))
    (wm-run-after 2000 (lambda () (clear-frame-overlays! generation)))))
(define (save-layout-prompt!)
  (read-one-line "save layout as: "
    (lambda (name) (unless (string-null? name) (save-layout! name)))
    #:completions layout-names #:history 'layout))
(define (desktop-file-prompt! prompt action)
  (read-one-line prompt
    (lambda (path) (unless (string-null? path) (action path)))
    #:initial (string-append (or (getenv "HOME") ".") "/.config/minde/desktop.scm")))
(define (dump-desktop-prompt!) (desktop-file-prompt! "dump desktop to: " dump-desktop-to-file))
(define (restore-desktop-prompt!) (desktop-file-prompt! "restore desktop from: " restore-from-file))

;; Declarative configuration is parsed and validated without evaluation. A
;; candidate binding table is built completely before these globals are
;; swapped, so a malformed reload cannot partially redefine the session.
(define %configuration-base-bindings #f)
(define %configuration-base-docs #f)
(define %configuration-base-globals #f)
(define %configuration-base-global-docs #f)
(define %configuration-default-file "default-config.scm")

(define (copy-hash-table table)
  (let ((copy (make-hash-table)))
    (hash-for-each (lambda (key value) (hash-set! copy key value)) table)
    copy))

(define (bundled-configuration-path file)
  (let ((directory (or (getenv "MINDE_SCHEME_DIR")
                           (let ((file (current-filename)))
                             (and (string? file) (dirname file)))
                           "scheme")))
    (string-append directory "/" file)))

(define (configuration-file-path)
  (or (getenv "MINDE_CONFIG") (bundled-configuration-path %configuration-default-file)))

(define (register-configuration-layer!)
  "Makes the bindings currently installed by a user init layer the atomic
reload baseline, including globals. Call once after adding imperative bindings."
  (set! %configuration-base-bindings (copy-hash-table %prefix-bindings))
  (copy-binding-submaps! %prefix-bindings %configuration-base-bindings)
  (set! %configuration-base-docs
        (let ((docs (hash-ref %binding-docs %prefix-bindings)))
          (if docs (copy-hash-table docs) (make-hash-table))))
  (set! %configuration-base-globals (copy-hash-table %keybindings))
  (copy-binding-submaps! %keybindings %configuration-base-globals)
  (set! %configuration-base-global-docs (copy-hash-table %global-binding-docs))
  #t)

(define (merge-configuration-bindings base overrides)
  ;; Merge data before constructing maps: overridden defaults are never built.
  (append (filter (lambda (binding) (not (assoc (car binding) overrides))) base)
          overrides))

(define (compile-configuration-action action)
  ;; Returns a callable/table, description and optional child-map relationship.
  (cond
   ((symbol? action)
    (let ((command (command-ref action)))
      (list (command-procedure command) (command-summary command) #f)))
   ((eq? (car action) 'call)
    (let* ((command (command-ref (cadr action)))
           (arguments (validate-command-arguments (cadr action) (caddr action)))
           (proc (command-procedure command)))
      (list (lambda () (apply proc arguments)) (command-summary command) #f)))
   (else
    (let* ((fields (cdr action))
           (child (make-hash-table)) (docs (make-hash-table))
           (enter (cadr (assq 'enter fields))))
      (apply-configuration-bindings! (cdr (assq 'bindings fields)) child docs #f)
      (hash-set! %binding-docs child docs)
      (list (if enter
                (let ((proc (car (compile-configuration-action enter))))
                  (lambda () (proc) child))
                child)
            (cadr (assq 'description fields)) child)))))

(define (apply-configuration-bindings! bindings table docs global?)
  (for-each
   (lambda (binding)
     (let ((key (if global? (call-with-values (lambda () (parse-key-spec (car binding))) cons)
                    (car binding)))
           (action (cadr binding)))
       (set-binding-submap! table key #f)
       (if action
           (let ((compiled (compile-configuration-action action)))
             (hash-set! table key (car compiled))
             (hash-set! docs key (if (pair? (cddr binding)) (caddr binding) (cadr compiled)))
             (when (caddr compiled) (set-binding-submap! table key (caddr compiled))))
           (begin (hash-remove! table key) (hash-remove! docs key)))))
   bindings))

(define (apply-configuration! config)
  (let* ((defaults (and (not %configuration-base-bindings)
                         (validate-configuration-file (bundled-configuration-path "default-config.scm"))))
         (candidate (if defaults (make-hash-table) (copy-hash-table %configuration-base-bindings)))
         (candidate-docs (if defaults (make-hash-table) (copy-hash-table %configuration-base-docs)))
         (globals (if defaults (make-hash-table) (copy-hash-table %configuration-base-globals)))
         (global-docs (if defaults (make-hash-table) (copy-hash-table %configuration-base-global-docs))))
    (unless defaults
      (copy-binding-submaps! %configuration-base-bindings candidate)
      (copy-binding-submaps! %configuration-base-globals globals))
    (apply-configuration-bindings!
     (merge-configuration-bindings (if defaults (configuration-bindings defaults) '())
                                    (configuration-bindings config)) candidate candidate-docs #f)
    (apply-configuration-bindings!
     (merge-configuration-bindings (if defaults (configuration-global-bindings defaults) '())
                                    (configuration-global-bindings config)) globals global-docs #t)
    ;; This native setting also works before the compositor state exists.
    ;; Tests without Rust may omit the primitive; a real rejection must leave
    ;; the rest of the active configuration untouched.
    (when (rust-bound? 'wm-set-emergency-exit)
      (let ((chord (configuration-emergency-exit config)))
        (unless (rust-call 'wm-set-emergency-exit
                           (and chord (key:modifiers->bitmask (car chord)))
                           (and chord (cadr chord)))
          (error "native emergency exit configuration rejected" chord))))
    ;; No validation or allocation that can reasonably fail remains after
    ;; this point: publish the complete candidate as one short commit step.
    (set! %prefix-bindings candidate)
    (hash-set! %binding-docs candidate candidate-docs)
    (set! %keybindings globals)
    (set! %global-binding-docs global-docs)
    (set-prefix-key! (configuration-prefix-modifiers config)
                     (configuration-prefix-key config))))

(define (report-callback-problems!)
  (check-compositor-callbacks!
   (lambda (problem)
     (wm-log (format #f "compositor callback ~a (~a args): ~a"
                     (car problem) (cadr problem) (caddr problem))))))

(define (reload-configuration!)
  (let ((path (configuration-file-path)))
    (call-with-scheme-backtrace "configuration reload failure"
      (lambda ()
        (let ((candidate (validate-configuration-file path)))
          (apply-configuration! candidate)
          (report-callback-problems!)
          (wm-log (string-append "reloaded configuration " path))
          ;; The boot-time reload runs before any output exists; an echo
          ;; then only costs a message nobody can see.
          (when (pair? (rust-call-if-bound 'wm-outputs))
            (echo (string-append "reloaded configuration " path)))
          #t))
      (lambda (key . arguments)
        (let ((message (format #f "configuration reload FAILED: ~a ~s"
                               key arguments)))
          (wm-log message)
          (echo message)
          #f)))))

(clear-command-registry!)
(for-each
 (lambda (entry)
   (let* ((name (car entry)) (proc (module-ref (current-module) name)))
     (register-builtin-command! name
       (if (memq name '(split-frame-horizontal! split-frame-vertical! remove-split!))
           (lambda () (guard-manual-tiling proc)) proc))))
 builtin-command-metadata)
(register-control-commands!)

(define (refresh-command-help!)
  ;; Fill key help from registry summaries wherever a binding is a command.
  ;; One procedure -> summary table up front; command-names re-sorts the
  ;; registry on every call, so it must not run once per binding.
  (define summaries (make-hash-table))
  (for-each (lambda (name)
              (let ((command (command-ref name)))
                (hashq-set! summaries (command-procedure command)
                            (command-summary command))))
            (command-names))
  (define (visit keymap)
    (hash-for-each
     (lambda (key value)
       (if (procedure? value)
           (let ((summary (hashq-ref summaries value)))
             (when summary (set-binding-doc! keymap key summary)))
           (when (hash-table? value) (visit value))))
     keymap))
  (visit %prefix-bindings))

(when (or (getenv "MINDE_FULL_KEYMAP") (getenv "MINDE_E2E_LEGACY_KEYMAP"))
  (load-from-path "keymaps/stumpwm.scm")
  (refresh-command-help!))

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
;;
;; Reply shape (always a single readable Scheme datum):
;;   (ok RESULT)
;;   (error KEY ARGS MESSAGE-STRING BACKTRACE-STRING)
;; MESSAGE-STRING is a human-readable rendering of the condition and
;; BACKTRACE-STRING a bounded Guile backtrace, so an automated client or human can
;; self-correct without a second round trip. RESULT is guaranteed to be
;; `write`-able and re-`read`-able data: an unreadable value (a record,
;; procedure or other object printing as #<...>) is reported as an
;; `unreadable-result` error rather than emitted as an unparseable datum.
;;
;; The reply envelope (minde-ipc-eval and its helpers) lives in the plain
;; file ipc-reply.scm so it can be exercised headlessly by
;; tests/ipc-reply-test.scm. `load` searches the load path, which already
;; contains the scheme/ directory (added above and via -L at startup).
(load-from-path "ipc-reply.scm")

;; Event push socket glue: run-event-hook! (in (minde hooks)) resolves and
;; calls the top-level `minde-mirror-event' defined here on every firing, so
;; every event is serialized to one readable s-expression line and mirrored to
;; $XDG_RUNTIME_DIR/minde-events.sock subscribers with no user-installed
;; hook. Loaded after ipc-reply.scm because it reuses ipc-writable-datum for the
;; writable-data guarantee. Kept in a plain file for headless testing
;; (tests/event-stream-test.scm).
(load-from-path "event-stream.scm")

;; Runtime API introspection: `describe-api' returns the whole control surface
;; (commands, public module procedures, wm-* gsubrs and event hooks) as plain
;; re-readable data, so an agent's first IPC call can discover what is callable
;; here. Kept in a plain file for headless testing and to feed the generated
;; doc/generated/api-catalog.scm from the same source (see api-introspect.scm).
(load-from-path "api-introspect.scm")

;; The Guile REPL server owns another thread and can race policy mutation.
;; Keep it solely as an explicit development escape hatch.
(when (getenv "MINDE_UNSAFE_REPL")
  (use-modules (system repl server) (minde unsafe-repl)))

;; A failure to start the REPL server (e.g. unwritable runtime dir) should
;; not prevent the rest of the init file / compositor from working.
;; Guarded via an environment variable (not a define) so that reloading
;; this file with C-t R doesn't spawn a second server: top-level defines
;; are re-evaluated on load, but the process environment persists.
(when (and (getenv "MINDE_UNSAFE_REPL")
           (not (getenv "MINDE_REPL_STARTED")))
  (catch #t
    (lambda ()
      (spawn-server (make-private-repl-socket (getenv "XDG_RUNTIME_DIR")))
      (setenv "MINDE_REPL_STARTED" "1")
      (wm-log (string-append "REPL server listening at "
                             (getenv "XDG_RUNTIME_DIR") "/minde-repl.sock")))
    (lambda (key . args)
      (wm-log (format #f "could not start unsafe REPL server: ~a ~a"
                       key args)))))

;; The compositor -> policy contract: every callback Rust will invoke is
;; defined by now (built in above or by the user's configuration), so a
;; missing or mis-arity definition is a startup fact worth logging rather
;; than a silent no-op the first time Rust calls it.
(report-callback-problems!)

(wm-log "minde scheme layer loaded")
