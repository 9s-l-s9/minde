;;; heads-test.scm -- Guile-only unit test of the multi-head (multi-
;;; output) model: per-head frame trees, snext/sother, cross-bezel
;;; directional focus, hotplug adoption, span mode.
;;;
;;; Run with: guile -L scheme tests/heads-test.scm
;;;
;;; Same stub pattern as tests/groups-test.scm; the "hardware" is
;;; simulated entirely through handle-heads-change!.

(use-modules (srfi srfi-1))

(define %placements (make-hash-table)) ; id -> (x y w h)
(define %focused #f)
(define %messages '())

(define (wm-place-window id x y w h) (hash-set! %placements id (list x y w h)) #t)
(define (wm-focus-window id) (set! %focused id) #t)
(define (wm-close-window id) #t)
(define (wm-clear-focus) (set! %focused #f) #t)
(define (wm-focus-rect x y w h) #t)
(define (wm-output-geometry) (list 0 0 1920 1080))
(define (wm-log msg) #t)
(define (wm-message text . _) (set! %messages (cons text %messages)) #t)

(use-modules (minde compositor frames))
(use-modules (minde groups))

(define %failures 0)
(define (check name got expected)
  (if (equal? got expected)
      (format #t "ok - ~a~%" name)
      (begin
        (set! %failures (+ %failures 1))
        (format #t "FAIL - ~a: expected ~s, got ~s~%" name expected got))))
(define (check-true name got) (check name (if got #t #f) #t))
(define (placement id) (hash-ref %placements id))
(define (onscreen-at? id x y)
  (let ((p (placement id)))
    (and p (= (car p) x) (= (cadr p) y))))
(define (offscreen? id)
  (let ((p (placement id)))
    (and p (< (car p) 0) (< (cadr p) 0))))

;; ---------------------------------------------------------------------
;; One head to start; a window maps onto it.
;; ---------------------------------------------------------------------

(handle-heads-change! '((10 0 0 1920 1080)))
(check "single head registered" (map car (heads)) '(10))
(check "current head follows the only head" (current-head-id) 10)

(handle-window-map! 1 "term" "foot")
(check-true "window 1 fills head A (minus border)" (onscreen-at? 1 3 3))

;; ---------------------------------------------------------------------
;; Second head appears to the right: desktop extends, window 1 stays.
;; ---------------------------------------------------------------------

(handle-heads-change! '((10 0 0 1920 1080) (11 1920 0 1280 1024)))
(check "two heads registered" (map car (heads)) '(10 11))
(check "current head unchanged on hotplug" (current-head-id) 10)
(check-true "window 1 undisturbed" (onscreen-at? 1 3 3))

;; focus-next-head! switches to head B; its tree is empty; a new window maps there.
(focus-next-head!)
(check "focus-next-head! moved to head B" (current-head-id) 11)
(handle-window-map! 2 "editor" "lem")
(check-true "window 2 fills head B at its origin"
            (onscreen-at? 2 1923 3))
(check-true "window 1 still visible on head A" (onscreen-at? 1 3 3))
(check "windows numbered uniquely across heads"
       (sort (list (window-number 1) (window-number 2)) <) '(0 1))

;; focus-last-head! toggles back; focus-next-head!/focus-previous-head! cycle.
(focus-last-head!)
(check "focus-last-head! back on head A" (current-head-id) 10)
(focus-previous-head!)
(check "focus-previous-head! wraps to head B" (current-head-id) 11)
(focus-last-head!)

;; ---------------------------------------------------------------------
;; Directional focus crosses the bezel; focus-window-by-id! switches heads.
;; ---------------------------------------------------------------------

(move-focus! 'right)
(check "move-focus! right crossed to head B" (current-head-id) 11)
(check "focus landed on window 2" (current-frame-window) 2)
(move-focus! 'left)
(check "move-focus! left crossed back" (current-head-id) 10)

(focus-window-by-id! 2)
(check "focus-window-by-id! switched heads" (current-head-id) 11)
(check "and focused the window" (current-frame-window) 2)
(focus-window-by-id! 1)
(check "and back" (current-head-id) 10)

;; move-window! across the bezel carries the window along.
(move-window! 'right)
(check "move-window! right landed on head B" (current-head-id) 11)
(check-true "window 1 now placed on head B" (onscreen-at? 1 1923 3))
(check-true "window 2 hidden behind it (same frame)" (offscreen? 2))
(move-window! 'left) ; bring it back
(check "window 1 back on head A" (current-head-id) 10)

;; ---------------------------------------------------------------------
;; Per-head trees are independent: split on A doesn't touch B.
;; ---------------------------------------------------------------------

(split-frame-vertical!)
(check "head A has two frames" (length (frame-leaves (current-tree))) 2)
(focus-next-head!)
(check "head B still one frame" (length (frame-leaves (current-tree))) 1)
(focus-last-head!)

;; Group switching keeps the current head; echo lists both heads' windows.
(switch-to-next-group!)
(check "group II starts empty across heads" (echo-windows-string) "no windows")
(switch-to-previous-group!)
(check-true "back in I, both windows listed"
            (let ((s (echo-windows-string)))
              (and (string-contains s "term") (string-contains s "editor"))))

;; ---------------------------------------------------------------------
;; Unplugging head B adopts its windows into the surviving head.
;; ---------------------------------------------------------------------

(focus-window-by-id! 2) ; be ON head B when it dies
(handle-heads-change! '((10 0 0 1920 1080)))
(check "back to one head" (map car (heads)) '(10))
(check "current head fell back to A" (current-head-id) 10)
(check-true "window 2 adopted into head A's frames"
            (member 2 (append-map frame-window-ids
                                  (frame-leaves (current-tree)))))
(check "no windows lost" (group-window-count (current-group)) 2)
(check-true "window 2 has a number still" (number? (window-number 2)))

;; ---------------------------------------------------------------------
;; Span mode: two heads become one big synthetic head.
;; ---------------------------------------------------------------------

(handle-heads-change! '((10 0 0 1920 1080) (11 1920 0 1280 1024)))
(set-head-mode! 'span)
(check "span mode: one effective head" (length (heads)) 1)
(check "span head covers the union"
       (cdr (car (heads))) '(0 0 3200 1080))
(check-true "a frame can now span the bezel"
            (>= (caddr (current-frame-rect)) 1920))
(set-head-mode! 'per-head)
(check "back to per-head: two heads again" (map car (heads)) '(10 11))

;; ---------------------------------------------------------------------
;; Output configuration wrappers: `configure-output!' forwards only the
;; keywords given as an alist to the `wm-configure-output!' primitive
;; and `output-heads' returns the `wm-output-heads' snapshot verbatim.
;; The primitives are stubbed at (guile-user) top level exactly as Rust
;; registers them.
;; ---------------------------------------------------------------------

(check "output-heads without the primitive is empty" (output-heads) '())
(check "configure-output! without the primitive is #f"
       (configure-output! "DP-1" #:scale 2) #f)

(define %configure-calls '())
(define (wm-configure-output! name alist)
  (set! %configure-calls (cons (cons name alist) %configure-calls))
  #t)
(define %fake-heads
  '(((name . "eDP-1") (enabled . #t) (make . "AUO") (model . "B140HAN")
     (serial . "") (description . "AUO B140HAN")
     (mode . (1920 1080 60000)) (preferred-mode . (1920 1080 60000))
     (position . (0 0)) (scale . 1.0) (transform . normal)
     (adaptive-sync . unsupported) (modes . ((1920 1080 60000))))
    ((name . "DP-1") (enabled . #f) (make . "DEL") (model . "U2723QE")
     (serial . "ABC") (description . "DEL U2723QE ABC")
     (mode . #f) (preferred-mode . (3840 2160 60000))
     (position . (1920 0)) (scale . 1.5) (transform . normal)
     (adaptive-sync . #f) (modes . ((3840 2160 60000) (1920 1080 60000))))))
(define (wm-output-heads) %fake-heads)

(check "output-heads returns every head, disabled ones too"
       (map (lambda (h) (assq-ref h 'name)) (output-heads))
       '("eDP-1" "DP-1"))
(check "output-heads keeps the disabled head's identity and modes"
       (let ((dp (cadr (output-heads))))
         (list (assq-ref dp 'enabled) (assq-ref dp 'model)
               (length (assq-ref dp 'modes))))
       '(#f "U2723QE" 2))

(check-true "configure-output! returns #t when queued"
            (configure-output! "DP-1" #:scale 1.5 #:position '(1920 0)))
(check "configure-output! forwards only the given keywords"
       (car %configure-calls)
       '("DP-1" (position . (1920 0)) (scale . 1.5)))
(configure-output! "DP-1")
(check "configure-output! with no settings sends an empty alist"
       (car %configure-calls) '("DP-1"))
(configure-output! "eDP-1" #:mode "1920x1080@60" #:transform '90
                   #:enabled #t #:adaptive-sync #f)
(check "configure-output! forwards mode/transform/enabled/adaptive-sync"
       (car %configure-calls)
       '("eDP-1" (mode . "1920x1080@60") (transform . 90)
         (enabled . #t) (adaptive-sync . #f)))

(check "output-configuration-allowed? defaults to accept"
       (output-configuration-allowed?) #t)
(check-true "handle-output-configured! default is a no-op"
            (begin (handle-output-configured!) #t))
(check-true "handle-output-configure-failed! default logs, never raises"
            (begin (handle-output-configure-failed! "DP-1" "unknown mode") #t))

;; ---------------------------------------------------------------------
;; Keyboard layout wrappers over stubbed `wm-keyboard-layouts' /
;; `wm-set-keyboard-layout!' primitives.
;; ---------------------------------------------------------------------

(check "keyboard-layouts without the primitive is empty" (keyboard-layouts) '())
(check "set-keyboard-layout! without the primitive is #f"
       (set-keyboard-layout! 'next) #f)

(define %layout-calls '())
(define (wm-set-keyboard-layout! spec)
  (set! %layout-calls (cons spec %layout-calls))
  #t)
(define (wm-keyboard-layouts)
  '(((name . "German (Bone)") (active . #t))
    ((name . "English (US)") (active . #f))))

(check "keyboard-layouts returns the snapshot"
       (map (lambda (l) (assq-ref l 'name)) (keyboard-layouts))
       '("German (Bone)" "English (US)"))
(check-true "set-keyboard-layout! next is queued" (set-keyboard-layout! 'next))
(check "set-keyboard-layout! forwards next" (car %layout-calls) 'next)
(set-keyboard-layout! 1)
(check "set-keyboard-layout! forwards an index" (car %layout-calls) 1)
(set-keyboard-layout! "english")
(check "set-keyboard-layout! resolves a name prefix to its index"
       (car %layout-calls) 1)
(check "set-keyboard-layout! with an unknown name is #f"
       (set-keyboard-layout! "French") #f)
(check-true "set-keyboard-layout! rejects malformed specs"
            (catch #t (lambda () (set-keyboard-layout! -1) #f)
              (lambda _ #t)))
(check-true "handle-keyboard-layout-changed! default never raises"
            (begin (handle-keyboard-layout-changed! "English (US)") #t))

;; ---------------------------------------------------------------------

(if (zero? %failures)
    (begin (format #t "all tests passed~%") (exit 0))
    (begin (format #t "~a test(s) FAILED~%" %failures) (exit 1)))
