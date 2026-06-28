;;; floats-test.scm -- Guile-only unit test of the floating-window layer.
;;;
;;; Run with: guile -L scheme tests/floats-test.scm
;;;
;;; Same stub-before-load pattern as frames-test.scm: every wm-* Rust
;;; subr the float code touches is recorded here.

(use-modules (srfi srfi-1))

;; ---------------------------------------------------------------------
;; Stubs
;; ---------------------------------------------------------------------

(define %placements (make-hash-table))       ; id -> (x y w h) via wm-place-window
(define %float-placements (make-hash-table)) ; id -> (x y w h) via wm-place-float
(define %floating-flags (make-hash-table))   ; id -> bool via wm-set-floating
(define %raised '())                         ; raise order, newest first
(define %focused #f)

(define (wm-place-window id x y w h)
  (hash-set! %placements id (list x y w h)) #t)
(define (wm-place-float id x y w h)
  (hash-set! %float-placements id (list x y w h)) #t)
(define (wm-set-floating id on)
  (hash-set! %floating-flags id on) #t)
(define (wm-raise-window id)
  (set! %raised (cons id %raised)) #t)
(define (wm-focus-window id) (set! %focused id) #t)
(define (wm-clear-focus) (set! %focused #f) #t)
(define (wm-focus-rect x y w h) #t)
(define (wm-close-window id) #t)
(define (wm-message text . rest) #t)
(define (wm-clear-message) #t)
(define (wm-log msg) #t)

(use-modules (minde frames) (minde groups))

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

;; ---------------------------------------------------------------------
;; Setup: one head, two windows in the default group
;; ---------------------------------------------------------------------

(wm-on-heads-changed '((0 0 0 1280 720)))
(wm-on-window-map 1 "one" "app-one")
(wm-on-window-map 2 "two" "app-two")

(check "two tiled windows to start" (frame-tree-window-count) 2)

;; ---------------------------------------------------------------------
;; float / unfloat
;; ---------------------------------------------------------------------

(float-this!) ; floats window 2 (the focused one)

(check-true "window 2 is floating" (window-floating? 2))
(check "float leaves the tree" (frame-tree-window-count) 1)
(check-true "rust told it's floating" (hash-ref %floating-flags 2))
(check-true "float placed via wm-place-float" (hash-ref %float-placements 2))
(check-true "float raised" (member 2 %raised))
(check "float focus" (focused-window-id) 2)
(check "keyboard focus followed" %focused 2)
(check-true "float still in all-window-ids" (member 2 (all-window-ids)))
(check-true "default rect within head"
            (let ((r (float-geometry 2)))
              (and (>= (car r) 0) (>= (cadr r) 0)
                   (<= (+ (car r) (caddr r)) 1280)
                   (<= (+ (cadr r) (cadddr r)) 720))))

;; Tiled placement never touched the float after floating.
(hash-remove! %placements 2)
(sync-frames!)
(check-false "sync does not tile-place a float" (hash-ref %placements 2))

;; Super+drag result flows back into the geometry table.
(wm-on-window-moved 2 40 50 300 200)
(check "wm-on-window-moved updates geometry" (float-geometry 2) '(40 50 300 200))
(sync-frames!)
(check "sync re-places the dragged rect" (hash-ref %float-placements 2) '(40 50 300 200))

;; Cycling reaches the float and back.
(focus-next-window!) ; from float 2 -> window 1
(check "cycle float -> tiled" (focused-window-id) 1)
(focus-next-window!) ; -> back to float 2
(check "cycle tiled -> float" (focused-window-id) 2)

;; Unfloat puts it back into the current frame.
(float-this!)
(check-false "window 2 unfloated" (window-floating? 2))
(check "back in the tree" (frame-tree-window-count) 2)
(check "rust flag cleared" (hash-ref %floating-flags 2) #f)

;; ---------------------------------------------------------------------
;; Float groups (gnew-float)
;; ---------------------------------------------------------------------

(gnew-float! " FL ")
(switch-to-group! " FL ")
(wm-on-window-map 3 "three" "app-three")

(check-true "window in a float group floats" (window-floating? 3))
(check "float group tree stays empty" (frame-tree-window-count) 0)
(check "float focused on map" (focused-window-id) 3)

;; gmove keeps float status: move 3 to the next group.
(move-window-to-next-group!)
(check-true "moved float still floating" (window-floating? 3))
(check-false "moved float left the group" (group-has-window? " FL " 3))
(check-true "next group adopted it" (group-has-window? " I " 3))

;; Switching to the float's group shows it again.
(switch-to-group! " I ")
(check-true "float visible after switch"
            (equal? (take (hash-ref %float-placements 3) 2)
                    (take (float-geometry 3) 2)))

;; ---------------------------------------------------------------------
;; Head changes clamp float geometry
;; ---------------------------------------------------------------------

(wm-on-window-moved 3 1200 600 400 300) ; hangs off the 1280x720 edge
(wm-on-heads-changed '((0 0 0 1024 600)))
(check-true "float clamped into the new head union"
            (let ((r (float-geometry 3)))
              (and (<= (+ (car r) (caddr r)) 1024)
                   (<= (+ (cadr r) (cadddr r)) 600))))

;; ---------------------------------------------------------------------
;; Unmap cleans everything up
;; ---------------------------------------------------------------------

(wm-on-window-unmap 3)
(check-false "unmapped float forgotten" (window-floating? 3))
(check-false "unmapped float out of the group" (group-has-window? " I " 3))
(check-false "unmapped float out of all-window-ids" (member 3 (all-window-ids)))

;; pull-by-number unfloats.
(float-window! 2)
(check-true "window 2 floating again" (window-floating? 2))
(pull-window-by-number! (window-number 2))
(check-false "pull unfloats" (window-floating? 2))

(if (zero? %failures)
    (format #t "all tests passed~%")
    (begin (format #t "~a failure(s)~%" %failures)
           (exit 1)))
