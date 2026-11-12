;;; winmgmt-test.scm -- window-management additions in
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
(use-modules (minde compositor frames) (minde compositor model))

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

(let ((ids (current-frame-window-ids))
      (count (group-window-count (current-group))))
  (pull-window-by-id! 3)
  (pull-window-by-id! 3)
  (pull-window-by-number! (window-number 1))
  (check "pulling a resident by number selects it" (current-frame-window) 1)
  (pull-window-by-number! (window-number 3))
  (check "repeated pulls preserve frame membership and order"
         (current-frame-window-ids) ids)
  (check "repeated pulls preserve the group count"
         (group-window-count (current-group)) count))

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
;; update-window-title! (handle-window-title-change! backing): late
;; title/app-id arrival updates the books; rename override sticks.
;; ---------------------------------------------------------------------

(track-window-map! 9 "" "")   ; Wayland clients map before set_title
(check "map-time title empty" (window-title 9) "")
(check "title update returns the previous raw metadata"
       (update-window-title! 9 "Page - zen" "zen") '("" . ""))
(check "late title recorded" (window-title 9) "Page - zen")
(check "late app-id recorded" (window-app-id 9) "zen")
(update-window-title! 9 "Other page" "zen")
(check "retitle follows the client" (window-title 9) "Other page")
(focus-window-by-id! 9)
(rename-window! "pinned")
(check "title update returns client metadata beneath a rename"
       (update-window-title! 9 "Yet another page" "zen") '("Other page" . "zen"))
(check "rename override sticks over client retitles" (window-title 9) "pinned")
(check "app-id still updates under an override" (window-app-id 9) "zen")
(forget-window-title! 9)
(update-window-title! 9 "fresh" "zen")
(check "forget clears the override" (window-title 9) "fresh")

;; Adding a window must preserve the frame's identity invariant, including
;; the valid ID zero. Invalid IDs must fail before changing the frame.
(let ((frame (make-frame 0 0 100 100 '() #f)))
  (frame-add-window! frame 0)
  (frame-add-window! frame 0)
  (check "frame insertion is idempotent for window zero"
         (frame-window-ids frame) '(0))
  (check "window zero is selectable" (frame-current-window frame) 0))
(for-each
 (lambda (id)
   (let ((frame (make-frame 0 0 100 100 '() #f)))
     (check-true "invalid frame member is rejected"
       (catch #t (lambda () (frame-add-window! frame id) #f) (lambda _ #t)))
     (check "rejected insertion preserves frame membership" (frame-window-ids frame) '())
     (check "rejected insertion preserves frame selection" (frame-current-window frame) #f)))
 '(#f -1 1.5 "1" bogus))

;; A cleared source retains windows but has no selected window. Pull its
;; first hidden window without revealing the others or inserting #f.
(for-each track-window-unmap! (all-window-ids))
(collapse-to-one-frame!)
(track-window-map! 90 "hidden one" "foot")
(track-window-map! 91 "hidden two" "foot")
(split-frame-horizontal!)
(define cleared-source (current-frame))
(clear-current-frame!)
(focus-next-frame!)
(pull-window-from-other-frame!)
(check "pull from cleared source moves its first window" (current-frame-window-ids) '(90))
(check "pulled hidden window receives focus" %focused 90)
(check "remaining hidden window stays in the source" (frame-window-ids cleared-source) '(91))
(check "pull does not reveal the cleared source" (frame-current-window cleared-source) #f)
(pull-window-from-other-frame!)
(check "second hidden window moves exactly once" (current-frame-window-ids) '(90 91))
(check "source is empty after both pulls" (frame-window-ids cleared-source) '())
(check "empty source stays cleared" (frame-current-window cleared-source) #f)
(pull-window-from-other-frame!)
(check "pull with no source preserves membership" (current-frame-window-ids) '(90 91))
(check "pull with no source preserves focus" %focused 91)

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
