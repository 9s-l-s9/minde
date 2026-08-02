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
(define %sent-strings '())
(define %clicks '())
(define (wm-send-string text) (set! %sent-strings (cons text %sent-strings)) #t)
(define (wm-click button) (set! %clicks (cons button %clicks)) #t)
(define (wm-idle-ms) 1234)

(use-modules (minde compositor frames) (minde groups))

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

(handle-heads-change! '((0 0 0 1280 720)))
(handle-window-map! 1 "one" "app-one")
(handle-window-map! 2 "two" "app-two")

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
(handle-window-move! 2 40 50 300 200)
(check "handle-window-move! updates geometry" (float-geometry 2) '(40 50 300 200))
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

(create-floating-group! " FL ")
(switch-to-group! " FL ")
(handle-window-map! 3 "three" "app-three")

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

(handle-window-move! 3 1200 600 400 300) ; hangs off the 1280x720 edge
(handle-heads-change! '((0 0 0 1024 600)))
(check-true "float clamped into the new head union"
            (let ((r (float-geometry 3)))
              (and (<= (+ (car r) (caddr r)) 1024)
                   (<= (+ (cadr r) (cadddr r)) 600))))

;; ---------------------------------------------------------------------
;; Unmap cleans everything up
;; ---------------------------------------------------------------------

(handle-window-unmap! 3)
(check-false "unmapped float forgotten" (window-floating? 3))
(check-false "unmapped float out of the group" (group-has-window? " I " 3))
(check-false "unmapped float out of all-window-ids" (member 3 (all-window-ids)))

;; pull-by-number unfloats.
(float-window! 2)
(check-true "window 2 floating again" (window-floating? 2))
(pull-window-by-number! (window-number 2))
(check-false "pull unfloats" (window-floating? 2))

;; ---------------------------------------------------------------------
;; flatten-floats, always-on-top, rename, place
;; existing, send-string/ratclick/idle wrappers
;; ---------------------------------------------------------------------

(float-window! 1)
(float-window! 2)
(check "two floats before flatten" (length (group-floats (current-group))) 2)
(flatten-floats!)
(check "flatten leaves no floats" (group-floats (current-group)) '())
(check-false "window 1 no longer floating" (window-floating? 1))
(check "both back in the tree" (frame-tree-window-count) 2)

;; Always-on-top: after a sync the ontop window is the last raise.
(focus-window-by-id! 1)
(toggle-always-on-top!)
(check-true "window 1 marked ontop" (member 1 (ontop-windows)))
(set! %raised '())
(sync-frames!)
(check "ontop raised last in sync" (and (pair? %raised) (car %raised)) 1)
(toggle-always-on-top!)
(check-false "ontop toggles off" (member 1 (ontop-windows)))

;; Rename (StumpWM title).
(rename-window! "my-editor")
(check "rename-window! overrides the title" (window-title 1) "my-editor")

;; place-existing-windows! re-applies rules to mapped windows.
(add-placement-rule! "two" #:group " II ")
(place-existing-windows!)
(check-false "rule moved window 2 out of I" (group-has-window? " I " 2))
(check-true "window 2 now tracked by II" (group-has-window? " II " 2))
(clear-placement-rules!)

;; Thin wrappers round-trip to the Rust stubs.
(window-send-string "hello")
(check "window-send-string forwards" %sent-strings '("hello"))
(ratclick! 1)
(check "ratclick forwards" %clicks '(1))
(check "idle-ms reads the subr" (idle-ms) 1234)

(if (zero? %failures)
    (format #t "all tests passed~%")
    (begin (format #t "~a failure(s)~%" %failures)
           (exit 1)))
