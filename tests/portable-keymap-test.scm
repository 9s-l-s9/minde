;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (srfi srfi-1))

(define (wm-spawn command) #t)
(define (wm-quit) #t)
(define %test-emergency-exit #f)
(define %test-emergency-reject? #f)
(define (wm-set-emergency-exit mods key)
  (and (not %test-emergency-reject?)
       (begin (set! %test-emergency-exit (list mods key)) #t)))
(define (wm-session-locked?) #f)
(define (wm-session-lock-confirmed?) #f)
(define (wm-log message) #t)
(define (wm-place-window . arguments) #t)
(define (wm-focus-window . arguments) #t)
(define (wm-close-window . arguments) #t)
(define (wm-clear-focus) #t)
(define (wm-output-geometry) '(0 0 1280 720))
(define (wm-message . arguments) #t)
(define (wm-run-after-ms . arguments) #t)
(define (wm-set-fullscreen . arguments) #t)
(define (wm-kill-window . arguments) #t)
(define (wm-warp-pointer . arguments) #t)
(define (wm-request-paste) #t)
(define (wm-set-clipboard . arguments) #t)
(define (wm-border-color . arguments) #t)
(define (wm-add-overlay . arguments) #t)
(define (wm-clear-overlays) #t)
(define (wm-send-key . arguments) #t)
(define (wm-warp-pointer-relative . arguments) #t)
(define (wm-set-key-repeat . arguments) #t)

(setenv "MINDE_RULES_FILE" "/nonexistent-minde-rules.scm")
(setenv "MINDE_CONFIG" "scheme/default-config.scm")
(fluid-set! %file-port-name-canonicalization 'absolute)
(primitive-load
 (canonicalize-path
  (string-append (dirname (current-filename)) "/../scheme/init.scm")))

(define failures 0)
(define (check description value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" description)))

(check "Super+Q has no compositor binding" (not (hash-ref %keybindings (cons 64 "q"))))
(check "default emergency exit reaches Rust" (equal? %test-emergency-exit '(12 "BackSpace")))
(let ((disabled '(minde-config (version 1) (prefix (ctrl) "g")
                                (emergency-exit #f) (bindings))))
  (set! %test-emergency-reject? #t)
  (let ((before %prefix-bindings))
    (check "native rejection fails the apply"
           (catch #t (lambda () (apply-configuration! (validate-configuration disabled)) #f)
                     (lambda _ #t)))
    (check "native rejection preserves prefix and bindings"
           (and (eq? before %prefix-bindings) (= %prefix-mods 4) (equal? %prefix-key "t"))))
  (set! %test-emergency-reject? #f)
  (apply-configuration! (validate-configuration disabled))
  (check "disabled exit reaches Rust" (equal? %test-emergency-exit '(#f #f)))
  (apply-configuration! (validate-configuration-file "scheme/default-config.scm"))
  (check "reload restores default emergency exit" (equal? %test-emergency-exit '(12 "BackSpace"))))

(define (entered-submap key)
  (let ((value (hash-ref %prefix-bindings key)))
    (or (binding-submap %prefix-bindings key)
        (and (hash-table? value) value))))

(define (nested-submap keymap key)
  (let ((value (hash-ref keymap key)))
    (or (binding-submap keymap key)
        (and (hash-table? value) value))))

(define required-keys
  '("h" "j" "k" "l" "n" "p"
    "r" "space" "colon" "w" "x" "f" "g" "m" "s"))
(for-each
 (lambda (key)
   (check (string-append "portable binding exists: " key)
          (hash-ref %prefix-bindings key)))
 required-keys)

;; Rust supplies xkbcommon's keysym name "space" for the space bar.
;; Exercise dispatch and the visible menu state, not only table membership.
(check "prefix is consumed" (wm-handle-key 4 #x74 "t" "t"))
(check "space after prefix is consumed" (wm-handle-key 0 #x20 "space" " "))
(check "prefix-space opens the command palette" (menu-active?))
(wm-handle-key 0 #xff1b "Escape" "")
(check "Escape dismisses the command palette" (not (menu-active?)))

(for-each
 (lambda (number)
   (let ((key (number->string number)))
     (check (string-append "number selection exists: " key)
            (hash-ref %prefix-bindings key))
     (check (string-append "number pull exists in window map: " key)
            (hash-ref (entered-submap "w") key))))
 (iota 10))

(for-each
 (lambda (key)
   (check (string-append "move exists in window map: " key)
          (hash-ref (entered-submap "w") key)))
 '("h" "j" "k" "l"))

(check "window list moved away from directional l"
       (and (hash-ref (entered-submap "w") "w")
            (string=? (binding-doc (entered-submap "w") "l") "move window")))
(check "previous window is available without Shift"
       (hash-ref (entered-submap "w") "p"))
(check "both hidden-window pull directions are lowercase"
       (let ((pull-map (nested-submap (entered-submap "w") "u")))
         (and pull-map (hash-ref pull-map "n") (hash-ref pull-map "p"))))

(for-each
 (lambda (key)
   (check (string-append "exchange exists in exchange map: " key)
          (hash-ref (entered-submap "x") key)))
 '("h" "j" "k" "l"))

(for-each
 (lambda (number)
   (let ((key (number->string number)))
     (check (string-append "frame selection exists in frame map: " key)
            (hash-ref (entered-submap "f") key))))
 (iota 10))

(for-each
 (lambda (key)
   (check (string-append "no Alt binding remains: " key)
          (not (hash-ref %prefix-bindings key))))
 '("M-h" "M-j" "M-k" "M-l"
   "M-0" "M-1" "M-2" "M-3" "M-4"
   "M-5" "M-6" "M-7" "M-8" "M-9"))

(check "Print ? top-level help is documented"
       (string-contains (keymap-help-string %prefix-bindings)
                        "w  window commands"))
(check "Print w ? documents direct numbered pulls"
       (string-contains
        (keymap-help-string (entered-submap "w"))
        "0  pull window 0"))

(define (uppercase-letter-key? key)
  (and (= (string-length key) 1)
       (char-upper-case? (string-ref key 0))))

(define (lowercase-keymap? keymap seen)
  (or (memq keymap seen)
      (hash-fold
       (lambda (key value valid?)
         (let ((child (or (binding-submap keymap key)
                          (and (hash-table? value) value))))
           (and valid?
                (not (uppercase-letter-key? key))
                (or (not child)
                    (lowercase-keymap? child (cons keymap seen))))))
       #t keymap)))

(check "portable keymaps contain no uppercase letter bindings"
       (lowercase-keymap? %prefix-bindings '()))

(check "portable prefix is C-t"
       (and (= %prefix-mods 4) (string=? %prefix-key "t")))
(check "help for registered commands comes from the registry"
       (string=? (binding-doc %prefix-bindings "n")
                 (command-summary (command-ref 'focus-next-window!))))
(check "portable defaults have no external global launchers"
       (zero? (hash-fold (lambda (key value count) (+ count 1)) 0 %keybindings)))
(wm-handle-key 4 #f "t" "")
(wm-handle-key 0 #f "Return" "")
(check "Return opens the run prompt" (input-active?))
(wm-handle-key 0 #f "Escape" "")

(define (hash-size table)
  (hash-fold (lambda (key value count) (+ count 1)) 0 table))

(check "every direct binding has generated-reference documentation"
       (= (hash-size %keybindings) (hash-size %global-binding-docs)))

(define (documented-keymap? keymap seen)
  (and
   (not (memq keymap seen))
   (hash-fold
    (lambda (key value valid?)
      (let ((child (or (binding-submap keymap key)
                       (and (hash-table? value) value))))
        (and valid?
             (string? (binding-doc keymap key))
             (or (not child)
                 (documented-keymap? child (cons keymap seen))))))
    #t keymap)))

(check "every portable binding and submap has documentation"
       (documented-keymap? %prefix-bindings '()))

(check "duplicate top-level keys are rejected"
       (catch #t
         (lambda ()
           (validate-configuration
            '(minde-config (version 1) (prefix (ctrl) "t")
                           (bindings ("h" windows) ("h" windows))))
           #f)
         (lambda _ #t)))

(check "IPC evaluator returns a success envelope"
       (string=? (minde-ipc-eval "(+ 2 3)") "(ok 5)"))
(check "IPC evaluator rejects multiple datums"
       (string-prefix? "(error " (minde-ipc-eval "1 2")))

(check "portable startup does not capture the optional legacy map"
       (and (not %configuration-base-bindings) (not (hash-ref %prefix-bindings "M-G"))))
(define %recorded-actions '())
(register-command! 'record-key! (lambda (value) (set! %recorded-actions (cons value %recorded-actions)))
                   #:category 'test
                   #:parameters '(((name . value) (type . integer)))
                   #:summary "Record a test key.")
(define minimal-config '(minde-config (version 1) (prefix (ctrl) "t") (bindings)))
(define override-config
  '(minde-config (version 1) (prefix (ctrl) "t")
    (global-bindings
     ("s-F8" (call record-key! ((value . 8))) "global action")
     ("s-F9" (keymap (description "global map")
                (enter (call record-key! ((value . 9))))
                (bindings ("Page_Up" (call record-key! ((value . 10)))))))
     ("s-F10" (keymap (description "direct global map")
                 (bindings ("x" (call record-key! ((value . 11))))))))
    (bindings ("n" #f)
              ("w" (keymap (description "replacement map")
                      (bindings ("a" (call record-key! ((value . 1))) "custom action")))))))
(apply-configuration! (validate-configuration override-config))
(check "candidate construction never invokes actions" (null? %recorded-actions))
(check "unbinding removes default action and description"
       (and (not (hash-ref %prefix-bindings "n")) (not (binding-doc %prefix-bindings "n"))))
(check "overriding a submap replaces its children"
       (and (= 1 (hash-size (entered-submap "w")))
            (equal? (binding-doc (entered-submap "w") "a") "custom action")))
(wm-handle-key 4 #f "t" "")
(wm-handle-key 0 #f "w" "")
(wm-handle-key 0 #f "a" "")
(wm-handle-key 64 #f "F8" "")
(wm-handle-key 64 #f "F9" "")
(check "global wrapped map arms its documented child"
       (eq? %key-state (binding-submap %keybindings (cons 64 "F9"))))
(wm-handle-key 0 #f "Prior" "")
(wm-handle-key 64 #f "F10" "")
(wm-handle-key 0 #f "x" "")
(check "typed prefix and global actions dispatch with their arguments"
       (equal? %recorded-actions '(11 10 9 8 1)))
(let ((prefix %prefix-bindings) (globals %keybindings) (docs %global-binding-docs))
  (set! %test-emergency-reject? #t)
  (catch #t (lambda () (apply-configuration! (validate-configuration minimal-config))) (lambda _ #f))
  (set! %test-emergency-reject? #f)
  (check "failed apply preserves prefix, globals and descriptions together"
         (and (eq? prefix %prefix-bindings) (eq? globals %keybindings)
              (eq? docs %global-binding-docs))))
(apply-configuration! (validate-configuration minimal-config))
(check "removing overrides restores defaults and removes global shortcuts"
       (and (hash-ref %prefix-bindings "n") (hash-ref (entered-submap "w") "l")
            (zero? (hash-size %keybindings)) (zero? (hash-size %global-binding-docs))))

;; Minimal personal files inherit fresh bundled data on every reload.
(let* ((directory (mkdtemp "/tmp/minde-keymap-defaults-XXXXXX"))
       (path (string-append directory "/default-config.scm"))
       (original (getenv "MINDE_SCHEME_DIR")))
  (dynamic-wind
    (lambda () (setenv "MINDE_SCHEME_DIR" directory))
    (lambda ()
      (call-with-output-file path (lambda (port) (write override-config port)))
      (apply-configuration! (validate-configuration minimal-config))
      (check "bundled global additions are read at reload" (hash-ref %keybindings (cons 64 "F8")))
      (apply-configuration!
       (validate-configuration '(minde-config (version 1) (prefix (ctrl) "t")
                                (global-bindings ("s-F8" #f)) (bindings))))
      (check "explicit unbind removes a bundled global shortcut"
             (and (not (hash-ref %keybindings (cons 64 "F8")))
                  (not (hash-ref %global-binding-docs (cons 64 "F8")))))
      (call-with-output-file path (lambda (port) (write minimal-config port)))
      (apply-configuration! (validate-configuration minimal-config))
      (check "removed bundled bindings do not survive reload"
             (and (zero? (hash-size %prefix-bindings)) (zero? (hash-size %keybindings)))))
    (lambda ()
      (if original (setenv "MINDE_SCHEME_DIR" original) (unsetenv "MINDE_SCHEME_DIR"))
      (delete-file path) (rmdir directory))))
(apply-configuration! (validate-configuration-file "scheme/default-config.scm"))

(bind-prefix-key! "k" (lambda () #f))
(check "undocumented rebind removes default help" (not (binding-doc %prefix-bindings "k")))
(check "which-key no longer shows the old description"
       (not (string-contains (keymap-help-string %prefix-bindings) "k  focus frame")))
(bind-key! '(super) "F21" (lambda () #f) "old global description")
(bind-key! '(super) "F21" (lambda () #f))
(check "undocumented global rebind removes help"
       (not (hash-ref %global-binding-docs (cons 64 "F21"))))
(let ((child (make-documented-keymap "x" (lambda () #f) "child action")))
  (bind-prefix-key! "F20" child "old submap")
  (bind-prefix-key! "F20" (lambda () #f))
  (check "plain rebind removes child-map relationship" (not (binding-submap %prefix-bindings "F20")))
  (check "plain rebind removes child-map description" (not (binding-doc %prefix-bindings "F20")))
  ;; Wrapped submaps need explicit metadata. Capture it with the personal
  ;; layer so a temporary declarative override can later be removed.
  (bind-prefix-key! "F20" (lambda () child) "wrapped submap")
  (set-binding-submap! %prefix-bindings "F20" child)
  (bind-key! '(super) "F20" (lambda () child) "global wrapped submap")
  (set-binding-submap! %keybindings (cons 64 "F20") child)
  (register-configuration-layer!)
  (apply-configuration!
   (validate-configuration '(minde-config (version 1) (prefix (ctrl) "t")
                                         (global-bindings ("s-F20" #f))
                                         (bindings ("F20" windows)))))
  (check "global override removes baseline action and child metadata"
         (and (not (hash-ref %keybindings (cons 64 "F20")))
              (not (binding-submap %keybindings (cons 64 "F20")))))
  (check "declarative command override removes submap relationship"
         (not (binding-submap %prefix-bindings "F20")))
  (check "declarative command override shows its own help"
         (equal? (binding-doc %prefix-bindings "F20") (command-summary (command-ref 'windows))))
  (apply-configuration!
   (validate-configuration '(minde-config (version 1) (prefix (ctrl) "t") (bindings))))
  (check "removing override restores baseline child map" (eq? child (binding-submap %prefix-bindings "F20")))
  (check "removing override restores baseline description"
         (equal? (binding-doc %prefix-bindings "F20") "wrapped submap"))
  (check "removing global override restores baseline description and child"
         (and (eq? child (binding-submap %keybindings (cons 64 "F20")))
              (equal? (hash-ref %global-binding-docs (cons 64 "F20")) "global wrapped submap")))
  (bind-key! '(super) "F20" (lambda () #f))
  (check "imperative global rebind removes child metadata"
         (not (binding-submap %keybindings (cons 64 "F20")))))

;; Selecting the legacy file must not build or even read portable defaults.
(let* ((directory (mkdtemp "/tmp/minde-keymap-legacy-XXXXXX"))
       (original (getenv "MINDE_SCHEME_DIR")))
  (dynamic-wind
    (lambda ()
      (mkdir (string-append directory "/keymaps"))
      (copy-file "scheme/keymaps/stumpwm.conf" (string-append directory "/keymaps/stumpwm.conf"))
      (setenv "MINDE_SCHEME_DIR" directory)
      (setenv "MINDE_FULL_KEYMAP" "1") (unsetenv "MINDE_CONFIG"))
    (lambda ()
      (primitive-load (canonicalize-path "scheme/init.scm"))
      (check "legacy startup works without the portable data file"
             (and %configuration-base-bindings (= %prefix-mods 0)
                  (equal? %prefix-key "Print") (hash-ref %prefix-bindings "M-G")))
      (check "legacy-only commands remain discoverable"
             (every command-ref '(fselect! expose-mode! mark-window-toggle! remember! dump-desktop-prompt!))))
    (lambda ()
      (unsetenv "MINDE_FULL_KEYMAP")
      (if original (setenv "MINDE_SCHEME_DIR" original) (unsetenv "MINDE_SCHEME_DIR"))
      (delete-file (string-append directory "/keymaps/stumpwm.conf"))
      (rmdir (string-append directory "/keymaps")) (rmdir directory))))

(if (zero? failures)
    (format #t "all portable keymap tests passed~%")
    (exit 1))
