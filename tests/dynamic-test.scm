;;; dynamic-test.scm -- Guile-only unit test of dynamic (auto-tiling)
;;; groups in (minde groups).
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
(use-modules (minde compositor frames))
(use-modules (minde groups) (minde compositor model))

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

(update-output-geometry! 0 0 1280 720)

;; ---------------------------------------------------------------------
;; create-dynamic-group!: creates, switches, tiles as windows arrive.
;; ---------------------------------------------------------------------

(create-dynamic-group! " dyn ")
(check "create-dynamic-group! switched" (current-group-name) " dyn ")
(check-true "group is dynamic" (dynamic-group?))

(handle-window-map! 1 "one" "foot")
(check "one window fills the head (minus border inset)"
       (length (frame-leaves (current-tree))) 1)
(check-true "window 1 visible" (visible? 1))

(handle-window-map! 2 "two" "foot")
(check "two frames after second map"
       (length (frame-leaves (current-tree))) 2)
;; Master = newest (2), at 2/3 of 1280 on the left.
(let ((m (rect 2)) (s (rect 1)))
  (check-true "master starts at the left edge region" (< (car m) 20))
  (check-true "master is ~2/3 wide" (and (> (caddr m) 800) (< (caddr m) 880)))
  (check-true "stack window sits right of the master" (> (car s) 800))
  (check-true "stack window is full height" (> (cadddr s) 650)))
(check "focus went to the new master" %focused 2)

(handle-window-map! 3 "three" "foot")
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

(focus-window-by-id! 1)
(switch-to-group! " I ")
(switch-to-group! " dyn ")
(check "returning to a dynamic group restores stack focus" (list (focused-window-id) %focused) '(1 1))
(for-each
 (lambda (layout)
   (change-layout! layout)
   (retile-dynamic!)
   (check "layout changes and retiles preserve stack focus" (list (focused-window-id) %focused) '(1 1)))
 '(right bottom top left))
(handle-window-unmap! 3)
(check "back to two frames" (length (frame-leaves (current-tree))) 2)
(check-true "window 2 promoted to master" (< (car (rect 2)) 20))
(check-true "window 1 still stacks" (> (car (rect 1)) 800))
(check "removing another window preserves surviving stack focus" (list (focused-window-id) %focused) '(1 1))

;; ---------------------------------------------------------------------
;; rotate-windows! / rotate-stack! / exchange-with-master!
;; ---------------------------------------------------------------------

(handle-window-map! 4 "four" "foot") ; order now (4 2 1)
(check-true "4 is master" (< (car (rect 4)) 20))
(check "newly mapped master still receives focus" (list (focused-window-id) %focused) '(4 4))
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

(create-group-in-background! " manual ")
(switch-to-group! " manual ")
(handle-window-map! 5 "five" "foot")
(switch-to-group! " dyn ")
(merge-group-into-current! " manual ") ; pulls 5 into the dynamic group
(check "gmerge into dynamic retiled: 4 frames"
       (length (frame-leaves (current-tree))) 4)

(focus-window-by-id! 5)
(float-window! 5)
(retile-dynamic!)
(check "float excluded from the tiling: 3 frames"
       (length (frame-leaves (current-tree))) 3)
(focus-window-by-id! 5)
(let ((before (dump-frames))
      (tree (current-tree))
      (order ((@@ (minde groups) dynamic-state) (current-group) (current-head-id)))
      (placements (map rect '(1 2 4 5))))
  (exchange-with-master!)
  (check "exchange with a float leaves the frame dump unchanged" (dump-frames) before)
  (check-true "exchange with a float does not rebuild the tree" (eq? tree (current-tree)))
  (check "exchange with a float preserves the dynamic order"
         ((@@ (minde groups) dynamic-state) (current-group) (current-head-id)) order)
  (check "exchange with a float keeps keyboard focus" %focused 5)
  (check "exchange with a float preserves placements" (map rect '(1 2 4 5)) placements))
(unfloat-window! 5)
(retile-dynamic!)
(check "unfloat re-enters the tiling: 4 frames"
       (length (frame-leaves (current-tree))) 4)

;; ---------------------------------------------------------------------
;; create-dynamic-group-in-background! stays in the background; switching retiles it.
;; ---------------------------------------------------------------------

(create-dynamic-group-in-background! " dyn2 ")
(check "create-dynamic-group-in-background! did not switch" (current-group-name) " dyn ")
(switch-to-group! " dyn2 ")
(check-true "dyn2 is dynamic" (dynamic-group?))

;; ---------------------------------------------------------------------
;; retile-dynamic-group! / rotate on a manual group refuses politely.
;; ---------------------------------------------------------------------

(switch-to-group! " I ")
(set! %messages '())
(retile-dynamic-group!)
(check-true "retile-dynamic-group! refuses in a manual group"
            (and (pair? %messages)
                 (string-contains (car %messages) "not a dynamic group")))
(rotate-windows! 'forward)
(check-true "rotate refuses in a manual group"
            (string-contains (car %messages) "not a dynamic group"))

;; Rule-driven changes must repair the source tiling immediately.
(create-dynamic-group! "rule-dynamic")
(for-each (lambda (id) (handle-window-map! id "rule candidate" "")) '(201 202 203))
(add-placement-rule! #:title "dynamic-PiP" #:float? #t)
(handle-window-title-change! 202 "dynamic-PiP" "")
(check "title float removes the empty dynamic leaf" (length (frame-leaves (current-tree))) 2)
(check "title float fills the remaining stack" (rect 201) '(856 3 421 714))
(check "retiling preserves the new float's input focus" %focused 202)
(create-group-in-background! "rule-destination")
(add-placement-rule! "late-dynamic" #:group "rule-destination")
(handle-window-map! 204 "late" "")
(handle-window-title-change! 204 "late" "late-dynamic")
(check "late placement repairs its dynamic source" (length (frame-leaves (current-tree))) 2)
(add-placement-rule! #:title "move-existing" #:group "rule-destination" #:lock? #f)
(handle-window-title-change! 203 "move-existing" "")
(place-existing-windows!)
(check "explicit placement repairs the dynamic source" (rect 201) '(3 3 1274 714))
(add-placement-rule! #:title "float-existing" #:float? #t #:lock? #f)
(handle-window-title-change! 201 "float-existing" "")
(place-existing-windows!)
(check-true "explicit float rule removes its window from tiling" (window-floating? 201))
(check "all-float dynamic group has no tiled residents" (frame-tree-window-count) 0)
(clear-placement-rules!)

;; Closing on another head (including in a hidden group) must rebuild
;; that head without borrowing focus from the current window/head.
(handle-heads-change! '((10 0 0 1920 1080) (11 1920 0 1280 1024)))
(define multi (create-dynamic-group! "multi-dynamic"))
(handle-window-map! 301 "head ten" "test")
(focus-next-head!)
(for-each (lambda (id) (handle-window-map! id "head eleven" "test")) '(302 303 304))
(focus-last-head!)
(handle-window-unmap! 303)
(check "unmap repairs the unfocused head immediately" (rect 302) '(2776 3 421 1018))
(check "other-head unmap keeps current head and keyboard focus"
       (list (current-head-id) (focused-window-id) %focused) '(10 301 301))
(check "unfocused head has no empty stack leaf"
       (length (frame-leaves (car (hash-ref (group-heads multi) 11)))) 2)
(switch-to-group! " I ")
(define before-hidden-unmap %focused)
(handle-window-unmap! 304)
(check "hidden group's other head is retiled"
       (length (frame-leaves (car (hash-ref (group-heads multi) 11)))) 1)
(check "hidden unmap leaves visible input focus alone" %focused before-hidden-unmap)
(switch-to-group! "multi-dynamic")
(focus-next-head!)
(check "returning shows the surviving window across its whole head" (rect 302) '(1923 3 1274 1018))
(for-each (lambda (id) (handle-window-map! id "hidden loaded head" "test")) '(305 306))
(switch-to-group! " I ")
(focus-previous-head!)
(handle-window-unmap! 305)
(check "hidden loaded head is rebuilt without changing the active head" (current-head-id) 10)
(check "hidden loaded head retains its own dimensions"
       (map (lambda (f) (list (frame-y f) (frame-h f))) (frame-leaves (group-tree multi)))
       '((0 1024) (0 1024)))
(switch-to-group! "multi-dynamic")
(focus-next-head!)
(check "hidden loaded-head repair is visible on return" (rect 302) '(2776 3 421 1018))

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
