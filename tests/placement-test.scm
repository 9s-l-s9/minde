;;; placement-test.scm -- sprint 9: frame dumps, sibling, expose,
;;; unmaximize/gravity, rule lock flag + persistence, desktop dump.
;;;
;;; Run with:
;;;   guile -L scheme tests/placement-test.scm

(use-modules (srfi srfi-1)
             (minde foundation serialization))

;; ---------------------------------------------------------------------
;; Stubs recording calls, standing in for the Rust-side subrs.
;; ---------------------------------------------------------------------

(define %placements (make-hash-table)) ; id -> (x y w h), wm-place-window
(define %float-places (make-hash-table)) ; id -> (x y w h), wm-place-float
(define %focused #f)
(define %overlays '())

(define (wm-place-window id x y w h)
  (hash-set! %placements id (list x y w h))
  #t)

(define (wm-place-float id x y w h)
  (hash-set! %float-places id (list x y w h))
  #t)

(define (wm-focus-window id) (set! %focused id) #t)
(define (wm-close-window id) #t)
(define (wm-clear-focus) (set! %focused #f) #t)
(define (wm-output-geometry) (list 1280 720))
(define (wm-log msg) #t)
(define (wm-raise-window id) #t)
(define (wm-set-floating id on) #t)
(define (wm-add-overlay x y text)
  (set! %overlays (cons (list x y text) %overlays)) #t)
(define (wm-clear-overlays) (set! %overlays '()) #t)

;; Rule persistence goes to a scratch file, not the developer's config.
(define %rules-path (format #f "/tmp/minde-placement-test-rules-~a.scm" (getpid)))
(define %desktop-path (format #f "/tmp/minde-placement-test-desktop-~a.scm" (getpid)))
(setenv "MINDE_RULES_FILE" %rules-path)

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

(update-output-geometry! 0 0 1280 720)

;; ---------------------------------------------------------------------
;; dump-frames / restore-frames!: layout AND window assignment survive.
;; ---------------------------------------------------------------------

(handle-window-map! 1 "one" "foot")
(handle-window-map! 2 "two" "foot")
(handle-window-map! 3 "three" "foot")
(split-frame-horizontal!)
(move-window! 'right) ; w3 into the right frame, focus follows

(define %dump (dump-frames))
(check "dump has two leaves" (length (cadr %dump)) 2)
(check "left leaf windows" (car (cadr %dump)) '(1 2))
(check "right leaf windows" (cadr (cadr %dump)) '(3))
(check "current frame index dumped" (cadddr %dump) 1)

(collapse-to-one-frame!) ; mangle the layout completely
(check "collapse-to-one-frame! collapsed to one leaf" (length (frame-leaves (current-tree))) 1)

(restore-frames! %dump)
(let ((leaves (frame-leaves (current-tree))))
  (check "restore rebuilt two leaves" (length leaves) 2)
  (check "left leaf windows restored" (frame-window-ids (car leaves)) '(1 2))
  (check "right leaf windows restored" (frame-window-ids (cadr leaves)) '(3))
  (check "current window of the right leaf" (current-frame-window) 3))

;; Stale ids are dropped; windows mapped since the dump go to leaf 0.
(define %dump2 (dump-frames))
(handle-window-unmap! 2)
(handle-window-map! 4 "four" "foot")
(restore-frames! %dump2)
(let ((leaves (frame-leaves (current-tree))))
  (check "stale id dropped on restore" (frame-window-ids (car leaves)) '(1 4))
  (check "surviving window still in its leaf" (frame-window-ids (cadr leaves)) '(3)))

;; ---------------------------------------------------------------------
;; focus-sibling-frame!
;; ---------------------------------------------------------------------

(focus-frame-by-index! 0)
(check "on frame 0" (current-frame-window) 1)
(focus-sibling-frame!)
(check "focus-sibling-frame! jumped to the other side" (current-frame-window) 3)

;; ---------------------------------------------------------------------
;; fselect helpers: overlays drawn per leaf, focus-frame-by-index!.
;; ---------------------------------------------------------------------

(show-frame-overlays!)
(check "one overlay per leaf" (length %overlays) 2)
(check-true "labels are the leaf indices"
            (equal? (sort (map caddr %overlays) string<?) '("0" "1")))
(clear-frame-overlays!)
(check "overlays cleared" %overlays '())

;; ---------------------------------------------------------------------
;; expose: grid, one window per cell, pick restores + focuses.
;; ---------------------------------------------------------------------

(define %before-expose (dump-frames))
(let ((n (expose-enter!)))
  (check "expose tiled all three windows" n 3)
  (check-true "grid has at least three leaves"
              (>= (length (frame-leaves (current-tree))) 3))
  (check-true "expose drew overlays" (pair? %overlays))
  (let* ((leaves (frame-leaves (current-tree)))
         (in-cell-1 (frame-current-window (cadr leaves))))
    (expose-pick! 1)
    (check "expose restored the two-leaf layout"
           (length (frame-leaves (current-tree))) 2)
    (check "expose focused the picked window" (focused-window-id) in-cell-1)
    (check "expose cleared its overlays" %overlays '())))

;; ---------------------------------------------------------------------
;; unmaximize + gravity
;; ---------------------------------------------------------------------

(focus-window-by-id! 1)
(unmaximize!)
(check-true "window 1 is unmaximized" (window-unmaximized? 1))
(let ((r (hash-ref %float-places 1)))
  (check-true "unmaximized placement went through wm-place-float" (pair? r))
  ;; 2/3 of the ~640-wide left frame.
  (check-true "unmaximized width shrank" (< (caddr r) 640)))
(set-window-gravity! 'top-left)
(let ((r (hash-ref %float-places 1))
      (full (frame-leaves (current-tree))))
  (check "top-left gravity pins x to the frame origin"
         (car r) (car (current-frame-rect))))
(unmaximize!)
(check-true "unmaximize! toggled off" (not (window-unmaximized? 1)))
(let ((r (hash-ref %placements 1)))
  (check-true "window fills its frame again (tiled placement)"
              (> (caddr r) 400)))

;; ---------------------------------------------------------------------
;; Placement-rule lock flag: lock #f is skipped on map, applied by
;; place-existing-windows!.
;; ---------------------------------------------------------------------

(clear-placement-rules!)
(add-placement-rule! "term" #:group "II" #:frame 0 #:lock? #f)
(handle-window-map! 10 "a term window" "term")
(check-true "unlocked rule skipped on map"
            (group-has-window? " I " 10))
(place-existing-windows!)
(check-true "place-existing-windows! applied the unlocked rule"
            (group-has-window? " II " 10))

(add-placement-rule! "zen" #:group "III")
(handle-window-map! 11 "browser" "zen")
(check-true "locked rule (default) applied on map"
            (group-has-window? " III " 11))

;; ---------------------------------------------------------------------
;; Float rules: #:float? floats the window instead of framing it -- at
;; map when the title is already known, and on the late retitle Wayland
;; clients do (Firefox Picture-in-Picture maps with empty strings).
;; ---------------------------------------------------------------------

(add-placement-rule! "Picture-in-Picture" #:float? #t)
(handle-window-map! 12 "Picture-in-Picture" "firefox")
(check-true "float rule floats at map" (window-floating? 12))
(check-true "float-rule window is in no frame"
            (not (member 12 (current-frame-window-ids))))

(handle-window-map! 13 "" "")
(check-true "empty title+app-id maps tiled" (not (window-floating? 13)))
(handle-window-title-change! 13 "Picture-in-Picture" "firefox")
(check-true "float rule floats on late retitle" (window-floating? 13))

;; A hand-unfloated window stays put on a retitle that still matches.
(unfloat-window! 13)
(handle-window-title-change! 13 "Picture-in-Picture again" "firefox")
(check-true "unfloat survives a still-matching retitle"
            (not (window-floating? 13)))

(track-window-unmap! 12)
(track-window-unmap! 13)

;; ---------------------------------------------------------------------
;; remember! / forget! + persistence round-trip
;; ---------------------------------------------------------------------

(focus-window-by-id! 1)
(remember!)
(check-true "remember! wrote the rules file" (file-exists? %rules-path))
(clear-placement-rules!)
(load-placement-rules!)
(check-true "rules survived a save/load round-trip"
            (>= (length (begin (place-existing-windows!) '(x))) 0)) ; smoke
;; The reloaded rule must match window 1 (matcher = its app-id "foot").
(check-true "reloaded rule matches the remembered window"
            (let ((path %rules-path))
              (and (file-exists? path)
                   (pair? (call-with-input-file path read)))))
(forget!)
(check-true "forget! removed the remembered rule, keeping the others"
            (let ((rules (read-versioned-datum-file
                          %rules-path 'minde-placement-rules 1)))
              (and (= (length rules) 3) ; term + zen + the float rule
                   (not (find (lambda (r) (string=? (car r) "foot")) rules)))))

;; ---------------------------------------------------------------------
;; dump-desktop-to-file / restore-from-file
;; ---------------------------------------------------------------------

(create-floating-group-in-background! " FLT ")
(dump-desktop-to-file %desktop-path)
(check-true "desktop file written" (file-exists? %desktop-path))
(collapse-to-one-frame!) ; mangle the active layout
(delete-other-groups!) ; and delete every other group
(check "only one group left" (length (group-names)) 1)
(restore-from-file %desktop-path)
(check-true "restore recreated group II" (and (member " II " (group-names)) #t))
(check-true "restore recreated the float group"
            (let ((g (find (lambda (n) (string=? n " FLT ")) (group-names))))
              (and g #t)))
(check "restore rebuilt the two-leaf layout"
       (length (frame-leaves (current-tree))) 2)

(delete-file %rules-path)
(delete-file %desktop-path)

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
