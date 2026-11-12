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
