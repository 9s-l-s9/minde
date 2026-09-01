;;; api-introspect.scm -- runtime discoverability of the control surface.
;;;
;;; Loaded by init.scm (and by tests/api-introspect-test.scm and
;;; scripts/generate-api-catalog.scm headlessly). Kept in a plain file rather
;;; than a public module so it can be exercised in isolation and so it does not
;;; add exports to the frozen public (minde ...) module API.
;;;
;;; `describe-api' answers an agent's first question -- "what can I do here?" --
;;; in one IPC call, returning the whole control surface as plain re-readable
;;; data (symbols, strings and lists only; no #<...> objects), so the reply
;;; honors the same writable-data guarantee as ipc-reply.scm. The four sections
;;; are:
;;;   commands   -- the registered command catalog (name, category, summary,
;;;                 arguments, documentation);
;;;   procedures -- public bindings of the eight documented (minde ...)
;;;                 modules (name, module, signature, documentation);
;;;   gsubrs     -- the wm-* Rust primitives (name, signature, documentation);
;;;   hooks      -- the (minde hooks) event hooks (name, arguments,
;;;                 documentation).
;;;
;;; The gsubr and hook facts live here as the single structured source: the
;;; runtime answer and the generated doc/generated/api-catalog.scm are both
;;; produced from it, so the file and the live reply cannot drift.

;;; --- Structured source of truth for the non-reflectable surface -----------

;; wm-* gsubrs are Rust primitives registered in src/guile/mod.rs; they carry
;; no Guile docstring at runtime, so their signature and one-line description
;; are recorded here.  Keep in sync with the register_gsubr calls in
;; src/guile/mod.rs; tests/api-introspect-test.scm compares this table against
;; the registration sites in the Rust source and fails on any omission.
(define %api-gsubr-metadata
  '((wm-spawn "(wm-spawn command)"
     "Spawn a child process on the main thread with WAYLAND_DISPLAY set.")
    (wm-quit "(wm-quit)"
     "Quit the compositor and stop the event loop.")
    (wm-log "(wm-log message)"
     "Append MESSAGE to the compositor log.")
    (wm-place-window "(wm-place-window id x y w h)"
     "Move and size the tiled window ID to the given rectangle.")
    (wm-focus-window "(wm-focus-window id)"
     "Give keyboard focus to the window ID.")
    (wm-close-window "(wm-close-window id)"
     "Politely request the client of window ID to close it.")
    (wm-clear-focus "(wm-clear-focus)"
     "Drop keyboard focus so no window is focused.")
    (wm-message "(wm-message text [timeout-ms])"
     "Show an on-screen message, optionally auto-clearing after TIMEOUT-MS.")
    (wm-clear-message "(wm-clear-message)"
     "Remove the current on-screen message.")
    (wm-add-overlay "(wm-add-overlay x y text)"
     "Add a positioned text overlay at output coordinates X,Y.")
    (wm-clear-overlays "(wm-clear-overlays)"
     "Remove every text overlay.")
    (wm-border-color "(wm-border-color hex)"
     "Set the focus-border color from a \"#rrggbb\" string.")
    (wm-focus-rect "(wm-focus-rect x y w h)"
     "Draw the focus rectangle at the given geometry.")
    (wm-output-geometry "(wm-output-geometry)"
     "Return the primary output usable rectangle.")
    (wm-run-after-ms "(wm-run-after-ms ms token)"
     "Run the callback stored under TOKEN after MS milliseconds.")
    (wm-set-fullscreen "(wm-set-fullscreen id fullscreen?)"
     "Set whether window ID is fullscreen.")
    (wm-kill-window "(wm-kill-window id)"
     "Forcibly terminate the client owning window ID.")
    (wm-warp-pointer "(wm-warp-pointer x y)"
     "Warp the pointer to global logical output coordinates X,Y.")
    (wm-pointer-position "(wm-pointer-position)"
     "Return the pointer position as (X Y) in global logical coordinates.")
    (wm-window-geometry "(wm-window-geometry id)"
     "Return visible window ID's global logical (X Y WIDTH HEIGHT), or #f.")
    (wm-request-paste "(wm-request-paste)"
     "Read CLIPBOARD asynchronously and deliver it to minde's active prompt.")
    (wm-outputs "(wm-outputs)"
     "Return ((id x y w h name) ...): every output's usable rectangle.")
    (wm-runtime-info "(wm-runtime-info)"
     "Return (backend xwayland-status xdisplay uptime-ms).")
    (wm-set-clipboard "(wm-set-clipboard text)"
     "Set the Wayland CLIPBOARD selection contents to TEXT.")
    (wm-set-primary "(wm-set-primary text)"
     "Set the Wayland PRIMARY (middle-click) selection contents to TEXT.")
    (wm-place-float "(wm-place-float id x y w h)"
     "Move and size the floating window ID to the given rectangle.")
    (wm-raise-window "(wm-raise-window id)"
     "Raise window ID to the top of the stack.")
    (wm-set-floating "(wm-set-floating id floating?)"
     "Set whether window ID floats above the tiling layout.")
    (wm-send-string "(wm-send-string text [delay-ms])"
     "Type TEXT through paced synthetic key events (20 ms between characters by default).")
    (wm-type "(wm-type text [delay-ms])"
     "Reliably type TEXT through paced synthetic key events; chars the layout cannot produce fall back to clipboard+Ctrl+V in order.")
    (wm-click "(wm-click button [count])"
     "Click BUTTON COUNT times; accepts left/middle/right symbols, 1/2/3, or evdev codes.")
    (wm-send-key "(wm-send-key mods keysym-name)"
     "Synthesize a paced key press/release with modifier mask MODS; Enter aliases Return.")
    (wm-paste "(wm-paste)"
     "Send Ctrl+V to the focused surface using the synthetic key queue.")
    (wm-scroll "(wm-scroll dx dy)"
     "Scroll DX/DY wheel notches (1 = one wheel click) at the current pointer position; sends discrete value120 plus continuous values.")
    (wm-screenshot "(wm-screenshot path [window-id])"
     "Write a deferred PNG of the output under the pointer (or WINDOW-ID's region) to absolute PATH; returns an automation token, completion via wm-automation-status.")
    (wm-warp-pointer-relative "(wm-warp-pointer-relative dx dy)"
     "Warp the pointer by a relative delta DX,DY.")
    (wm-set-key-repeat "(wm-set-key-repeat spec)"
     "Configure keyboard repeat rate and delay from SPEC.")
    (wm-idle-ms "(wm-idle-ms)"
     "Return milliseconds elapsed since the last input event.")
    (wm-input-devices "(wm-input-devices)"
     "Return ((name cap ...) ...): the present libinput devices.")
    (wm-configure-input-rule! "(wm-configure-input-rule! match key value kind flag)"
     "Low-level libinput rule primitive wrapped by wm-configure-input!.")
    (wm-session-locked? "(wm-session-locked?)"
     "Return whether the session is currently locked.")
    (wm-publish-event "(wm-publish-event line)"
     "Mirror one serialized event LINE to every event-socket subscriber.")
    (wm-drop-files "(wm-drop-files x y paths)"
     "Schedule a native Wayland copy drop of absolute regular-file PATHS and return its token or #f.")
    (wm-drop-text "(wm-drop-text x y text)"
     "Schedule a native Wayland plain-text copy drop and return its token or #f.")
    (wm-automation-status "(wm-automation-status token)"
     "Return (OPERATION STATUS) for a recent asynchronous automation token, or #f.")))

;; Event hooks fired by the bundled modules (see scheme/minde/hooks.scm).
;; name, payload argument names, one-line description.
(define %api-hook-metadata
  '((new-window (id title app-id) "A window was mapped and placed.")
    (destroy-window (id) "A window was unmapped.")
    (focus-window (id) "The shown window changed (ID or #f).")
    (focus-frame (x y w h) "The current frame changed.")
    (focus-group (name) "The focused group switched.")
    (message (text) "Something was echoed to the user.")
    (session-lock () "The session-lock surface came up.")
    (session-unlock () "The session-lock surface went away.")
    (automation-result (token operation status)
     "An asynchronous automation request reached a terminal status.")))

;; The eight documented public modules, matching generate-api-reference.scm.
(define %api-public-modules
  '((minde windows)
    (minde frames)
    (minde groups)
    (minde layouts)
    (minde input)
    (minde commands)
    (minde hooks)
    (minde status)))

;;; --- Helpers ---------------------------------------------------------------

(define (api-arity-signature name procedure)
  "Best-effort signature string for PROCEDURE from its minimum arity."
  (let* ((arity (procedure-minimum-arity procedure))
         (required (and arity (list-ref arity 0)))
         (optional (and arity (list-ref arity 1)))
         (rest? (and arity (list-ref arity 2))))
    (if (not arity)
        (format #f "(~a ...)" name)
        (let ((parts (append
                      (map (lambda (i) (format #f "arg-~a" (+ i 1)))
                           (iota required))
                      (map (lambda (i) (format #f "[opt-~a]" (+ i 1)))
                           (iota optional))
                      (if rest? '("...") '()))))
          (format #f "(~a~a)" name
                  (if (null? parts) ""
                      (string-append " " (string-join parts " "))))))))

(define (api-string doc)
  "DOC as a plain string, or the empty string when absent."
  (if (and (string? doc) (not (string-null? doc))) doc ""))

(define (api-name-matches? filter name)
  "True when FILTER is #f/empty or a case-insensitive substring of NAME."
  (or (not filter)
      (not (string? filter))
      (string-null? filter)
      (string-contains (string-downcase (symbol->string name))
                       (string-downcase filter))))

;;; --- Section builders ------------------------------------------------------

(define (api-commands filter)
  "Registered command catalog entries as plain alists, optionally filtered."
  (filter-map
   (lambda (name)
     (and (api-name-matches? filter name)
          (let ((command (command-ref name)))
            (list (cons 'name name)
                  (cons 'category (command-category command))
                  (cons 'summary (api-string (command-summary command)))
                  (cons 'arguments (command-arguments command))
                  (cons 'documentation
                        (api-string (command-documentation command)))))))
   (command-names)))

(define (api-module-procedures module-name filter)
  "Public bindings of MODULE-NAME as plain alists, optionally filtered."
  (let ((interface (resolve-interface module-name)))
    (sort
     (filter-map
      (lambda (pair)
        (let ((name (car pair)) (value (cdr pair)))
          (and (api-name-matches? filter name)
               (list (cons 'name name)
                     (cons 'module module-name)
                     (cons 'signature
                           (if (procedure? value)
                               (api-arity-signature name value)
                               (symbol->string name)))
                     (cons 'documentation
                           (api-string
                            (and (procedure? value)
                                 (procedure-documentation value))))))))
      (module-map cons interface))
     (lambda (a b)
       (string<? (symbol->string (assq-ref a 'name))
                 (symbol->string (assq-ref b 'name)))))))

(define (api-procedures filter)
  "Public bindings of every documented module, optionally filtered."
  (append-map (lambda (module-name)
                (api-module-procedures module-name filter))
              %api-public-modules))

(define (api-gsubrs filter)
  "The wm-* Rust primitives as plain alists, optionally filtered."
  (filter-map
   (lambda (entry)
     (and (api-name-matches? filter (car entry))
          (list (cons 'name (car entry))
                (cons 'signature (cadr entry))
                (cons 'documentation (caddr entry)))))
   %api-gsubr-metadata))

(define (api-hooks filter)
  "The (minde hooks) event hooks as plain alists, optionally filtered."
  (filter-map
   (lambda (entry)
     (and (api-name-matches? filter (car entry))
          (list (cons 'name (car entry))
                (cons 'arguments (cadr entry))
                (cons 'documentation (caddr entry)))))
   %api-hook-metadata))

;;; --- Entry point -----------------------------------------------------------

(define* (describe-api #:optional filter)
  "Returns the full compositor control surface as plain re-readable data:
an alist with keys commands, procedures, gsubrs and hooks, each mapping to a
list of per-item alists. FILTER, when a non-empty string, keeps only items
whose name contains it (case-insensitive apropos). Everything returned is
`write'-able and re-`read'-able, honoring the IPC writable-data guarantee.

Typical agent use over the IPC socket:
  (describe-api)          ; the whole surface
  (describe-api \"float\")  ; just the floating-window primitives"
  (list (cons 'commands (api-commands filter))
        (cons 'procedures (api-procedures filter))
        (cons 'gsubrs (api-gsubrs filter))
        (cons 'hooks (api-hooks filter))))
