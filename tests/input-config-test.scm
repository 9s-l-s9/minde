;;; input-config-test.scm -- the libinput configuration surface
;;; (`wm-configure-input!` / `wm-input-devices`) exercised on the
;;; no-libinput (winit/nested) path: the primitives must exist, validate
;;; their arguments, and normalize rules exactly as the udev backend
;;; expects, without ever touching a real device.
;;;
;;; Run with: guile -L scheme tests/input-config-test.scm

(use-modules (srfi srfi-1) (srfi srfi-13))

;; Minimal Rust-primitive stubs (same technique as input-test.scm). The
;; low-level `wm-configure-input-rule!` records the normalized scalar tuple
;; the real Rust subr would receive; `wm-input-devices` mirrors the winit
;; path where no libinput context exists.
(define %rules '())
(define (wm-configure-input-rule! match tap natural accel click)
  (set! %rules (cons (list match tap natural accel click) %rules))
  #t)
(define (wm-input-devices) '())

(define %logs '())
(define (wm-spawn cmd) #t)
(define (wm-quit) #t)
(define (wm-log msg) (set! %logs (cons msg %logs)) #t)
(define (wm-place-window id x y w h) #t)
(define (wm-focus-window id) #t)
(define (wm-close-window id) #t)
(define (wm-clear-focus) #t)
(define (wm-focus-rect x y w h) #t)
(define (wm-output-geometry) (list 0 0 1280 720))
(define (wm-message text . _) #t)
(define (wm-clear-message) #t)
(define (wm-request-paste) #t)
(define (wm-set-clipboard text) #t)

(fluid-set! %file-port-name-canonicalization 'absolute)
(primitive-load (canonicalize-path
                 (string-append (dirname (current-filename)) "/../scheme/init.scm")))

(define %failures 0)
(define (check name got expected)
  (if (equal? got expected)
      (format #t "ok - ~a~%" name)
      (begin
        (set! %failures (+ %failures 1))
        (format #t "FAIL - ~a: expected ~s, got ~s~%" name expected got))))
(define (check-true name got) (check name (if got #t #f) #t))

(define (last-rule) (car %rules))

;; ---------------------------------------------------------------------
;; wm-input-devices is available and empty on the no-libinput path.
;; ---------------------------------------------------------------------
(check "wm-input-devices returns the empty list with no libinput"
       (wm-input-devices) '())

;; ---------------------------------------------------------------------
;; #t match becomes the empty (match-everything) string; omitted settings
;; are left unchanged (tristate -1 / empty string).
;; ---------------------------------------------------------------------
(check-true "configure returns #t" (wm-configure-input! #t))
(check "match #t normalizes to the empty string" (last-rule)
       (list "" -1 -1 "" ""))

;; ---------------------------------------------------------------------
;; A named match with every setting present, fully normalized.
;; ---------------------------------------------------------------------
(wm-configure-input! "Touchpad"
                     #:tap-to-click #t
                     #:natural-scroll #f
                     #:accel-profile 'adaptive
                     #:click-method 'clickfinger)
(check "named rule normalizes all settings" (last-rule)
       (list "Touchpad" 1 0 "adaptive" "clickfinger"))

;; ---------------------------------------------------------------------
;; tap/natural tristate: #f -> 0, #t -> 1.
;; ---------------------------------------------------------------------
(wm-configure-input! "" #:tap-to-click #f #:natural-scroll #t)
(check "tristate maps #f/#t to 0/1" (last-rule)
       (list "" 0 1 "" ""))

;; ---------------------------------------------------------------------
;; accel-profile 'flat / click-method 'button-areas normalize to strings.
;; ---------------------------------------------------------------------
(wm-configure-input! "mouse" #:accel-profile 'flat #:click-method 'button-areas)
(check "flat/button-areas normalize to strings" (last-rule)
       (list "mouse" -1 -1 "flat" "button-areas"))

;; ---------------------------------------------------------------------
;; Unknown enum values are reported (wm-log) and dropped to "" rather than
;; passed through to the device.
;; ---------------------------------------------------------------------
(set! %logs '())
(wm-configure-input! "bad" #:accel-profile 'turbo #:click-method 'wiggle)
(check "unknown enum values become empty strings" (last-rule)
       (list "bad" -1 -1 "" ""))
(check-true "unknown accel-profile is logged"
            (find (lambda (m) (string-contains m "accel-profile")) %logs))
(check-true "unknown click-method is logged"
            (find (lambda (m) (string-contains m "click-method")) %logs))

(if (zero? %failures)
    (begin (format #t "all tests passed~%") (exit 0))
    (begin (format #t "~a test(s) FAILED~%" %failures) (exit 1)))
