;;; groups-test.scm -- Guile-only unit test of the group (workspace) logic.
;;;
;;; Run with:
;;;   guile -L scheme tests/groups-test.scm
;;;
;;; Stubs every wm-* Rust subr *before* loading (minde frames)/(minde
;;; groups), same pattern as tests/frames-test.scm.

(use-modules (srfi srfi-1))

;; ---------------------------------------------------------------------
;; Stubs recording calls, standing in for the Rust-side subrs.
;; ---------------------------------------------------------------------

(define %placements (make-hash-table)) ; id -> (x y w h)
(define %focused #f)
(define %focus-cleared 0)
(define %closed '())
(define %log-lines '())

(define (wm-place-window id x y w h)
  (hash-set! %placements id (list x y w h))
  #t)

(define (wm-focus-window id)
  (set! %focused id)
  #t)

(define (wm-close-window id)
  (set! %closed (cons id %closed))
  #t)

(define (wm-clear-focus)
  (set! %focused #f)
  (set! %focus-cleared (+ %focus-cleared 1))
  #t)

(define (wm-output-geometry)
  (list 1280 720))

(define (wm-log msg)
  (set! %log-lines (cons msg %log-lines))
  #t)

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

(define (offscreen? p)
  (and p (< (car p) 0) (< (cadr p) 0)))

;; ---------------------------------------------------------------------
;; Set up a 1280x720 output.
;; ---------------------------------------------------------------------

(handle-output-geometry! 1280 720)

(check "default groups are I, II, III" (group-names) (list " I " " II " " III "))
(check "current group starts as I" (current-group-name) " I ")

;; ---------------------------------------------------------------------
;; (a) Map two windows in group I, switch to II -> both parked off-screen,
;; focus cleared.
;; ---------------------------------------------------------------------

(wm-on-window-map 1 "term" "foot")
(wm-on-window-map 2 "editor" "emacs")

(check "window 2 is current after both maps" (current-frame-window) 2)
(check-true "window 2 on-screen before switch" (not (offscreen? (hash-ref %placements 2))))

(switch-to-group! " II ")

(check "current group is now II" (current-group-name) " II ")
(check-true "window 1 parked off-screen after switching away from I"
            (offscreen? (hash-ref %placements 1)))
(check-true "window 2 parked off-screen after switching away from I"
            (offscreen? (hash-ref %placements 2)))
(check "focus cleared on switch to an empty group" %focused #f)

;; ---------------------------------------------------------------------
;; (b) Switch back -> windows placed back at frame geometry, focus
;; restored.
;; ---------------------------------------------------------------------

(switch-to-group! " I ")

(check "current group is I again" (current-group-name) " I ")
(check "window 2 back on-screen full-screen" (hash-ref %placements 2) (list 0 0 1280 720))
(check "focus restored to window 2" %focused 2)
(check-true "window 1 still parked off-screen (not current in its frame)"
            (offscreen? (hash-ref %placements 1)))

;; ---------------------------------------------------------------------
;; (c) move-window-to-next-group! moves the current window (2) from I to
;; II; after switching to II it's placed.
;; ---------------------------------------------------------------------

(move-window-to-next-group!)

(check "window 2 gone from group I" (group-has-window? " I " 2) #f)
(check "window 2 now present in group II" (group-has-window? " II " 2) #t)
(check "window 1 still in group I" (group-has-window? " I " 1) #t)
(check "current group stays I after move-window-to-next-group!" (current-group-name) " I ")
(check-true "window 2 parked off-screen after being moved away" (offscreen? (hash-ref %placements 2)))
(check "window 1 becomes current group I's window after 2 left" (current-frame-window) 1)

(switch-to-group! " II ")
(check "window 2 placed full-screen after switching to II where it now lives"
       (hash-ref %placements 2)
       (list 0 0 1280 720))
(check "focus on window 2 in group II" %focused 2)

;; Back to I for the remaining tests.
(switch-to-group! " I ")

;; ---------------------------------------------------------------------
;; (d) Unmap of a window in a hidden group removes it from that group's
;; tree.
;; ---------------------------------------------------------------------

(check "window 2 is in hidden group II before unmap" (group-has-window? " II " 2) #t)
(wm-on-window-unmap 2)
(check "window 2 removed from group II after unmap while hidden" (group-has-window? " II " 2) #f)
(check "window 1 (in the current, active group) unaffected" (group-has-window? " I " 1) #t)

;; ---------------------------------------------------------------------
;; (e) pull-window-from-other-frame! moves a window from the other frame
;; after a split.
;; ---------------------------------------------------------------------

;; Group I currently: single frame with window 1 current. Map a second
;; window (3), then split so 1 and 3 end up in different frames.
(wm-on-window-map 3 "browser" "zen")
(check "window 3 current after map" (current-frame-window) 3)

(split-frame-vertical!)
;; After the vsplit, both windows 1 and 3 are tracked by the *original*
;; frame (now the top half, and still current); the bottom frame is new
;; and empty. Move to the bottom frame before pulling.
(check "top (still current) frame keeps window 3" (current-frame-window) 3)

(focus-next-frame!)
(check "bottom (new current) frame has no window yet" (current-frame-window) #f)

(pull-window-from-other-frame!)
(check-true "pull-window-from-other-frame! brought a window into the current frame"
            (current-frame-window))
(let ((pulled (current-frame-window)))
  (check-true "pulled window is one of 1 or 3" (member pulled (list 1 3))))

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
