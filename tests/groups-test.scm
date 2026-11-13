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
(define %placement-history '())
(define %fullscreen-flags (make-hash-table))
(define (wm-window-geometry id) (hash-ref %placements id))
(define (wm-set-fullscreen id on)
  (hash-set! %fullscreen-flags id on)
  (when on (hash-set! %placements id '(0 0 1280 720)))
  #t)
(define (wm-place-float id x y w h) (wm-place-window id x y w h))
(define (wm-set-floating id on) #t)
(define (wm-raise-window id) #t)
(define %focused #f)
(define %focus-cleared 0)
(define %closed '())
(define %log-lines '())

(define (wm-place-window id x y w h)
  (set! %placement-history (cons (list id x y w h) %placement-history))
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
(use-modules (minde compositor frames))
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

(update-output-geometry! 0 0 1280 720)

(check "default groups are I, II, III" (group-names) (list " I " " II " " III "))
(check "current group starts as I" (current-group-name) " I ")

;; ---------------------------------------------------------------------
;; (a) Map two windows in group I, switch to II -> both parked off-screen,
;; focus cleared.
;; ---------------------------------------------------------------------

(handle-window-map! 1 "term" "foot")
(handle-window-map! 2 "editor" "emacs")

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
(handle-window-unmap! 2)
(check "window 2 removed from group II after unmap while hidden" (group-has-window? " II " 2) #f)
(check "window 1 (in the current, active group) unaffected" (group-has-window? " I " 1) #t)

;; ---------------------------------------------------------------------
;; (e) pull-window-from-other-frame! moves a window from the other frame
;; after a split.
;; ---------------------------------------------------------------------

;; Group I currently: single frame with window 1 current. Map a second
;; window (3), then split so 1 and 3 end up in different frames.
(handle-window-map! 3 "browser" "zen")
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
(handle-window-map! 4 "some page" "zen-two")
(check "rule-matched window landed in group II" (group-has-window? " II " 4) #t)
(check "rule-matched window not in group I" (group-has-window? " I " 4) #f)
(check "current group unchanged by non-follow rule" (current-group-name) " I ")
(check-true "rule-matched window parked off-screen"
            (offscreen? (hash-ref %placements 4)))

;; By title, with #:follow?: switches to the target group and shows it.
(add-placement-rule! "follow-me" #:group "III" #:follow? #t)
(handle-window-map! 5 "follow-me notes" "someapp")
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
(handle-window-map! 6 "plain" "plainapp")
(check "unmatched window mapped into the current frame" (current-frame-window) 6)

(clear-placement-rules!)

;; ---------------------------------------------------------------------
;; (g) switch-to-last-group! toggles between the last two groups.
;; ---------------------------------------------------------------------

;; We're in III (followed window 5 there). The previous group was I.
(check "still in group III" (current-group-name) " III ")
(switch-to-last-group!)
(check "switch-to-last-group! goes back to the previous group (I)" (current-group-name) " I ")
(switch-to-last-group!)
(check "switch-to-last-group! toggles forward again (III)" (current-group-name) " III ")

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
;; (i) rename-current-group! / delete-current-group! / move-current-window-to-next-group-and-follow! / focus-group hook.
;; ---------------------------------------------------------------------

(use-modules (minde hooks))
(define %focused-groups '())
(add-event-hook! 'focus-group (lambda (name) (set! %focused-groups (cons name %focused-groups))))

(rename-current-group! "Work")
(check "rename-current-group! renamed the current group" (current-group-name) " Work ")

;; move-current-window-to-next-group-and-follow!: current window travels and we switch with it.
(let ((id (current-frame-window)))
  (check-true "a window is current before move-current-window-to-next-group-and-follow!" id)
  (move-current-window-to-next-group-and-follow!)
  (check-true "move-current-window-to-next-group-and-follow! switched groups" (not (string=? (current-group-name) " Work ")))
  (check "the window came along and is focused" (current-frame-window) id)
  (check-true "focus-group hook fired" (pair? %focused-groups)))

;; delete-current-group!: the current group dies, its windows land in the next group.
(let ((doomed (current-group-name))
      (id (current-frame-window))
      (n-before (length (group-names))))
  (delete-current-group!)
  (check "delete-current-group! removed a group" (length (group-names)) (- n-before 1))
  (check-true "killed group is gone" (not (member doomed (group-names))))
  (check-true "its window survived into another group"
              (find (lambda (g) (group-has-window? g id)) (group-names))))

;; ---------------------------------------------------------------------
;; gnewbg / switching gnew, *-with-window, gmerge, gkill-other,
;; gmove-marked, kill-windows, always-show, groups echo
;; ---------------------------------------------------------------------

;; Clean slate: collapse everything into one group named A.
(delete-other-groups!)
(rename-current-group! "A")
(check "delete-other-groups! left a single group" (length (group-names)) 1)

(create-group-in-background! " B ")
(check "create-group-in-background! does not switch" (current-group-name) " A ")
(check-true "create-group-in-background! created B" (and (member " B " (group-names)) #t))

(create-group! " C ")
(check "create-group! switches to the new group" (current-group-name) " C ")

(let ((g (create-floating-group-in-background! " F ")))
  (check-true "create-floating-group-in-background! creates a float group" (group-float? g))
  (check "create-floating-group-in-background! does not switch" (current-group-name) " C "))

;; gnext/shift-current-window-to-previous-group!: the window travels and stays focused.
(handle-window-map! 71 "traveler" "foot")
(shift-current-window-to-next-group!)
(check-true "shift-current-window-to-next-group! left C" (not (string=? (current-group-name) " C ")))
(check-true "the window came along" (group-has-window? (current-group-name) 71))
(check "and is focused" (focused-window-id) 71)
(shift-current-window-to-previous-group!)
(check "shift-current-window-to-previous-group! went back to C" (current-group-name) " C ")
(check-true "the window came back too" (group-has-window? " C " 71))

;; merge-group-into-current!: B's window moves here, B dies.
(switch-to-group! " B ")
(handle-window-map! 72 "b-window" "foot")
(switch-to-group! " C ")
(let ((n (length (group-names))))
  (merge-group-into-current! " B ")
  (check "merge-group-into-current! deleted the source group" (length (group-names)) (- n 1))
  (check-true "its window landed here" (group-has-window? " C " 72)))

;; move-marked-windows-to-group!: both windows marked, moved to F, marks cleared.
(focus-window-by-id! 71)
(mark-window-toggle!)
(focus-window-by-id! 72)
(mark-window-toggle!)
(move-marked-windows-to-group! " F ")
(check-true "71 moved to F" (group-has-window? " F " 71))
(check-true "72 moved to F" (group-has-window? " F " 72))
(check "marks cleared by the move" (marked-windows) '())

;; kill-windows: other groups' windows close, the current group's don't
;; -- and vice versa.
(handle-window-map! 73 "local" "foot")
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

;; handle-window-title-change!: a window that mapped with empty title/app-id gets
;; its lock rule applied when the app-id finally arrives.
(add-placement-rule! "latecomer" #:group " F " #:frame 0)
(handle-window-map! 80 "" "")
(check-true "empty-id window stayed in the current group"
            (not (group-has-window? " F " 80)))
(handle-window-title-change! 80 "some page" "latecomer")
(check "late app-id recorded" (window-app-id 80) "latecomer")
(check-true "lock rule applied on late app-id"
            (group-has-window? " F " 80))
;; A later retitle must not re-place or duplicate the window.
(handle-window-title-change! 80 "another page" "latecomer")
(check-true "retitle keeps it in the rule group"
            (group-has-window? " F " 80))

;; ---------------------------------------------------------------------
;; The foreign-toplevel activate hook (defined in init.scm) is
;; thin glue over these exported group operations. Exercise the exact
;; building blocks it uses -- find the group holding an id, switch to it,
;; focus the window -- across freshly-created groups.
;; ---------------------------------------------------------------------

(create-group! " FTA ")
(switch-to-group! " FTA ")
(handle-window-map! 200 "one" "app-one")
(create-group! " FTB ")
(switch-to-group! " FTB ")
(handle-window-map! 201 "two" "app-two")

(handle-foreign-activate! 200)
(check "foreign-activate reaches the window's group"
       (current-group-name) " FTA ")
(check "foreign-activate focuses the requested window" %focused 200)

;; A vanished window is a no-op (find returns #f; focus untouched).
(set! %focused 'unchanged)
(handle-foreign-activate! 999999)
(check "foreign-activate on a missing window does nothing"
       %focused 'unchanged)

;; ---------------------------------------------------------------------

(check "empty window set has zero distinct IDs"
       ((@@ (minde groups) distinct-window-count) '()) 0)
(check "mirrored and repeated IDs are counted once"
       ((@@ (minde groups) distinct-window-count) '(1 2 3 2 1 3 3)) 3)
(check "large numeric window IDs retain equality semantics"
       ((@@ (minde groups) distinct-window-count)
        (list (expt 2 80) (+ (expt 2 80) 0) 7)) 2)

;; Fullscreen is group-local: other groups keep tiling/focus, and coming
;; back restores fullscreen without an intermediate tiled-size configure.
(create-group! " full-a ")
(handle-window-map! 300 "fullscreen-a" "foot")
(fullscreen!)
(check "group A owns fullscreen" (fullscreen-window) 300)
(check "fullscreen geometry recorded by Rust" (hash-ref %placements 300) '(0 0 1280 720))
(create-group! " full-b ")
(check "new group has no fullscreen window" (fullscreen-window) #f)
(check "switching from fullscreen to empty group clears focus" %focused #f)
(check "hidden fullscreen window keeps its full size"
       (cddr (hash-ref %placements 300)) '(1280 720))
(handle-window-map! 301 "fullscreen-b" "foot")
(check "new group's client fills its frame" (hash-ref %placements 301) '(3 3 1274 714))
(check "new group's client receives focus" %focused 301)
(fullscreen!)
(check "group B can have its own fullscreen" (fullscreen-window) 301)
(set! %placement-history '())
(switch-to-group! " full-a ")
(check "returning restores A's fullscreen id" (fullscreen-window) 300)
(check "returning restores A's fullscreen geometry" (hash-ref %placements 300) '(0 0 1280 720))
(check "returning restores A's focus" %focused 300)
(check-true "A never received a tiled-size configure on return"
            (every (lambda (p) (or (not (= (car p) 300))
                                  (equal? (cdddr p) '(1280 720))))
                   %placement-history))
(switch-to-group! " full-b ")
(check "returning restores B's independent fullscreen" (fullscreen-window) 301)
(fullscreen!)
(check "exiting B's fullscreen restores its tiled rectangle" (hash-ref %placements 301) '(3 3 1274 714))
(handle-window-unmap! 300)
(check "closing hidden fullscreen does not change B's focus" %focused 301)
(switch-to-group! " full-a ")
(check "closing a hidden fullscreen window clears its group's state" (fullscreen-window) #f)
(check "empty former fullscreen group clears focus" %focused #f)
(handle-window-map! 302 "move-fullscreen" "foot")
(fullscreen!)
(move-current-window-to-next-group-and-follow!)
(check "moving fullscreen to another group clears old ownership" (fullscreen-window) #f)
(check "moving fullscreen clears the protocol state" (hash-ref %fullscreen-flags 302) #f)
(check "moved fullscreen window tiles in its destination" (hash-ref %placements 302) '(3 3 1274 714))
(switch-to-group! " full-a ")
(check "old group remains usable after moving fullscreen away" (fullscreen-window) #f)
(handle-window-map! 303 "replacement" "foot")
(check "old group places new windows after fullscreen was moved" (hash-ref %placements 303) '(3 3 1274 714))

;; Floating fullscreen windows likewise retain their full buffer while
;; parked, then recover their original float rectangle when fullscreen ends.
(float-window! 303)
(define saved-float (float-geometry 303))
(fullscreen!)
(switch-to-group! " full-b ")
(check "parked floating fullscreen keeps its full size"
       (cddr (hash-ref %placements 303)) '(1280 720))
(switch-to-group! " full-a ")
(check "floating fullscreen restores focus" %focused 303)
(check "floating fullscreen restores full geometry" (hash-ref %placements 303) '(0 0 1280 720))
(fullscreen!)
(check "fullscreen exit restores the original floating rectangle"
       (hash-ref %placements 303) saved-float)

;; Application requests change their owner's group without activating it.
(create-group! " request-anchor ")
(handle-window-map! 310 "anchor" "foot")
(create-group! " request-target ")
(handle-window-map! 311 "requester" "foot")
(handle-window-map! 312 "peer" "foot")
(switch-to-group! " request-anchor ")
(for-each
 (lambda (on)
   (check "hidden request resolves" (handle-window-fullscreen! 311 on) #t)
   (check "hidden request preserves group" (current-group-name) " request-anchor ")
   (check "hidden request preserves keyboard focus" %focused 310)
   (check-true "hidden requester stays parked" (offscreen? (hash-ref %placements 311))))
 '(#t #f #t))
(switch-to-group! " request-target ")
(check "visiting the requested group restores fullscreen" (fullscreen-window) 311)
(check "visiting selects the fullscreen owner" %focused 311)
(check "visiting applies fullscreen geometry" (hash-ref %placements 311) '(0 0 1280 720))
(handle-window-fullscreen! 311 #f)
(focus-window-by-id! 312)
(for-each
 (lambda (_)
   (handle-window-fullscreen! 311 #t)
   (check "same-group request uses requesting ID" (fullscreen-window) 311)
   (check "same-group request preserves logical focus" (focused-window-id) 312)
   (check "same-group request preserves keyboard focus" %focused 312)
   (sync-frames!)
   (check "sync preserves the original keyboard focus" %focused 312))
 '(first repeated))
(handle-window-fullscreen! 312 #f)
(check "a peer cannot cancel someone else's fullscreen" (fullscreen-window) 311)
(for-each
 (lambda (on)
   (check "unknown fullscreen ID rejected" (handle-window-fullscreen! 999999 on) #f)
   (check "unknown ID preserves fullscreen owner" (fullscreen-window) 311)
   (check "unknown ID preserves keyboard focus" %focused 312))
 '(#t #f))

;; A stale float mirror may still occur in a frame, but cannot be selected.
(handle-window-map! 313 "stale float" "foot")
(hash-set! (@@ (minde compositor frames) %floating) 313 '(0 0 100 100))
(switch-to-group! " request-anchor ")
(check "stale float request rejected" (handle-window-fullscreen! 313 #t) #f)
(check "stale taskbar activation rejected" (handle-foreign-activate! 313) #f)
(check "stale requests preserve active group" (current-group-name) " request-anchor ")
(check "stale requests preserve keyboard focus" %focused 310)
(hash-remove! (@@ (minde compositor frames) %floating) 313)
(handle-window-unmap! 313)
(switch-to-group! " request-target ")
(check "revisiting preserves separate fullscreen focus" %focused 312)
(handle-window-unmap! 312)
(check "vanished focus falls back to surviving fullscreen owner" %focused 311)
(handle-window-map! 314 "replacement owner" "foot")
(check "mapping behind fullscreen preserves its focus" %focused 311)
(handle-window-fullscreen! 311 #f)
(check "fullscreen exit preserves focus after a new mapping" %focused 311)
(handle-window-fullscreen! 311 #t)
(handle-window-fullscreen! 314 #t)
(check "request replaces the fullscreen owner" (fullscreen-window) 314)
(check "replacement clears the previous protocol state" (hash-ref %fullscreen-flags 311) #f)
(check "replacement preserves the keyboard target" %focused 311)
(handle-foreign-activate! 314)
(check "explicit activation selects an unfocused fullscreen owner" %focused 314)
(handle-foreign-activate! 311)
(check "explicit activation clears another window's fullscreen" (fullscreen-window) #f)
(check "explicit activation selects the requested peer" %focused 311)
(handle-window-fullscreen! 314 #t)
(handle-window-unmap! 314)
(check "unmapping fullscreen clears ownership" (fullscreen-window) #f)
(check "unmapping fullscreen preserves the peer's focus" %focused 311)
(handle-window-fullscreen! 311 #t)
(switch-to-group! " request-anchor ")
(handle-window-fullscreen! 311 #f)
(check "hidden fullscreen exit preserves focus" %focused 310)
(check-true "hidden fullscreen exit stays parked" (offscreen? (hash-ref %placements 311)))
(switch-to-group! " request-target ")
(check "cancelled hidden request stays cancelled on return" (fullscreen-window) #f)
(clear-current-frame!)
(handle-window-fullscreen! 311 #t)
(check "fullscreen request preserves an empty keyboard target" %focused #f)
(handle-window-map! 315 "new unfocused float" "foot")
(float-window! 315)
(check "mapping behind unfocused fullscreen keeps no keyboard target" %focused #f)
(handle-window-fullscreen! 311 #f)
(check "fullscreen exit preserves an empty keyboard target" %focused #f)
(check "fullscreen exit preserves empty logical focus" (focused-window-id) #f)
(handle-window-unmap! 315)
(handle-window-unmap! 311)
(handle-window-unmap! 310)

;; Names are presentation and lookup keys; internal group transitions must
;; retain record identity even for legacy duplicate names.
(use-modules (minde compositor model))
(define (group-list) (@@ (minde groups) %groups))
(define (goto-group g)
  (switch-to-group! (list-index (lambda (candidate) (eq? candidate g)) (group-list))))
(define (raises? thunk)
  (catch #t (lambda () (thunk) #f) (lambda args #t)))
(define named (create-group! " name-check "))
(define original-count (length (group-list)))
(for-each
 (lambda (constructor)
   (check-true "group constructors reject names differing only in padding"
               (raises? (lambda () (constructor "name-check"))))
   (check "rejected creation leaves the group list unchanged" (length (group-list)) original-count)
   (check-true "rejected creation preserves the active group" (eq? named (current-group)))
   (goto-group named))
 (list create-group-in-background! create-group!
       create-floating-group-in-background! create-floating-group!
       create-dynamic-group-in-background! create-dynamic-group!))
(check-true "exact duplicate names are also rejected"
            (raises? (lambda () (create-group! " name-check "))))
(goto-group named)
(check-true "renaming to another group's name is rejected"
            (raises? (lambda () (rename-current-group! "full-a"))))
(check "rejected rename keeps the old name" (group-name named) " name-check ")
(rename-current-group! "name-check")
(check "renaming to one's own normalized name is allowed" (group-name named) " name-check ")
(rename-current-group! "  ")
(check "blank rename leaves the name intact" (group-name named) " name-check ")
(for-each (lambda (name)
            (check-true "blank creation is rejected"
                        (raises? (lambda () (create-group-in-background! name)))))
          '("" "  "))
(goto-group named)
(delete-other-groups!)
(define legacy-a (create-group! "legacy-a"))
(handle-window-map! 400 "legacy a" "test")
(define legacy-b (create-group! "legacy-b"))
(handle-window-map! 401 "legacy b" "test")
(define legacy-c (create-group! "legacy-c"))
(for-each (lambda (g) (set-group-name! g " legacy ")) (list legacy-a legacy-b legacy-c))
(switch-to-group! legacy-b)
(check-true "switch accepts a live group record" (eq? legacy-b (current-group)))
(goto-group legacy-b)
(goto-group legacy-a)
(switch-to-last-group!)
(check-true "last-group selects the previous record despite duplicate names" (eq? legacy-b (current-group)))
(goto-group legacy-a)
(add-urgent-window! 401)
(next-urgent!)
(check-true "urgent focus finds the owning record despite duplicate names" (eq? legacy-b (current-group)))
(check "urgent focus reaches the actual window" %focused 401)
(goto-group legacy-b)
(shift-current-window-to-next-group!)
(check-true "shift follows the intended next group record" (eq? legacy-c (current-group)))
(check "shift preserves actual window focus" %focused 401)
(goto-group legacy-c)
(shift-current-window-to-previous-group!)
(check-true "reverse shift follows the intended previous group" (eq? legacy-b (current-group)))
(goto-group legacy-a)
(delete-current-group!)
(check-true "deletion activates the next record with an identical name" (eq? legacy-b (current-group)))
(check-true "active group remains a member after deletion" (memq (current-group) (group-list)))
(check-true "fallback shows the deleted group's window" (member 400 (all-window-ids)))
(switch-to-group! 0)
(goto-group legacy-c)
(handle-window-map! 402 "last group" "test")
(delete-current-group!)
(check-true "deleting the last group wraps to the first" (eq? (car (group-list)) (current-group)))
(check-true "wrap fallback adopts the last group's window" (member 402 (all-window-ids)))
(let ((before (current-group)))
  (switch-to-group! legacy-c)
  (check-true "deleted group records cannot become active" (eq? before (current-group))))

;; Auto names fill holes and never reuse an existing numeral, including
;; an unpadded name supplied explicitly by the user.
(delete-other-groups!)
(rename-current-group! "I")
(define auto-two (create-auto-named-group!))
(create-auto-named-group!)
(create-auto-named-group!)
(goto-group auto-two)
(delete-current-group!)
(create-auto-named-group!)
(check "automatic names reuse the first free numeral" (current-group-name) " II ")
(create-group-in-background! "V")
(create-auto-named-group!)
(check "automatic names skip unpadded existing numerals" (current-group-name) " VI ")
(check "automatic names remain unique after deletion"
       (length (delete-duplicates (map string-trim-both (group-names)))) (length (group-names)))

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
