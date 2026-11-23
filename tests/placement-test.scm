;;; placement-test.scm -- frame dumps, sibling, expose,
;;; unmaximize/gravity, rule lock flag + persistence, desktop dump.
;;;
;;; Run with:
;;;   guile -L scheme tests/placement-test.scm

(use-modules (srfi srfi-1)
             (minde placement)
             (minde foundation serialization))

;; ---------------------------------------------------------------------
;; Stubs recording calls, standing in for the Rust-side subrs.
;; ---------------------------------------------------------------------

(define %placements (make-hash-table)) ; id -> (x y w h), wm-place-window
(define %float-places (make-hash-table)) ; id -> (x y w h), wm-place-float
(define %focused #f)
(define %overlays '())
(define %messages '())
(define (wm-message text . _) (set! %messages (cons text %messages)))
(define %client-titles (make-hash-table))
(define (wm-window-title id) (hash-ref %client-titles id))

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
(use-modules (minde groups) (minde hooks) (minde compositor model))

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

;; App identifiers are identities; titles are independent substring predicates.
(for-each
 (lambda (case)
   (let ((matcher (list-ref case 0)) (title (list-ref case 1))
         (app (list-ref case 2)) (expected (list-ref case 3)))
     (check (format #f "matcher ~s against ~s / ~s" matcher title app)
            (placement-rule-matches? (list matcher #f 0 #f #t #f) title app)
            expected)))
 '((((app-id . "code")) "editor" "code" #t)
   (((app-id . "code")) "editor" "code-oss" #f)
   (((app-id . "code")) "editor" "barcode-reader" #f)
   (((app-id . "foot")) "football results" "browser" #f)
   (((app-id . "foot")) "foot" "" #f)
   (((legacy . "foot")) "football results" "browser" #f)
   (((legacy . "foot")) "terminal" "foot-extra" #f)
   (((legacy . "foot")) "terminal" "foot" #t)
   (((legacy . "foot")) "football results" "" #t)
   (((legacy . "foot")) "football results" #f #t)
   (((legacy . "foot")) #f #f #f)
   (((title . "Picture-in-Picture")) "Picture-in-Picture — video" "firefox" #t)
   (((title . "Picture-in-Picture")) "video" "Picture-in-Picture" #f)
   (((app-id . "firefox") (title . "Picture")) "Picture-in-Picture" "firefox" #t)
   (((app-id . "firefox") (title . "Picture")) "Picture-in-Picture" "zen" #f)
   (((app-id . "firefox") (title . "Picture")) "homepage" "firefox" #f)))

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

(add-placement-rule! #:title "Picture-in-Picture" #:float? #t)
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

;; Rust exposes the new metadata before delivering its notification.
;; Rules and events must compare against the previously reported raw pair,
;; even when a user rename overrides the title displayed in window lists.
(define title-events '())
(define (record-title id title app-id)
  (set! title-events (append title-events (list (list id title app-id)))))
(add-event-hook! 'window-title-changed record-title)
(define (retitle! id title app-id)
  (hash-set! %client-titles id (cons title app-id))
  (handle-window-title-change! id title app-id))
(handle-window-map! 120 "" "")
(retitle! 120 "Other" "")
(retitle! 120 "Other" "")
(check "retitle events detect changes despite live metadata" title-events '((120 "Other" "")))
(retitle! 120 "Picture-in-Picture" "")
(check-true "live title transition triggers the float rule" (window-floating? 120))
(unfloat-window! 120)
(focus-window-by-id! 120)
(rename-window! "pinned")
(retitle! 120 "Picture-in-Picture again" "")
(check "rename does not retrigger an already matching rule" (window-floating? 120) #f)
(retitle! 120 "Third" "")
(retitle! 120 "Picture-in-Picture" "")
(check-true "new title edge still triggers beneath a rename" (window-floating? 120))
(check "client changes preserve the display override" (window-title 120) "pinned")
(check "each real title edge emits its raw client metadata" title-events
       '((120 "Other" "") (120 "Picture-in-Picture" "")
         (120 "Picture-in-Picture again" "") (120 "Third" "")
         (120 "Picture-in-Picture" "")))
(handle-window-map! 121 "" "")
(retitle! 121 "browser" "zen")
(check-true "live late app-id applies the placement rule" (group-has-window? " III " 121))
(handle-window-map! 122 "" "test")
(retitle! 122 "test" "test")
(check "display-title fallback is not previous client metadata"
       (last title-events) '(122 "test" "test"))
(remove-event-hook! 'window-title-changed record-title)
(for-each handle-window-unmap! '(120 121 122))
(hash-clear! %client-titles)

;; ---------------------------------------------------------------------
;; remember! / forget! + persistence round-trip
;; ---------------------------------------------------------------------

(focus-window-by-id! 1)
(remember!)
(check-true "remember! wrote the rules file" (file-exists? %rules-path))
(define %saved-rules
  (read-versioned-datum-file %rules-path 'minde-placement-rules 2))
(check "remembered rule pins foot to the original group and frame"
       (find (lambda (rule) (equal? (car rule) '((app-id . "foot")))) %saved-rules)
       '(((app-id . "foot")) "I" 0 #f #t #f))
(clear-placement-rules!)
(check "rules cleared before reload" (@@ (minde groups) %placement-rules) '())
(load-placement-rules!)
(check "all rules survived a save/load round-trip"
       (@@ (minde groups) %placement-rules) %saved-rules)
(for-each
 (lambda (boot)
   (clear-placement-rules!)
   (load-placement-rules!)
   (add-placement-rule! #:title "Picture-in-Picture" #:float? #t)
   (remember!)
   (check (format #f "restart ~a keeps identical rules and order" boot)
          (read-versioned-datum-file %rules-path 'minde-placement-rules 2)
          %saved-rules))
 '(1 2 3))
;; New matching windows must be placed by the restored rule, even when the
;; selected frame would normally receive them elsewhere.
(focus-frame-by-index! 1)
(handle-window-map! 14 "new terminal" "foot")
(check-true "reloaded rule places a matching window in the remembered frame"
            (member 14 (frame-window-ids (car (frame-leaves (current-tree))))))
(check-true "matching window did not go to the selected second frame"
            (not (member 14 (frame-window-ids (cadr (frame-leaves (current-tree)))))))
(handle-window-unmap! 14)
(focus-window-by-id! 1)
(forget!)
(check-true "forget! removed the remembered rule, keeping the others"
            (let ((rules (read-versioned-datum-file
                          %rules-path 'minde-placement-rules 2)))
              (and (= (length rules) 3) ; term + zen + the float rule
                   (not (find (lambda (r) (equal? (car r) '((app-id . "foot")))) rules)))))

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

;; A late follow rule targeting the current group must still place the
;; window in its new frame, whether the group is named or implicit.
(create-group! "late-placement")
(split-frame-horizontal!)
(for-each
 (lambda (options)
   (clear-placement-rules!)
   (focus-frame-by-index! 0)
   (handle-window-map! 130 "late" "")
   (add-placement-rule! "late-same-group" #:group (car options)
                        #:frame 1 #:follow? (cadr options))
   (hash-clear! %placements)
   (handle-window-title-change! 130 "late" "late-same-group")
   (check "late rule moves the window into the selected target frame"
          (cadr (cadr (dump-frames))) '(130))
   (check-true "late rule immediately places the window in its new frame"
               (let ((r (hash-ref %placements 130))) (and r (> (car r) 600))))
   (handle-window-unmap! 130))
 '((#f #t) ("late-placement" #t) (#f #f)))

;; A floating target group determines window kind for map-time, late and
;; explicit placement, including parking before a hidden group is shown.
(clear-placement-rules!)
(create-floating-group! "rule-floats")
(for-each
 (lambda (target id)
   (clear-placement-rules!)
   (add-placement-rule! "float-group-app" #:group target)
   (handle-window-map! id "float candidate" "float-group-app")
   (check-true "rule mapping into a float group floats" (window-floating? id))
   (check "float group keeps its unused frame empty" (frame-tree-window-count) 0)
   (check "new float in the active group receives input focus" %focused id)
   (check-true "new float is placed with floating geometry" (hash-ref %float-places id)))
 '(#f "rule-floats") '(140 141))
(define hidden-floats (create-floating-group-in-background! "hidden-rule-floats"))
(switch-to-group! "late-placement")
(clear-placement-rules!)
(add-placement-rule! "hidden-float" #:group "hidden-rule-floats")
(handle-window-map! 143 "hidden" "hidden-float")
(handle-window-map! 144 "late hidden" "")
(handle-window-title-change! 144 "late hidden" "hidden-float")
(handle-window-map! 145 "explicit hidden" "explicit-float")
(add-placement-rule! "explicit-float" #:group "hidden-rule-floats" #:lock? #f)
(place-existing-windows!)
(for-each
 (lambda (id)
   (check-true "each placement path floats in the hidden target" (window-floating? id))
   (check-true "hidden float is owned by the target group" (member id (group-floats hidden-floats)))
   (check-true "hidden floats are parked with their floating state"
               (let ((r (hash-ref %float-places id))) (and r (< (car r) 0)))))
 '(143 144 145))
(check "hidden floating group keeps all frame trees empty"
       (append-map (lambda (tree) (append-map frame-window-ids (frame-leaves tree)))
                   (group-all-trees hidden-floats)) '())
(check "rules without follow preserve the active group" (current-group-name) "late-placement")
(add-placement-rule! "follow-float" #:group "hidden-rule-floats" #:follow? #t)
(handle-window-map! 146 "follow" "follow-float")
(check "follow switches to the floating group" (current-group-name) "hidden-rule-floats")
(check-true "follow places the new float visibly"
            (let ((r (hash-ref %float-places 146))) (and r (>= (car r) 0))))
(check "follow focuses the new float" %focused 146)

;; Exercise independent matchers through real routing and remember/forget.
(clear-placement-rules!)
(create-group! "exact-source")
(create-group-in-background! "exact-target")
(add-placement-rule! #:app-id "code" #:title "notes" #:group "exact-target")
(add-placement-rule! #:app-id "foot" #:group "exact-target")
(handle-window-map! 160 "notes" "code")
(handle-window-map! 161 "notes" "barcode-reader")
(handle-window-map! 162 "other document" "code")
(handle-window-map! 163 "football results" "browser")
(check-true "AND match routes to the target" (group-has-window? "exact-target" 160))
(for-each (lambda (id) (check-true "unrelated window stays in source"
                                  (group-has-window? "exact-source" id))) '(161 162 163))
(focus-window-by-id! 163)
(forget!)
(check "forget on a browser title preserves the terminal rule"
       (length (@@ (minde groups) %placement-rules)) 2)
(handle-window-map! 164 "terminal" "foot")
(check-true "terminal still uses its rule" (group-has-window? "exact-target" 164))
(switch-to-group! "exact-target")
(focus-window-by-id! 164)
(forget!)
(check "forget removes only the actual matching rule"
       (map car (@@ (minde groups) %placement-rules))
       '(((app-id . "code") (title . "notes"))))
(handle-window-map! 165 "transient README title" "")
(focus-window-by-id! 165)
(let ((before (read-datum-file %rules-path))
      (rules (@@ (minde groups) %placement-rules)))
  (check "remember refuses an empty app-id" (remember!) #f)
  (check "refused remember preserves the file" (read-datum-file %rules-path) before)
  (check "refused remember preserves the registry" (@@ (minde groups) %placement-rules) rules)
  (check-true "remember explains the missing app-id"
              (any (lambda (text) (string-contains text "no app-id")) %messages)))
(delete-file %rules-path)

;; Display names may fall back to app IDs or be renamed. Neither is a title
;; reported by the client, and neither may influence explicit title rules.
(clear-placement-rules!)
(create-group! "raw-title-source")
(handle-window-map! 166 "" "title-token")
(handle-window-map! 167 "actual title" "browser")
(focus-window-by-id! 167)
(rename-window! "title-token")
(add-placement-rule! #:title "title-token" #:group "exact-target" #:lock? #f)
(forget!)
(check "forget ignores a display-title override"
       (length (@@ (minde groups) %placement-rules)) 1)
(add-placement-rule! #:title "title-token" #:group "exact-target" #:lock? #f)
(handle-window-map! 168 "title-token" "browser")
(focus-window-by-id! 168)
(rename-window! "different display name")
(place-existing-windows!)
(check-true "title rule ignores an app-id display fallback"
            (group-has-window? "raw-title-source" 166))
(check-true "title rule ignores a display-title override"
            (group-has-window? "raw-title-source" 167))
(check-true "title rule uses the client title even after renaming"
            (group-has-window? "exact-target" 168))
(delete-file %rules-path)

;; ---------------------------------------------------------------------
;; Dialogs: transient / DIALOG / UTILITY / SPLASH windows float centered over
;; their parent, in their own size; plain toplevels stay tiled; the
;; #:dialog? selector overrides the default.
;; ---------------------------------------------------------------------

(define %hints (make-hash-table))   ; id -> wm-window-hints alist
(define %geoms (make-hash-table))   ; id -> wm-window-geometry list
(define (wm-window-hints id) (hash-ref %hints id #f))
(define (wm-window-geometry id) (hash-ref %geoms id #f))

(clear-placement-rules!)
(handle-window-map! 300 "parent" "app")
(hash-set! %hints 300 '((parent . #f) (type . normal) (size . #f)))
(hash-set! %geoms 300 '(100 100 400 300))
(check-true "toplevel without parent stays tiled" (not (window-floating? 300)))
(check "window-dialog? is false for a plain toplevel" (window-dialog? 300) #f)

(hash-set! %hints 301 '((parent . 300) (type . normal) (size . (200 . 100))))
(handle-window-map! 301 "Open file" "app")
(check-true "transient window floats" (window-floating? 301))
(check "transient window is centered over its parent in its own size"
       (float-geometry 301) '(200 200 200 100))

(hash-set! %hints 302 '((parent . #f) (type . dialog) (size . #f)))
(handle-window-map! 302 "Settings" "other")
(check-true "dialog type without parent floats" (window-floating? 302))
(check "dialog without size hint uses the default float size"
       (cddr (float-geometry 302)) (cddr (default-float-rect)))

(hash-set! %hints 303 '((parent . #f) (type . splash) (size . (5000 . 5000))))
(handle-window-map! 303 "Splash" "other")
(check "oversized hint is clamped to the head"
       (cddr (float-geometry 303)) (list (list-ref (car (heads)) 3) (list-ref (car (heads)) 4)))

(hash-set! %hints 304 '((parent . #f) (type . other) (size . #f)))
(handle-window-map! 304 "tooltip-ish" "other")
(check-true "other window types stay tiled" (not (window-floating? 304)))

;; Selector: #:dialog? matches only dialogs and can keep them tiled.
(add-placement-rule! #:app-id "keep" #:dialog? #t #:float? #f)
(hash-set! %hints 305 '((parent . 300) (type . normal) (size . #f)))
(handle-window-map! 305 "dlg" "keep")
(check-true "#:dialog? #:float? #f keeps a dialog tiled" (not (window-floating? 305)))
(hash-set! %hints 306 '((parent . #f) (type . normal) (size . #f)))
(handle-window-map! 306 "main" "keep")
(check-true "dialog rule ignores non-dialogs" (not (window-floating? 306)))
(check "dialog selector matcher is canonical data"
       (car (car (@@ (minde groups) %placement-rules))) '((app-id . "keep") (dialog . #t)))
(check "dialog-only rule needs a dialog"
       (placement-rule-matches? (list '((dialog . #t)) #f 0 #f #t #f) "t" "a") #f)
(check "dialog-only rule matches a dialog"
       (placement-rule-matches? (list '((dialog . #t)) #f 0 #f #t #f) "t" "a" #t) #t)
(check-true "dialog rules validate"
            (valid-placement-rules? (list (list '((dialog . #t)) #f 0 #f #t #t))))
(check "dialog selector must be #t"
       (valid-placement-rules? (list (list '((dialog . #f)) #f 0 #f #t #t))) #f)
(clear-placement-rules!)
(add-placement-rule! #:dialog? #t #:float? #t #:group "III")
(check-true "dialog rule alone is valid" (= 1 (length (@@ (minde groups) %placement-rules))))
(clear-placement-rules!)

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin
      (format #t "all tests passed~%")
      (exit 0))
    (begin
      (format #t "~a test(s) FAILED~%" %failures)
      (exit 1)))
