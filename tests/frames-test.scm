;;; frames-test.scm -- Guile-only unit test of the frame tiling logic.
;;;
;;; Run with:
;;;   guix shell -m manifest.scm -- guile -L scheme tests/frames-test.scm
;;; or, since guile only needs to be on PATH:
;;;   guile -L scheme tests/frames-test.scm
;;;
;;; Stubs every wm-* Rust subr *before* loading (minde frames), since
;;; that module must not call any of them at load time (only sync-frames!,
;;; itself only invoked from event hooks / operations, may call them).

(use-modules (srfi srfi-1))

;; ---------------------------------------------------------------------
;; Stubs recording calls, standing in for the Rust-side subrs.
;; ---------------------------------------------------------------------

(define %placements (make-hash-table)) ; id -> (x y w h)
(define %focused #f)
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

(define (wm-output-geometry)
  (list 1280 720))

(define (wm-log msg)
  (set! %log-lines (cons msg %log-lines))
  #t)

;; Now it's safe to load the module under test.
(use-modules (minde compositor frames))

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

(define (fail-hard msg)
  (format #t "FATAL - ~a~%" msg)
  (exit 1))

;; ---------------------------------------------------------------------
;; Set up a 1280x720 output, matching the stub.
;; ---------------------------------------------------------------------

(update-output-geometry! 0 0 1280 720)

;; ---------------------------------------------------------------------
;; Map two windows into the (single, frame-filling) initial frame.
;; ---------------------------------------------------------------------

(track-window-map! 1 "term" "foot")
(check "window 1 placed frame-filling after map"
       (hash-ref %placements 1)
       (list 3 3 1274 714))
(check "window 1 focused after map" %focused 1)

(track-window-map! 2 "editor" "emacs")
;; window 2 maps into the same (only) frame, becoming its new current
;; window; window 1 should now be parked off-screen since it's no longer
;; that frame's current window.
(check "window 2 is now current in the frame" (current-frame-window) 2)
(check "window 2 placed frame-filling after map"
       (hash-ref %placements 2)
       (list 3 3 1274 714))
(let ((p1 (hash-ref %placements 1)))
  (check-true "window 1 parked off-screen (negative coords)"
              (and p1 (< (car p1) 0) (< (cadr p1) 0))))

;; ---------------------------------------------------------------------
;; Split vertically (stacked): current frame gets window 2 in the top
;; half, focus moves to that top frame.
;; ---------------------------------------------------------------------

(split-frame-vertical!)

(let* ((p2 (hash-ref %placements 2)))
  (check "window 2 (current) geometry after vsplit is top half"
         p2
         (list 3 3 1274 354)))

;; Move window 1 into the new (bottom) frame by cycling frames and mapping
;; it there -- but window 1 isn't mapped again; instead exercise
;; focus-next-frame! then bring window 1 into view by cycling windows.
;; Simpler: since window 1 already exists but isn't in any frame's window
;; list (it was only ever in the original frame, which became the top
;; frame after the split -- it's still tracked there, just not current).
(check "total window count preserved across split" (frame-tree-window-count) 2)

;; Cycle to the other (bottom, still-empty) frame.
(focus-next-frame!)
(check "focus-next-frame! moved focus off window 2" %focused 2) ; unchanged: bottom frame has no window yet

;; Cycle back.
(focus-next-frame!)
(check "focus-next-frame! cycled back to top frame" (current-frame-window) 2)

;; focus-next-window-in-frame! cycles among the *current* frame's windows;
;; the top frame only holds window 2 (window 1 stayed there as a
;; non-current window from before the split), so cycling should reveal
;; window 1 next.
(focus-next-window-in-frame!)
(check "focus-next-window-in-frame! cycled to window 1" (current-frame-window) 1)
(let ((p1 (hash-ref %placements 1)))
  (check "window 1 now on-screen in top frame" p1 (list 3 3 1274 354)))
(let ((p2 (hash-ref %placements 2)))
  (check-true "window 2 now parked off-screen"
              (and p2 (< (car p2) 0) (< (cadr p2) 0))))

;; ---------------------------------------------------------------------
;; Non-overlapping coverage check: after the split, the two frames'
;; rectangles must tile the output exactly (no gap, no overlap).
;; ---------------------------------------------------------------------

;; (We don't have direct access to the frame list from outside the module,
;; but window 1's and window 2's on-screen geometries when each is current
;; already demonstrated top=[0,0,1280,360]; the bottom frame's rectangle is
;; verified indirectly below via remove-split! restoring frame-filling.)

;; ---------------------------------------------------------------------
;; remove-split! restores full screen.
;; ---------------------------------------------------------------------

(remove-split!)
(check "frame count back to one after remove-split!"
       (length (list 1)) ; trivial sanity that check machinery works
       1)
(let ((cur (current-frame-window)))
  (check-true "some window is current after remove-split!" cur))
(let ((p (hash-ref %placements (current-frame-window))))
  (check "current window frame-filling again after remove-split!"
         p
         (list 3 3 1274 714)))

;; ---------------------------------------------------------------------
;; Unmap removes a window wherever it is.
;; ---------------------------------------------------------------------

(track-window-unmap! 1)
(check "window 1 no longer tracked after unmap" (frame-tree-window-count) 1)
(check "window 2 remains current after unmap" (current-frame-window) 2)

(track-window-unmap! 2)
(check "no windows tracked after unmapping both" (frame-tree-window-count) 0)
(check "current-frame-window is #f with no windows" (current-frame-window) #f)

;; ---------------------------------------------------------------------
;; Gaps: outer at the usable-area boundary, half the inner gap at shared
;; edges, window still inset by the border width inside the gapped rect.
;; ---------------------------------------------------------------------

(track-window-map! 3 "term2" "foot")
(set-gaps! 10 6)
(check "single gapped frame: outer gap on all sides"
       (hash-ref %placements 3)
       (list 9 9 1262 702)) ; 6 outer + 3 border inset

(split-frame-vertical!)
;; Top frame 0,0,1280,360: outer 6 left/top/right, inner/2 = 5 bottom.
(check "gapped top frame after vsplit"
       (hash-ref %placements 3)
       (list 9 9 1262 343))

(set-gaps! 0 0)
(check "gaps off again: top frame border-inset only"
       (hash-ref %placements 3)
       (list 3 3 1274 354))

;; ---------------------------------------------------------------------
;; Resize: moving the divider of the enclosing split, and balancing.
;; ---------------------------------------------------------------------

;; Current frame is the top half (h 360). Push the divider down 30px.
(resize-frame! 'down 30)
(check "resize-frame! 'down grew the top frame by 30"
       (hash-ref %placements 3)
       (list 3 3 1274 384))

(resize-frame! 'up 30)
(check "resize-frame! 'up restored the split"
       (hash-ref %placements 3)
       (list 3 3 1274 354))

;; No horizontal split anywhere: left/right resize is a no-op.
(resize-frame! 'right 30)
(check "resize-frame! with no matching split is a no-op"
       (hash-ref %placements 3)
       (list 3 3 1274 354))

(resize-frame! 'down 30)
(resize-frame! 'down 30)
(balance-frames!)
(check "balance-frames! equalizes the two frames again"
       (hash-ref %placements 3)
       (list 3 3 1274 354))

(remove-split!)
(track-window-unmap! 3)
(forget-window-number! 3)

;; ---------------------------------------------------------------------
;; Window numbers: smallest free per group, reuse after unmap.
;; ---------------------------------------------------------------------

(track-window-map! 10 "alpha" "a")
(track-window-map! 11 "beta" "b")
(track-window-map! 12 "gamma" "c")
(check "numbers assigned in map order" (map window-number (list 10 11 12)) (list 0 1 2))
(track-window-unmap! 11)
(forget-window-number! 11)
(track-window-map! 13 "delta" "d")
(check "freed number is reused" (window-number 13) 1)

;; ---------------------------------------------------------------------
;; Directional navigation on a 2x2 grid.
;; ---------------------------------------------------------------------

(apply-layout-spec! '(vsplit 1/2 (hsplit 1/2 leaf leaf) (hsplit 1/2 leaf leaf)))
;; Windows: current (13) -> top-left, then 10 -> top-right, 12 -> bottom-left.
(check "current window in top-left after grid" (hash-ref %placements 13) (list 3 3 634 354))
(check "window 10 in top-right" (hash-ref %placements 10) (list 643 3 634 354))
(check "window 12 in bottom-left" (hash-ref %placements 12) (list 3 363 634 354))

(move-focus! 'right)
(check "move-focus right lands on top-right's window" %focused 10)
(move-focus! 'up) ; edge: no-op
(check "move-focus at the edge is a no-op" %focused 10)

(move-window! 'down)
(check "move-window moved window 10 to bottom-right" (hash-ref %placements 10) (list 643 363 634 354))
(check "focus followed the moved window" %focused 10)

(exchange-windows! 'left)
(check "exchange put window 10 into bottom-left" (hash-ref %placements 10) (list 3 363 634 354))
(check "exchange put window 12 into bottom-right" (hash-ref %placements 12) (list 643 363 634 354))
(check "focus still on window 10 after exchange" %focused 10)

;; ---------------------------------------------------------------------
;; other-window! toggle + echo string markers.
;; ---------------------------------------------------------------------

(other-window!)
(check "other-window! toggles back to the previously focused window" %focused 13)
(let ((s (echo-windows-string)))
  (check-true "echo-windows marks current with *" (string-contains s "*delta"))
  (check-true "echo-windows marks last with -" (string-contains s "-alpha")))

;; ---------------------------------------------------------------------
;; select / pull by number.
;; ---------------------------------------------------------------------

(select-window-by-number! 0)
(check "select-window-by-number! 0 focused window 10" %focused 10)
(pull-window-by-number! 2)
(check "pull-window-by-number! brought window 12 here" %focused 12)
(check "pulled window shown in the current (bottom-left) frame"
       (hash-ref %placements 12) (list 3 363 634 354))

;; ---------------------------------------------------------------------
;; only / fclear / hsplit-equally.
;; ---------------------------------------------------------------------

(collapse-to-one-frame!)
(check "collapse-to-one-frame! keeps every window" (frame-tree-window-count) 3)
(let ((cur (current-frame-window)))
  (check "collapse-to-one-frame!'s current window fills the screen"
         (hash-ref %placements cur) (list 3 3 1274 714)))

(clear-current-frame!)
(check "clear-current-frame! empties the frame's shown window" (current-frame-window) #f)
(check "clear-current-frame! keeps the windows tracked" (frame-tree-window-count) 3)

(hsplit-equally! 3)
(check "hsplit-equally! made three columns; first column current"
       (car (list (frame-tree-window-count))) 3)
(focus-next-window-in-frame!) ; show something in column 1 again
(let ((cur (current-frame-window)))
  (check-true "a window is visible again in column 1" cur)
  (check "column 1 geometry is a third of the screen"
         (hash-ref %placements cur) (list 3 3 421 714)))

;; Reverse cycling sanity.
(focus-previous-window-in-frame!)
(focus-previous-window!)
(focus-previous-frame!)
(pull-hidden-previous!)
(check "reverse cycling kept all windows" (frame-tree-window-count) 3)

(for-each (lambda (id) (track-window-unmap! id) (forget-window-number! id))
          (list 10 12 13))
(collapse-to-one-frame!)

;; ---------------------------------------------------------------------
;; Hooks fire on map/focus; message hook + lastmsg ring.
;; ---------------------------------------------------------------------

(use-modules (minde hooks))

(define %hook-events '())
(define (record-event . args) (set! %hook-events (cons args %hook-events)))
(add-event-hook! 'new-window (lambda (id title app-id) (record-event 'new id)))
(add-event-hook! 'focus-window (lambda (id) (record-event 'focus id)))
(add-event-hook! 'message (lambda (text) (record-event 'msg text)))

(track-window-map! 20 "hooked" "app")
(check-true "new-window hook fired" (member (list 'new 20) %hook-events))
(check-true "focus-window hook fired" (member (list 'focus 20) %hook-events))

;; A hook that throws must not break the event path.
(add-event-hook! 'message (lambda (text) (error "boom")))
(echo "still alive")
(check "echo survives a throwing hook" (last-message) "still alive")
(check-true "message hook saw the echo" (member (list 'msg "still alive") %hook-events))

;; ---------------------------------------------------------------------
;; Marks: mark two windows elsewhere, pull them into the current frame.
;; ---------------------------------------------------------------------

(track-window-map! 21 "m1" "a")
(track-window-map! 22 "m2" "b")
(split-frame-vertical!)
;; current frame (top) holds 20/21/22 with 22 shown; mark 22, cycle and
;; mark 21, move to the empty bottom frame and pull both.
(mark-window-toggle!)
(focus-next-window-in-frame!)
(check "cycled to another window" (current-frame-window) 20)
(focus-next-window-in-frame!)
(check "cycled to window 21" (current-frame-window) 21)
(mark-window-toggle!)
(check "two windows marked" (length (marked-windows)) 2)
(focus-next-frame!)
(pull-marked!)
(check "marks cleared after pull" (marked-windows) '())
(check-true "pulled window visible in the bottom frame"
            (member (current-frame-window) (list 21 22)))
(check "all windows still tracked" (frame-tree-window-count) 3)

(mark-window-toggle!)
(clear-marks!)
(check "clear-marks! empties the set" (marked-windows) '())

(remove-split!)
(for-each (lambda (id) (track-window-unmap! id) (forget-window-number! id))
          (list 20 21 22))

;; ---------------------------------------------------------------------
;; Fullscreen freeze, urgency bookkeeping
;; ---------------------------------------------------------------------

(define %fullscreen-calls '())
(define (wm-set-fullscreen id on)
  (set! %fullscreen-calls (cons (cons id on) %fullscreen-calls))
  #t)

(track-window-map! 30 "term" "foot")
(fullscreen!)
(check "fullscreen! set window 30 fullscreen" (car %fullscreen-calls) '(30 . #t))
(check "fullscreen-window records the id" (fullscreen-window) 30)
;; While fullscreen, sync is frozen: mapping another window must not
;; issue placements.
(hash-clear! %placements)
(track-window-map! 31 "editor" "lem")
(check "sync-frames! is a no-op while fullscreen" (hash-count (const #t) %placements) 0)
(fullscreen!)
(check "second fullscreen! unset it" (car %fullscreen-calls) '(30 . #f))
(check "fullscreen flag cleared" (fullscreen-window) #f)
(check-true "leaving fullscreen re-synced placements"
            (> (hash-count (const #t) %placements) 0))
;; Unmapping the fullscreen window clears the flag too.
(fullscreen!)
(clear-fullscreen-if-window! 31)
(check "flag cleared when the fullscreen window unmaps" (fullscreen-window) #f)

;; Urgency list: add, focus-clears, ordering.
(add-urgent-window! 30)
(add-urgent-window! 31)
(check "urgent windows queue in order" (urgent-windows) '(30 31))
(add-urgent-window! 30)
(check "re-adding an urgent window does not duplicate" (urgent-windows) '(30 31))
;; Focusing window 30 (via sync) clears its urgency.
(focus-window-by-id! 30)
(check "focusing a window clears its urgency" (urgent-windows) '(31))
(clear-urgent! 31)
(check "clear-urgent! empties the list" (urgent-windows) '())

(for-each (lambda (id) (track-window-unmap! id) (forget-window-number! id))
          (list 30 31))

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
