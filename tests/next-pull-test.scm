;;; next-pull-test.scm -- StumpWM `next` / `pull` semantics:
;;; focus-next-window! cycles the whole group across frames;
;;; pull-hidden-next! only moves hidden windows.
;;;
;;; Run with: guile -L scheme tests/next-pull-test.scm

(use-modules (srfi srfi-1))

(define %placements (make-hash-table)) ; id -> (x y w h)
(define %focused #f)

(define (wm-place-window id x y w h) (hash-set! %placements id (list x y w h)) #t)
(define (wm-focus-window id) (set! %focused id) #t)
(define (wm-close-window id) #t)
(define (wm-clear-focus) (set! %focused #f) #t)
(define (wm-focus-rect x y w h) #t)
(define (wm-output-geometry) (list 0 0 1280 720))
(define (wm-log msg) #t)

(use-modules (minde compositor frames))

(define %failures 0)
(define (check name got expected)
  (if (equal? got expected)
      (format #t "ok - ~a~%" name)
      (begin
        (set! %failures (+ %failures 1))
        (format #t "FAIL - ~a: expected ~s, got ~s~%" name expected got))))

(define (visible? id)
  (let ((p (hash-ref %placements id)))
    (and p (>= (car p) 0))))

;; Layout: two frames side by side. Left frame holds windows 1 and 2
;; (2 visible, 1 hidden behind it), right frame holds window 3.
(update-output-geometry! 0 0 1280 720)
(track-window-map! 1 "one" "app")
(track-window-map! 2 "two" "app")
(split-frame-horizontal!)      ; 1,2 stay in the left frame, which is current
(focus-next-frame!)            ; move to the empty right frame
(track-window-map! 3 "three" "app")

(check "window 3 focused in right frame" %focused 3)
(check "window 2 visible in left frame" (visible? 2) #t)
(check "window 1 hidden" (visible? 1) #f)

;; focus-next-window! from 3 cycles group-wide: 1 -> 2 -> 3 -> 1 ...
(focus-next-window!)
(check "next from 3 wraps to 1 (crosses frames, raises hidden)" %focused 1)
(check "window 1 now visible" (visible? 1) #t)
(check "window 2 now hidden behind 1" (visible? 2) #f)

(focus-next-window!)
(check "next again reaches 2 (same frame stack)" %focused 2)

(focus-next-window!)
(check "next again jumps to 3 in the other frame" %focused 3)

;; pull-hidden-next! from the right frame: window 1 is hidden (2 is
;; visible on the left) -> 1 moves into the right frame; 2 must stay put.
(pull-hidden-next!)
(check "pull brought hidden window 1 here" %focused 1)
(check "window 2 still visible in its own frame" (visible? 2) #t)
(check "window 1 and 3 now share the right frame stack"
       (frame-tree-window-count) 3)

;; Nothing hidden in the left frame's stack now except 3 (behind 1).
;; Pull again: 3 is hidden in *this* frame -- StumpWM pull would raise
;; the next hidden window regardless of frame; it lands here (no-op move
;; within the same frame is acceptable: it becomes visible).
(pull-hidden-next!)
(check "pull raises remaining hidden window 3" %focused 3)

;; With 3 windows in 2 frames one is always hidden (1 behind 3 now), so
;; pull keeps cycling the hidden stack.
(pull-hidden-next!)
(check "pull keeps cycling the hidden stack" %focused 1)

;; True no-op case: close the extra window so everything fits on screen.
(remove-window-from-active-tree! 1)
(pull-hidden-next!)
(check "pull with nothing hidden is a no-op" (current-frame-window) 3)

(if (zero? %failures)
    (begin (format #t "all tests passed~%") (exit 0))
    (begin (format #t "~a test(s) FAILED~%" %failures) (exit 1)))
