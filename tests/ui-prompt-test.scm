;;; SPDX-License-Identifier: GPL-3.0-or-later
(use-modules (minde ui prompt))

(define shown #f)
(define cleared 0)
(define repeat #f)
(define submitted #f)

(configure-prompt-ui!
 #:show (lambda (text duration) (set! shown text))
 #:clear (lambda () (set! cleared (+ cleared 1)))
 #:set-key-repeat (lambda (enabled?) (set! repeat enabled?))
 #:request-paste (lambda () #t)
 #:set-clipboard (lambda (text) #t))

(read-one-line "run: " (lambda (text) (set! submitted text))
               #:completions '("foot" "firefox"))
(unless (and (input-active?) repeat (string=? shown "run: |")) (exit 1))
(input-handle-key! 0 "f" "f")
(input-handle-key! 0 "Tab" "")
(unless (string=? shown "run: foot|") (exit 1))
(input-handle-key! 0 "Return" "")
(unless (and (string=? submitted "foot") (not (input-active?))
             (not repeat) (= cleared 1))
  (exit 1))
(format #t "all UI prompt tests passed~%")
