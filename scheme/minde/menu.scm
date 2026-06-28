;;; menu.scm -- multi-line navigable menu, StumpWM's select-from-menu.
;;;
;;; Renders a list of items in the message overlay (which is already
;;; multi-line; see render_message in src/render.rs) with a "> " marker
;;; on the selected row, and owns the keyboard while open -- init.scm's
;;; wm-handle-key delegates every keypress here while (menu-active?),
;;; exactly like the input prompt.
;;;
;;; Keys: C-n/C-p/Down/Up move, digits jump into the visible page,
;;; Return selects, C-g/Escape aborts. Printable characters narrow the
;;; list incrementally (StumpWM's menu search); BackSpace widens it
;;; again. Pass #:filter? #f to disable filtering, which frees j/k for
;;; navigation (rat-free StumpWM style).
;;;
;;; Same load-time constraint as frames.scm: nothing here calls a wm-*
;;; Rust subr at module load time.

(define-module (minde menu)
  #:use-module (srfi srfi-1)
  #:use-module (srfi srfi-9)
  #:use-module (ice-9 optargs)
  #:export (select-from-menu
            menu-active?
            menu-handle-key!
            menu-abort!))

(define (rust-call name . args)
  (let* ((mod (resolve-module '(guile-user) #:ensure #f))
         (var (and mod (module-variable mod name))))
    (if var (apply (variable-ref var) args) #f)))

(define (show! text) (rust-call 'wm-message text 0)) ; sticky
(define (clear!) (rust-call 'wm-clear-message))

;; ---------------------------------------------------------------------
;; State
;; ---------------------------------------------------------------------

;; ITEMS is the full list of (label . value) pairs; VIEW is the filtered
;; list currently shown; SELECTED indexes into VIEW; OFFSET is the first
;; visible row (scrolling window of %page-size rows).
(define-record-type <menu-state>
  (make-menu-state prompt items view selected offset filter filter?
                   on-select on-abort)
  menu-state?
  (prompt menu-prompt)
  (items menu-items)
  (view menu-view set-menu-view!)
  (selected menu-selected set-menu-selected!)
  (offset menu-offset set-menu-offset!)
  (filter menu-filter set-menu-filter!)
  (filter? menu-filter?)
  (on-select menu-on-select)
  (on-abort menu-on-abort))

(define %menu #f)

;; Rows shown at once; the message overlay caps its own height too, so
;; keep this comfortably below a 720p screen's line budget.
(define %page-size 15)

(define (menu-active?) (and %menu #t))

(define* (select-from-menu items on-select
                           #:key (prompt "") (on-abort #f) (filter? #t))
  "Opens a menu over ITEMS -- (label . value) pairs, or plain strings
(which stand for themselves). ON-SELECT is called with the chosen
item's value; ON-ABORT (if given) on C-g/Escape."
  (let ((norm (map (lambda (it) (if (pair? it) it (cons it it))) items)))
    (if (null? norm)
        (rust-call 'wm-message "nothing to select" 1500)
        (begin
          (set! %menu (make-menu-state prompt norm norm 0 0 "" filter?
                                       on-select on-abort))
          (redraw!)))))

;; ---------------------------------------------------------------------
;; Rendering
;; ---------------------------------------------------------------------

(define (visible-rows)
  (let* ((m %menu)
         (view (menu-view m))
         (off (menu-offset m)))
    (take (drop view (min off (length view)))
          (min %page-size (- (length view) (min off (length view)))))))

(define (redraw!)
  (let* ((m %menu)
         (view (menu-view m))
         (off (menu-offset m))
         (sel (menu-selected m))
         (header (string-append
                  (menu-prompt m)
                  (if (string-null? (menu-filter m))
                      ""
                      (string-append " [" (menu-filter m) "]"))))
         (rows
          (let loop ((items (visible-rows)) (i off) (acc '()))
            (if (null? items)
                (reverse acc)
                (loop (cdr items) (+ i 1)
                      (cons (format #f "~a~a ~a"
                                    (if (= i sel) "> " "  ")
                                    (if (< (- i off) 10)
                                        (number->string (- i off))
                                        " ")
                                    (car (car items)))
                            acc))))))
    (show! (string-join
            (append (if (string-null? header) '() (list header))
                    (if (null? rows) (list "  (no match)") rows)
                    (if (< (+ off %page-size) (length view))
                        (list (format #f "  ... (~a more)"
                                      (- (length view) off %page-size)))
                        '()))
            "\n"))))

;; ---------------------------------------------------------------------
;; Navigation / filtering
;; ---------------------------------------------------------------------

(define (clamp-view!)
  "Keeps SELECTED inside the view and OFFSET scrolled to show it."
  (let* ((m %menu)
         (n (length (menu-view m)))
         (sel (max 0 (min (menu-selected m) (- n 1)))))
    (set-menu-selected! m (max 0 sel))
    (let ((off (menu-offset m)))
      (cond
       ((< sel off) (set-menu-offset! m sel))
       ((>= sel (+ off %page-size)) (set-menu-offset! m (- sel (- %page-size 1))))))))

(define (move-selection! delta)
  (let* ((m %menu)
         (n (length (menu-view m))))
    (when (> n 0)
      (set-menu-selected! m (modulo (+ (menu-selected m) delta) n))
      ;; Wrapping jumps need the offset re-derived, not just nudged.
      (let ((sel (menu-selected m)))
        (set-menu-offset! m (max 0 (min (menu-offset m) sel)))
        (when (>= sel (+ (menu-offset m) %page-size))
          (set-menu-offset! m (- sel (- %page-size 1))))))
    (redraw!)))

(define (apply-filter!)
  (let* ((m %menu)
         (f (string-downcase (menu-filter m))))
    (set-menu-view! m
                    (if (string-null? f)
                        (menu-items m)
                        (filter (lambda (it)
                                  (string-contains (string-downcase (car it)) f))
                                (menu-items m))))
    (set-menu-selected! m 0)
    (set-menu-offset! m 0)
    (redraw!)))

(define (close!)
  (set! %menu #f)
  (clear!))

(define (menu-abort!)
  (let ((m %menu))
    (close!)
    (when (and m (menu-on-abort m))
      ((menu-on-abort m)))))

(define (select-current!)
  (let* ((m %menu)
         (view (menu-view m))
         (sel (menu-selected m)))
    (if (or (null? view) (>= sel (length view)))
        (menu-abort!)
        (let ((value (cdr (list-ref view sel)))
              (on-select (menu-on-select m)))
          (close!)
          (on-select value)))))

(define (select-visible-digit! d)
  "Digits select from the visible page (matching the numbers drawn)."
  (let* ((m %menu)
         (idx (+ (menu-offset m) d)))
    (when (< idx (length (menu-view m)))
      (set-menu-selected! m idx)
      (select-current!))))

(define (menu-handle-key! mods keysym-name utf8)
  "Handles one keypress while the menu is open. Always returns #t (the
menu owns the keyboard)."
  (let* ((m %menu)
         (ctrl? (positive? (logand mods 4)))
         (key (if ctrl? (string-append "C-" keysym-name) keysym-name)))
    (cond
     ((or (equal? key "C-g") (string=? keysym-name "Escape"))
      (menu-abort!))
     ((string=? keysym-name "Return") (select-current!))
     ((or (equal? key "C-n") (string=? keysym-name "Down")) (move-selection! 1))
     ((or (equal? key "C-p") (string=? keysym-name "Up")) (move-selection! -1))
     ((and (not (menu-filter? m)) (equal? key "j")) (move-selection! 1))
     ((and (not (menu-filter? m)) (equal? key "k")) (move-selection! -1))
     ((and (= (string-length keysym-name) 1)
           (char-numeric? (string-ref keysym-name 0))
           (not ctrl?))
      (select-visible-digit! (- (char->integer (string-ref keysym-name 0))
                                (char->integer #\0))))
     ((string=? keysym-name "BackSpace")
      (let ((f (menu-filter m)))
        (unless (string-null? f)
          (set-menu-filter! m (substring f 0 (- (string-length f) 1)))
          (apply-filter!))))
     ((and (menu-filter? m) (string? utf8) (= (string-length utf8) 1)
           (not ctrl?))
      (set-menu-filter! m (string-append (menu-filter m) utf8))
      (apply-filter!))
     (else #t)))
  #t)
