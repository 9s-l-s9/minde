;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (minde commands) (minde command-catalog) (minde config)
             (minde control))

(define failures 0)
(define (check name actual expected)
  (unless (equal? actual expected)
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a: expected ~s got ~s~%" name expected actual)))

(register-builtin-command-schemas!)
(register-control-commands!)
(register-command! 'personal-noop! (lambda () (error "validation invoked a command"))
                   #:category 'test #:summary "Check without invoking a command.")
(define (candidate command)
  `(minde-config (version 1) (prefix (ctrl) "t") (bindings ("d" ,command))))
(for-each
 (lambda (command)
   (check (format #f "registered zero-argument command ~a" command)
          (validate-configuration (candidate command)) (candidate command)))
 '(focus-next-window! desktop-snapshot windows personal-noop!))
(define (with-keys keys)
  `(minde-config (version 1) (prefix (ctrl) "t")
                 (bindings ,@(map (lambda (key) (list key 'personal-noop!)) keys))))
(define (rejected? datum)
  (catch #t (lambda () (validate-configuration datum) #f) (lambda _ #t)))
(define (with-emergency . fields)
  `(minde-config (version 1) (prefix (ctrl) "t") (bindings) ,@fields))
(check "omitted emergency exit uses the native default"
       (configuration-emergency-exit (validate-configuration (with-emergency)))
       '((ctrl alt) "BackSpace"))
(check "emergency exit can be disabled"
       (configuration-emergency-exit
        (validate-configuration (with-emergency '(emergency-exit #f)))) #f)
(let* ((datum (with-emergency '(emergency-exit (control meta shift) "Page_Up")))
       (normalized (validate-configuration datum)))
  (check "emergency exit normalizes XKB aliases"
         (configuration-emergency-exit normalized) '((control meta shift) "Prior"))
  (check "emergency normalization is idempotent" (validate-configuration normalized) normalized)
  (check "emergency normalization preserves input"
         (configuration-emergency-exit datum) '((control meta shift) "Page_Up")))
(for-each
 (lambda (entry)
   (check (format #f "invalid emergency exit ~s" entry)
          (rejected? (with-emergency entry)) #t))
 '((emergency-exit) (emergency-exit #t) (emergency-exit #f "BackSpace")
   (emergency-exit (hyper) "F12") (emergency-exit (ctrl) F12)
   (emergency-exit (ctrl) "Retrun") (emergency-exit (ctrl) "NoSymbol")
   (emergency-exit (ctrl) "F12" extra) (emergency-exit . #f)))
(check "duplicate emergency settings rejected"
       (rejected? (with-emergency '(emergency-exit #f)
                                 '(emergency-exit (ctrl alt) "BackSpace"))) #t)
(for-each
 (lambda (pair)
   (let* ((datum (with-keys (list (car pair))))
          (normalized (validate-configuration datum)))
     (check (format #f "canonical binding ~s" (car pair))
            (configuration-bindings normalized) (list (list (cdr pair) 'personal-noop!)))
     (check "validation leaves input unchanged" (configuration-bindings datum)
            (list (list (car pair) 'personal-noop!)))
     (check "normalization is idempotent" (validate-configuration normalized) normalized)))
 '(("M-C-g" . "C-M-g") ("s-S-M-C-g" . "C-M-S-s-g") ("C-C-g" . "C-g")
   ("Page_Up" . "Prior") ("M-C-Page_Up" . "C-M-Prior")
   ("U0061" . "a") ("A" . "A") ("XF86AudioPlay" . "XF86AudioPlay")
   ("KP_Enter" . "KP_Enter") ("minus" . "minus")))
(for-each
 (lambda (key)
   (check (format #f "invalid binding ~s" key) (rejected? (with-keys (list key))) #t)
   (check (format #f "invalid prefix ~s" key)
          (rejected? `(minde-config (version 1) (prefix () ,key) (bindings))) #t))
 (append '("Retrun" "Space" "esc" "Pritn" "c-x" "Ctrl-x" "C-" "C--g"
           "" " " "Return " "NoSymbol")
         (list (string-append "Return" (string #\nul) "ignored"))))
(for-each
 (lambda (keys)
   (check (format #f "canonical duplicates ~s" keys) (rejected? (with-keys keys)) #t))
 '(("C-M-g" "M-C-g") ("C-g" "C-C-g") ("Prior" "Page_Up")
   ("M-C-Prior" "C-M-Page_Up") ("a" "U0061")))
(check "modifier and keysym case remain distinct"
       (length (configuration-bindings (validate-configuration (with-keys '("S-g" "s-g" "A" "a"))))) 4)
(check "prefix aliases normalize without reordering fields"
       (validate-configuration '(minde-config (bindings) (prefix (control) "Page_Up") (version 1)))
       '(minde-config (bindings) (prefix (control) "Prior") (version 1)))
(check "argument-requiring command rejected during validation"
       (catch #t
         (lambda () (validate-configuration (candidate 'focus-window!)) #f)
         (lambda _ #t)) #t)
(let* ((template (string-copy "/tmp/minde-config-read-XXXXXX"))
       (port (mkstemp! template)))
  (dynamic-wind
    (lambda () #t)
    (lambda ()
      (display "#.(error \"configuration evaluated\")" port)
      (close-port port)
      (with-fluids ((read-eval? #t))
        (check "configuration reader disables evaluation"
               (catch #t
                 (lambda () (read-configuration template) #f)
                 (lambda (key . args)
                   (and (string-contains (format #f "~s" args) "read-eval? is #f") #t))) #t)))
    (lambda () (unless (port-closed? port) (close-port port)) (delete-file template))))
(let ((config (validate-configuration-file "scheme/default-config.scm")))
  (check "prefix modifiers" (configuration-prefix-modifiers config) '(ctrl))
  (check "prefix key" (configuration-prefix-key config) "t")
  (check "explicit default emergency exit" (configuration-emergency-exit config)
         '((ctrl alt) "BackSpace"))
  (check "binding count" (length (configuration-bindings config)) 0))
(check "unknown command rejected"
       (catch #t
         (lambda ()
           (validate-configuration
            '(minde-config (version 1) (prefix () "Print")
                              (bindings ("x" missing-command!))))
           #f)
         (lambda _ #t))
       #t)
(check "duplicate binding rejected"
       (catch #t
         (lambda ()
           (validate-configuration
            '(minde-config (version 1) (prefix () "Print")
                              (bindings ("x" switch-to-next-group!)
                                        ("x" reload-configuration!))))
           #f)
         (lambda _ #t))
       #t)
(if (zero? failures)
    (format #t "all configuration tests passed~%")
    (exit 1))
