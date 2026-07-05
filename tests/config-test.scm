;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (minde commands) (minde command-catalog) (minde config))

(define failures 0)
(define (check name actual expected)
  (unless (equal? actual expected)
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a: expected ~s got ~s~%" name expected actual)))

(register-builtin-command-schemas!)
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
