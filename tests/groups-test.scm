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

(handle-output-geometry! 0 0 1280 720)

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
(check "window 2 back on-screen frame-filling" (hash-ref %placements 2) (list 3 3 1274 714))
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
(check "window 2 placed frame-filling after switching to II where it now lives"
       (hash-ref %placements 2)
       (list 3 3 1274 714))
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
;; (f) Placement rules route mapping windows by app-id/title.
;; ---------------------------------------------------------------------

;; By app-id, into hidden group II: window must land there parked, with
;; the current group unchanged.
(add-placement-rule! "zen-two" #:group "II" #:frame 0)
(wm-on-window-map 4 "some page" "zen-two")
(check "rule-matched window landed in group II" (group-has-window? " II " 4) #t)
(check "rule-matched window not in group I" (group-has-window? " I " 4) #f)
(check "current group unchanged by non-follow rule" (current-group-name) " I ")
(check-true "rule-matched window parked off-screen"
            (offscreen? (hash-ref %placements 4)))

;; By title, with #:follow?: switches to the target group and shows it.
(add-placement-rule! "follow-me" #:group "III" #:follow? #t)
(wm-on-window-map 5 "follow-me notes" "someapp")
(check "follow rule switched to group III" (current-group-name) " III ")
(check "followed window is in group III" (group-has-window? " III " 5) #t)
(check "followed window placed frame-filling"
       (hash-ref %placements 5)
       (list 3 3 1274 714))
(check "followed window focused" %focused 5)

;; status-line reflects the group list and current window title.
(check-true "status-line shows bracketed current group"
            (string-contains (status-line) "[III]"))
(check-true "status-line shows the window title"
            (string-contains (status-line) "follow-me notes"))

;; Unmatched windows still go through the default path.
(wm-on-window-map 6 "plain" "plainapp")
(check "unmatched window mapped into the current frame" (current-frame-window) 6)

(clear-placement-rules!)

;; ---------------------------------------------------------------------
;; (g) gother! toggles between the last two groups.
;; ---------------------------------------------------------------------

;; We're in III (followed window 5 there). The previous group was I.
(check "still in group III" (current-group-name) " III ")
(gother!)
(check "gother! goes back to the previous group (I)" (current-group-name) " I ")
(gother!)
(check "gother! toggles forward again (III)" (current-group-name) " III ")

;; ---------------------------------------------------------------------
;; (h) Window numbers stay unique within a group across moves.
;; ---------------------------------------------------------------------

;; III currently holds windows 5 and 6.
(check-true "numbers in III are distinct"
            (not (equal? (window-number 5) (window-number 6))))
;; Move 6 (current) into the next group and make sure its number doesn't
;; collide there.
(move-window-to-next-group!)
(let* ((moved 6)
       (n (window-number moved)))
  (check-true "moved window still has a number" n))

(check-true "echo-windows-string marks the current window"
            (string-contains (echo-windows-string) "*"))

;; ---------------------------------------------------------------------
;; (i) grename! / gkill! / gmove-and-follow! / focus-group hook.
;; ---------------------------------------------------------------------

(use-modules (minde hooks))
(define %focused-groups '())
(add-hook!* 'focus-group (lambda (name) (set! %focused-groups (cons name %focused-groups))))

(grename! "Work")
(check "grename! renamed the current group" (current-group-name) " Work ")

;; gmove-and-follow!: current window travels and we switch with it.
(let ((id (current-frame-window)))
  (check-true "a window is current before gmove-and-follow!" id)
  (gmove-and-follow!)
  (check-true "gmove-and-follow! switched groups" (not (string=? (current-group-name) " Work ")))
  (check "the window came along and is focused" (current-frame-window) id)
  (check-true "focus-group hook fired" (pair? %focused-groups)))

;; gkill!: the current group dies, its windows land in the next group.
(let ((doomed (current-group-name))
      (id (current-frame-window))
      (n-before (length (group-names))))
  (gkill!)
  (check "gkill! removed a group" (length (group-names)) (- n-before 1))
  (check-true "killed group is gone" (not (member doomed (group-names))))
  (check-true "its window survived into another group"
              (find (lambda (g) (group-has-window? g id)) (group-names))))

;; ---------------------------------------------------------------------
;; Sprint 8: gnewbg / switching gnew, *-with-window, gmerge, gkill-other,
;; gmove-marked, kill-windows, always-show, groups echo
;; ---------------------------------------------------------------------

;; Clean slate: collapse everything into one group named A.
(gkill-other!)
(grename! "A")
(check "gkill-other! left a single group" (length (group-names)) 1)

(gnewbg! " B ")
(check "gnewbg! does not switch" (current-group-name) " A ")
(check-true "gnewbg! created B" (and (member " B " (group-names)) #t))

(gnew! " C ")
(check "gnew! switches to the new group" (current-group-name) " C ")

(let ((g (gnewbg-float! " F ")))
  (check-true "gnewbg-float! creates a float group" (group-float? g))
  (check "gnewbg-float! does not switch" (current-group-name) " C "))

;; gnext/gprev-with-window!: the window travels and stays focused.
(wm-on-window-map 71 "traveler" "foot")
(gnext-with-window!)
(check-true "gnext-with-window! left C" (not (string=? (current-group-name) " C ")))
(check-true "the window came along" (group-has-window? (current-group-name) 71))
(check "and is focused" (focused-window-id) 71)
(gprev-with-window!)
(check "gprev-with-window! went back to C" (current-group-name) " C ")
(check-true "the window came back too" (group-has-window? " C " 71))

;; gmerge!: B's window moves here, B dies.
(switch-to-group! " B ")
(wm-on-window-map 72 "b-window" "foot")
(switch-to-group! " C ")
(let ((n (length (group-names))))
  (gmerge! " B ")
  (check "gmerge! deleted the source group" (length (group-names)) (- n 1))
  (check-true "its window landed here" (group-has-window? " C " 72)))

;; gmove-marked-to!: both windows marked, moved to F, marks cleared.
(focus-window-by-id! 71)
(mark-window-toggle!)
(focus-window-by-id! 72)
(mark-window-toggle!)
(gmove-marked-to! " F ")
(check-true "71 moved to F" (group-has-window? " F " 71))
(check-true "72 moved to F" (group-has-window? " F " 72))
(check "marks cleared by the move" (marked-windows) '())

;; kill-windows: other groups' windows close, the current group's don't
;; -- and vice versa.
(wm-on-window-map 73 "local" "foot")
(set! %closed '())
(kill-windows-other!)
(check-true "kill-windows-other! closed 71" (and (member 71 %closed) #t))
(check-true "kill-windows-other! closed 72" (and (member 72 %closed) #t))
(check-true "kill-windows-other! spared the current group"
            (not (member 73 %closed)))
(set! %closed '())
(kill-windows-current-group!)
(check-true "kill-windows-current-group! closed 73" (and (member 73 %closed) #t))

;; Always-show: the sticky window follows every switch until toggled off.
(focus-window-by-id! 73)
(toggle-always-show!)
(switch-to-group! " A ")
(check-true "sticky window followed the switch" (group-has-window? " A " 73))
(check-true "and left its old group" (not (group-has-window? " C " 73)))
(switch-to-group! " C ")
(check-true "it follows back" (group-has-window? " C " 73))
(focus-window-by-id! 73)
(toggle-always-show!)
(switch-to-group! " A ")
(check-true "after toggling off it stays put" (group-has-window? " C " 73))

;; groups echo: current group marked *, one line per group with count.
(check-true "groups-echo-string marks the current group"
            (and (string-contains (groups-echo-string) "*A") #t))

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
