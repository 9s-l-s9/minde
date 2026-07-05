;;; layouts-test.scm -- Guile-only unit test of layout presets,
;;; dump/apply round-trips, and persistence.
;;;
;;; Run with:
;;;   guile -L scheme tests/layouts-test.scm
;;;
;;; Same stub pattern as tests/frames-test.scm.

(use-modules (srfi srfi-1))

;; ---------------------------------------------------------------------
;; Stubs recording calls, standing in for the Rust-side subrs.
;; ---------------------------------------------------------------------

(define %placements (make-hash-table)) ; id -> (x y w h)
(define %focused #f)
(define %messages '())

(define (wm-place-window id x y w h)
  (hash-set! %placements id (list x y w h))
  #t)

(define (wm-focus-window id) (set! %focused id) #t)
(define (wm-close-window id) #t)
(define (wm-clear-focus) (set! %focused #f) #t)
(define (wm-focus-rect x y w h) #t)
(define (wm-log msg) #t)
(define (wm-message text . _) (set! %messages (cons text %messages)) #t)

;; Persistence goes to a scratch file, not the user's config.
(define %layouts-file (string-append (or (getenv "TMPDIR") "/tmp")
                                     "/minde-layouts-test.scm"))
(setenv "MINDE_LAYOUTS_FILE" %layouts-file)
(when (file-exists? %layouts-file) (delete-file %layouts-file))

(use-modules (minde frames))
(use-modules (minde layouts))

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

(define (check-true name got) (check name (if got #t #f) #t))

;; ---------------------------------------------------------------------
;; Setup: 1280x720 output, three windows in the single default frame.
;; ---------------------------------------------------------------------

(update-output-geometry! 0 0 1280 720)
(track-window-map! 1 "one" "a")
(track-window-map! 2 "two" "b")
(track-window-map! 3 "three" "c")

;; ---------------------------------------------------------------------
;; apply-layout!: main + right column (2/3 | stacked halves), windows
;; redistributed with the current window (3) into the first leaf.
;; ---------------------------------------------------------------------

(define-layout! "ms" '(hsplit 2/3 leaf (vsplit 1/2 leaf leaf)))
(check-true "layout registered" (member "ms" (layout-names)))

(apply-layout! "ms")

;; Leaf rects: (0 0 853 720), (853 0 427 360), (853 360 427 360);
;; windows placed border-inset (3px).
(check "current window 3 fills the main (left) leaf"
       (hash-ref %placements 3)
       (list 3 3 847 714))
(check "window 1 in the top-right leaf"
       (hash-ref %placements 1)
       (list 856 3 421 354))
(check "window 2 in the bottom-right leaf"
       (hash-ref %placements 2)
       (list 856 363 421 354))
(check "focus on the first leaf's window" %focused 3)
(check "window count preserved" (frame-tree-window-count) 3)

(check-true "unknown layout echoes an error, no crash"
            (begin (apply-layout! "nope")
                   (string-contains (car %messages) "no layout")))

;; ---------------------------------------------------------------------
;; dump-layout-spec round-trip: dump, re-apply, geometry unchanged.
;; ---------------------------------------------------------------------

(define %dumped (dump-layout-spec))
(check-true "dump is an hsplit" (eq? (car %dumped) 'hsplit))
(define-layout! "dumped" %dumped)
(apply-layout! "dumped")
(check "round-trip keeps the main leaf geometry"
       (hash-ref %placements 3)
       (list 3 3 847 714))
(check "round-trip keeps the stacked leaves"
       (hash-ref %placements 2)
       (list 856 363 421 354))

;; ---------------------------------------------------------------------
;; Persistence: save-layout! writes the registry; load-layouts! reads it.
;; ---------------------------------------------------------------------

(save-layout! "snap")
(check-true "layouts file written" (file-exists? %layouts-file))
(check "saved snapshot equals the live dump" (layout-spec "snap") (dump-layout-spec))
(load-layouts!)
(check-true "load-layouts! keeps the snapshot present"
            (member "snap" (layout-names)))
(check-true "saved echoed" (string-contains (car %messages) "saved layout"))

(delete-file %layouts-file)

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
