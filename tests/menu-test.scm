;;; menu-test.scm -- Guile-only unit test of select-from-menu.
;;;
;;; Run with: guile -L scheme tests/menu-test.scm

(use-modules (srfi srfi-1))

;; ---------------------------------------------------------------------
;; Stubs
;; ---------------------------------------------------------------------

(define %shown #f)   ; last wm-message text
(define %cleared 0)

(define (wm-message text . rest) (set! %shown text) #t)
(define (wm-clear-message) (set! %cleared (+ %cleared 1)) #t)
(define (wm-log msg) #t)

(use-modules (minde menu))

;; ---------------------------------------------------------------------
;; Assertion helpers
;; ---------------------------------------------------------------------

(define %failures 0)

(define (check name got expected)
  (if (equal? got expected)
      (format #t "ok - ~a~%" name)
      (begin
        (set! %failures (+ %failures 1))
        (format #t "FAIL - ~a: expected ~s, got ~s~%" name expected got))))

(define (check-true name got) (check name (if got #t #f) #t))
(define (check-false name got) (check name (if got #t #f) #f))

(define (shown-has? s) (and %shown (string-contains %shown s) #t))

;; key helpers: (menu-handle-key! mods keysym-name utf8), ctrl bit = 4
(define (key name utf8) (menu-handle-key! 0 name utf8))
(define (ctrl name) (menu-handle-key! 4 name ""))

;; ---------------------------------------------------------------------
;; Selection + navigation
;; ---------------------------------------------------------------------

(define %selected #f)
(define %aborted 0)

(define (open-menu . args)
  (set! %selected #f)
  (select-from-menu '(("alpha" . a) ("beta" . b) ("gamma" . c))
                    (lambda (v) (set! %selected v))
                    #:prompt "test:"
                    #:on-abort (lambda () (set! %aborted (+ %aborted 1)))))

(open-menu)
(check-true "menu active after open" (menu-active?))
(check-true "prompt rendered" (shown-has? "test:"))
(check-true "first row selected" (shown-has? "> 0 alpha"))
(check-true "other rows numbered" (shown-has? "  2 gamma"))

(ctrl "n")
(check-true "C-n moves the marker" (shown-has? "> 1 beta"))
(ctrl "p")
(ctrl "p")
(check-true "C-p wraps to the end" (shown-has? "> 2 gamma"))

(key "Return" "")
(check "Return selects the marked value" %selected 'c)
(check-false "menu closed after select" (menu-active?))

;; Digits select directly.
(open-menu)
(key "1" "1")
(check "digit selects" %selected 'b)

;; Typing filters; BackSpace widens.
(open-menu)
(key "g" "g")
(check-true "filter narrows" (shown-has? "> 0 gamma"))
(check-false "filtered rows gone" (shown-has? "alpha"))
(key "BackSpace" "")
(check-true "backspace widens" (shown-has? "alpha"))

;; C-g aborts.
(let ((before %aborted))
  (ctrl "g")
  (check "C-g aborts" %aborted (+ before 1))
  (check-false "menu closed after abort" (menu-active?)))

;; filter? #f frees j/k for navigation.
(select-from-menu '("one" "two") (lambda (v) (set! %selected v)) #:filter? #f)
(key "j" "j")
(check-true "j navigates with filter? #f" (shown-has? "> 1 two"))
(key "Return" "")
(check "plain string items stand for themselves" %selected "two")

(if (zero? %failures)
    (format #t "all tests passed~%")
    (begin (format #t "~a failure(s)~%" %failures)
           (exit 1)))
