;;; winmgmt-test.scm -- sprint 8 window-management additions in
;;; (minde frames): app-id bookkeeping, pull-window-by-id!,
;;; show-window-properties!, unmark-window!, sticky list.
;;;
;;; Run with:
;;;   guile -L scheme tests/winmgmt-test.scm

(use-modules (srfi srfi-1))

;; ---------------------------------------------------------------------
;; Stubs recording calls, standing in for the Rust-side subrs.
;; ---------------------------------------------------------------------

(define %placements (make-hash-table)) ; id -> (x y w h)
(define %focused #f)

(define (wm-place-window id x y w h)
  (hash-set! %placements id (list x y w h))
  #t)

(define (wm-focus-window id)
  (set! %focused id)
  #t)

(define (wm-close-window id) #t)
(define (wm-output-geometry) (list 1280 720))
(define (wm-log msg) #t)

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

(update-output-geometry! 0 0 1280 720)

;; ---------------------------------------------------------------------
;; App-id bookkeeping: title and class both remembered; rename keeps
;; the class.
;; ---------------------------------------------------------------------

(track-window-map! 1 "Terminal" "foot")
(track-window-map! 2 "" "org.zen.browser")

(check "window-title remembered" (window-title 1) "Terminal")
(check "window-app-id remembered" (window-app-id 1) "foot")
(check "empty title falls back to the class" (window-title 2) "org.zen.browser")
(check "app-id of the fallback window" (window-app-id 2) "org.zen.browser")

(focus-window-by-id! 1)
(rename-window! "renamed")
(check "rename-window! changed the title" (window-title 1) "renamed")
(check "rename-window! kept the app-id" (window-app-id 1) "foot")

(forget-window-title! 2)
(check-true "forgotten window has no app-id" (not (window-app-id 2)))

;; ---------------------------------------------------------------------
;; pull-window-by-id!: window from another frame lands here; a float
;; unfloats.
;; ---------------------------------------------------------------------

(track-window-map! 3 "other" "foot")
(split-frame-horizontal!)
;; 1 and 3 stayed in the left frame; move 3 right, then pull it back.
(move-window! 'right)
(focus-previous-frame!)
(check "current frame no longer shows 3" (current-frame-window) 1)
(pull-window-by-id! 3)
(check "pull-window-by-id! made 3 current here" (current-frame-window) 3)
(check-true "3 is in the current frame's list"
            (and (member 3 (current-frame-window-ids)) #t))

(float-window! 3)
(check-true "3 floats" (window-floating? 3))
(pull-window-by-id! 3)
(check-true "pulling a float unfloats it" (not (window-floating? 3)))

;; ---------------------------------------------------------------------
;; show-window-properties!: echoes id/title/class and flags via the
;; message history.
;; ---------------------------------------------------------------------

(focus-window-by-id! 1)
(mark-window-toggle!)
(show-window-properties!)
(let ((m (last-message)))
  (check-true "properties echo names the title" (and (string-contains m "renamed") #t))
  (check-true "properties echo names the class" (and (string-contains m "foot") #t))
  (check-true "properties echo shows the mark" (and (string-contains m "marked") #t)))

;; unmark-window!: silent single unmark.
(unmark-window! 1)
(check "unmark-window! removed the mark" (marked-windows) '())

;; ---------------------------------------------------------------------
;; Sticky list bookkeeping (the group-switch behavior is covered in
;; groups-test.scm).
;; ---------------------------------------------------------------------

(toggle-always-show!)
(check "toggle-always-show! added the focused window" (sticky-windows) '(1))
(clear-sticky! 1)
(check "clear-sticky! removed it" (sticky-windows) '())

;; ---------------------------------------------------------------------
;; update-window-title! (sprint 10 handle-window-title-change! backing): late
;; title/app-id arrival updates the books; rename override sticks.
;; ---------------------------------------------------------------------

(track-window-map! 9 "" "")   ; Wayland clients map before set_title
(check "map-time title empty" (window-title 9) "")
(update-window-title! 9 "Page - zen" "zen")
(check "late title recorded" (window-title 9) "Page - zen")
(check "late app-id recorded" (window-app-id 9) "zen")
(update-window-title! 9 "Other page" "zen")
(check "retitle follows the client" (window-title 9) "Other page")
(focus-window-by-id! 9)
(rename-window! "pinned")
(update-window-title! 9 "Yet another page" "zen")
(check "rename override sticks over client retitles" (window-title 9) "pinned")
(check "app-id still updates under an override" (window-app-id 9) "zen")
(forget-window-title! 9)
(update-window-title! 9 "fresh" "zen")
(check "forget clears the override" (window-title 9) "fresh")

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
