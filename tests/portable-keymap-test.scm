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
(unsetenv "MINDE_TERMINAL")
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
(check "terminal has a dependency-light default"
       (string=? (terminal-command) "foot || xterm"))
(setenv "MINDE_TERMINAL" "my-terminal --flag")
(check "terminal can be configured"
       (string=? (terminal-command) "my-terminal --flag"))

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
           (bind-portable-key! "h" (lambda () #t) "duplicate")
           #f)
         (lambda _ #t)))

(check "IPC evaluator returns a success envelope"
       (string=? (minde-ipc-eval "(+ 2 3)") "(ok 5)"))
(check "IPC evaluator rejects multiple datums"
       (string-prefix? "(error " (minde-ipc-eval "1 2")))

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
  (register-configuration-layer!)
  (apply-configuration!
   (validate-configuration '(minde-config (version 1) (prefix (ctrl) "t")
                                         (bindings ("F20" windows)))))
  (check "declarative command override removes submap relationship"
         (not (binding-submap %prefix-bindings "F20")))
  (check "declarative command override shows its own help"
         (equal? (binding-doc %prefix-bindings "F20") (command-summary (command-ref 'windows))))
  (apply-configuration!
   (validate-configuration '(minde-config (version 1) (prefix (ctrl) "t") (bindings))))
  (check "removing override restores baseline child map" (eq? child (binding-submap %prefix-bindings "F20")))
  (check "removing override restores baseline description"
         (equal? (binding-doc %prefix-bindings "F20") "wrapped submap")))

(if (zero? failures)
    (format #t "all portable keymap tests passed~%")
    (exit 1))
