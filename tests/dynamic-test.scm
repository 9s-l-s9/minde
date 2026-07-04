;;; dynamic-test.scm -- Guile-only unit test of dynamic (auto-tiling)
;;; groups in (minde groups), sprint 11.
;;;
;;; Run with:
;;;   guile -L scheme tests/dynamic-test.scm
;;;
;;; Stubs every wm-* Rust subr *before* loading the modules, same
;;; pattern as tests/groups-test.scm.

(use-modules (srfi srfi-1))

;; ---------------------------------------------------------------------
;; Stubs recording calls, standing in for the Rust-side subrs.
;; ---------------------------------------------------------------------

(define %placements (make-hash-table)) ; id -> (x y w h)
(define %focused #f)

(define (wm-place-window id x y w h)
  (hash-set! %placements id (list x y w h))
  #t)

(define (wm-place-float id x y w h)
  (hash-set! %placements id (list x y w h))
  #t)

(define (wm-focus-window id) (set! %focused id) #t)
(define (wm-close-window id) #t)
(define (wm-clear-focus) (set! %focused #f) #t)
(define (wm-output-geometry) (list 1280 720))
(define (wm-log msg) #t)
(define %messages '())
(define (wm-message text . _) (set! %messages (cons text %messages)) #t)
(define (wm-set-floating id on) #t)
(define (wm-raise-window id) #t)

;; Now it's safe to load the modules under test.
(use-modules (minde frames))
(use-modules (minde groups))

;; ---------------------------------------------------------------------
;; Tiny assertion helpers
;; ---------------------------------------------------------------------

(define %failures 0)

(define (check name got expected)
  (if (equal? got expected)
      (format #t "ok - ~a~%" name)
      (begin
        (set! %failures (+ %failures 1))
        (format #t "FAIL - ~a: expected ~s, got ~s~%" name expected got))))

(define (check-true name got)
  (check name (if got #t #f) #t))

(define (rect id) (hash-ref %placements id))
(define (visible? id)
  (let ((p (rect id))) (and p (>= (car p) 0) (>= (cadr p) 0))))

(handle-output-geometry! 0 0 1280 720)

;; ---------------------------------------------------------------------
;; gnew-dynamic!: creates, switches, tiles as windows arrive.
;; ---------------------------------------------------------------------

(gnew-dynamic! " dyn ")
(check "gnew-dynamic! switched" (current-group-name) " dyn ")
(check-true "group is dynamic" (dynamic-group?))

(wm-on-window-map 1 "one" "foot")
(check "one window fills the head (minus border inset)"
       (length (frame-leaves (current-tree))) 1)
(check-true "window 1 visible" (visible? 1))

(wm-on-window-map 2 "two" "foot")
(check "two frames after second map"
       (length (frame-leaves (current-tree))) 2)
;; Master = newest (2), at 2/3 of 1280 on the left.
(let ((m (rect 2)) (s (rect 1)))
  (check-true "master starts at the left edge region" (< (car m) 20))
  (check-true "master is ~2/3 wide" (and (> (caddr m) 800) (< (caddr m) 880)))
  (check-true "stack window sits right of the master" (> (car s) 800))
  (check-true "stack window is full height" (> (cadddr s) 650)))
(check "focus went to the new master" %focused 2)

(wm-on-window-map 3 "three" "foot")
(check "three frames after third map"
       (length (frame-leaves (current-tree))) 3)
(let ((m (rect 3)) (s1 (rect 2)) (s2 (rect 1)))
  (check-true "newest window is the master" (< (car m) 20))
  (check-true "old master moved to the stack" (> (car s1) 800))
  (check-true "stack splits the height"
              (and (< (cadddr s1) 400) (< (cadddr s2) 400)))
  (check-true "stack windows don't overlap"
              (not (= (cadr s1) (cadr s2)))))

;; ---------------------------------------------------------------------
;; Unmap: master vanishes, next window is promoted.
;; ---------------------------------------------------------------------

(wm-on-window-unmap 3)
(check "back to two frames" (length (frame-leaves (current-tree))) 2)
(check-true "window 2 promoted to master" (< (car (rect 2)) 20))
(check-true "window 1 still stacks" (> (car (rect 1)) 800))

;; ---------------------------------------------------------------------
;; rotate-windows! / rotate-stack! / exchange-with-master!
;; ---------------------------------------------------------------------

(wm-on-window-map 4 "four" "foot") ; order now (4 2 1)
(check-true "4 is master" (< (car (rect 4)) 20))
(rotate-windows! 'forward) ; (1 4 2)
(check-true "rotate: 1 became master" (< (car (rect 1)) 20))
(rotate-windows! 'backward) ; (4 2 1)
(check-true "rotate back: 4 is master again" (< (car (rect 4)) 20))
(rotate-stack! 'forward) ; stack (2 1) -> (1 2)
(check-true "rotate-stack kept the master" (< (car (rect 4)) 20))
(check-true "stack order swapped" (< (cadr (rect 1)) (cadr (rect 2))))

(focus-window-by-id! 2)
(exchange-with-master!)
(check-true "exchange: 2 is master now" (< (car (rect 2)) 20))
(check "focus follows the exchanged window" %focused 2)

;; ---------------------------------------------------------------------
;; change-split-ratio! / change-layout!
;; ---------------------------------------------------------------------

(change-split-ratio! 1/2)
(check-true "ratio 1/2: master is half the width"
            (let ((w (caddr (rect 2)))) (and (> w 600) (< w 680))))
(change-layout! 'right)
(check-true "layout right: master on the right"
            (> (car (rect 2)) 600))
(check-true "layout right: stack on the left"
            (< (car (rect 4)) 20))
(change-layout! 'top)
(check-true "layout top: master on top"
            (and (< (cadr (rect 2)) 20) (< (car (rect 2)) 20)))
(check-true "layout top: master full width" (> (caddr (rect 2)) 1200))
(change-layout! 'left)

;; ---------------------------------------------------------------------
;; gmove into a dynamic group retiles it; float excludes a window.
;; ---------------------------------------------------------------------

(gnewbg! " manual ")
(switch-to-group! " manual ")
(wm-on-window-map 5 "five" "foot")
(switch-to-group! " dyn ")
(gmerge! " manual ") ; pulls 5 into the dynamic group
(check "gmerge into dynamic retiled: 4 frames"
       (length (frame-leaves (current-tree))) 4)

(focus-window-by-id! 5)
(float-window! 5)
(retile-dynamic!)
(check "float excluded from the tiling: 3 frames"
       (length (frame-leaves (current-tree))) 3)
(unfloat-window! 5)
(retile-dynamic!)
(check "unfloat re-enters the tiling: 4 frames"
       (length (frame-leaves (current-tree))) 4)

;; ---------------------------------------------------------------------
;; gnewbg-dynamic! stays in the background; switching retiles it.
;; ---------------------------------------------------------------------

(gnewbg-dynamic! " dyn2 ")
(check "gnewbg-dynamic! did not switch" (current-group-name) " dyn ")
(switch-to-group! " dyn2 ")
(check-true "dyn2 is dynamic" (dynamic-group?))

;; ---------------------------------------------------------------------
;; retile! / rotate on a manual group refuses politely.
;; ---------------------------------------------------------------------

(switch-to-group! " I ")
(set! %messages '())
(retile!)
(check-true "retile! refuses in a manual group"
            (and (pair? %messages)
                 (string-contains (car %messages) "not a dynamic group")))
(rotate-windows! 'forward)
(check-true "rotate refuses in a manual group"
            (string-contains (car %messages) "not a dynamic group"))

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
