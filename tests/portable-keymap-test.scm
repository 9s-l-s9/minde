;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (srfi srfi-1))

(define (wm-spawn command) #t)
(define (wm-quit) #t)
(define (wm-session-locked?) #f)
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
    "r" "Space" "colon" "w" "x" "f" "g" "m" "s"))
(for-each
 (lambda (key)
   (check (string-append "portable binding exists: " key)
          (hash-ref %prefix-bindings key)))
 required-keys)

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

(if (zero? failures)
    (format #t "all portable keymap tests passed~%")
    (exit 1))
