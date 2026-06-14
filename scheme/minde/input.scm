;;; input.scm -- native input prompt, StumpWM's read-one-line.
;;;
;;; The compositor itself prompts for a line of input in the message
;;; overlay: no external launcher involved. init.scm delegates every
;;; keypress here while a prompt is active. Editing keys mirror StumpWM's
;;; *input-map* (input.lisp).
;;;
;;; Same load-time constraint as frames.scm: nothing here calls a wm-*
;;; Rust subr at module load time.

(define-module (minde input)
  #:use-module (srfi srfi-1)
  #:use-module (srfi srfi-9)
  #:export (read-one-line
            input-active?
            input-handle-key!
            input-abort!))

;; ---------------------------------------------------------------------
;; Rust subrs, looked up dynamically (see frames.scm for why)
;; ---------------------------------------------------------------------

(define (rust-call name . args)
  (let* ((mod (resolve-module '(guile-user) #:ensure #f))
         (var (and mod (module-variable mod name))))
    (if var (apply (variable-ref var) args) #f)))

(define (show! text) (rust-call 'wm-message text 0)) ; sticky
(define (clear!) (rust-call 'wm-clear-message))

;; ---------------------------------------------------------------------
;; State
;; ---------------------------------------------------------------------

(define-record-type <input-state>
  (make-input-state prompt buffer cursor on-submit on-abort completions
                    comp-matches comp-index history-key history-pos saved)
  input-state?
  (prompt in-prompt)
  (buffer in-buffer set-in-buffer!)
  (cursor in-cursor set-in-cursor!)
  (on-submit in-on-submit)
  (on-abort in-on-abort)
  (completions in-completions)
  (comp-matches in-comp-matches set-in-comp-matches!)
  (comp-index in-comp-index set-in-comp-index!)
  (history-key in-history-key)
  (history-pos in-history-pos set-in-history-pos!)
  (saved in-saved set-in-saved!))

(define %current #f)

;; Per-prompt histories, most recent first (StumpWM's *input-history*,
;; in-memory only).
(define %histories (make-hash-table))

(define (input-active?) (and %current #t))

(define* (read-one-line prompt on-submit
                        #:key (completions '()) (initial "") (history 'default)
                        (on-abort #f))
  "Prompts for a line of input in the message overlay. ON-SUBMIT is
called with the entered string on RET; ON-ABORT (if given) on C-g/ESC.
COMPLETIONS is a list of strings or a thunk returning one (TAB cycles
prefix matches). HISTORY names the history list (C-p/C-n)."
  (set! %current
        (make-input-state prompt initial (string-length initial)
                          on-submit on-abort completions
                          #f 0 history -1 ""))
  (redraw!))

;; ---------------------------------------------------------------------
;; Rendering
;; ---------------------------------------------------------------------

(define (redraw!)
  (let* ((i %current)
         (buf (in-buffer i))
         (pos (in-cursor i)))
    (show! (string-append (in-prompt i)
                          (substring buf 0 pos) "|" (substring buf pos)))))

;; ---------------------------------------------------------------------
;; Buffer editing primitives
;; ---------------------------------------------------------------------

(define (insert! s)
  (let* ((i %current) (buf (in-buffer i)) (pos (in-cursor i)))
    (set-in-buffer! i (string-append (substring buf 0 pos) s (substring buf pos)))
    (set-in-cursor! i (+ pos (string-length s)))))

(define (delete-backward!)
  (let* ((i %current) (buf (in-buffer i)) (pos (in-cursor i)))
    (when (> pos 0)
      (set-in-buffer! i (string-append (substring buf 0 (- pos 1)) (substring buf pos)))
      (set-in-cursor! i (- pos 1)))))

(define (delete-forward!)
  (let* ((i %current) (buf (in-buffer i)) (pos (in-cursor i)))
    (when (< pos (string-length buf))
      (set-in-buffer! i (string-append (substring buf 0 pos) (substring buf (+ pos 1)))))))

(define (move! delta)
  (let ((i %current))
    (set-in-cursor! i (max 0 (min (string-length (in-buffer i))
                                  (+ (in-cursor i) delta))))))

(define (word-char? c) (or (char-alphabetic? c) (char-numeric? c)))

(define (word-boundary buf pos dir)
  "Position after moving one word in DIR (+1/-1) from POS."
  (let ((len (string-length buf)))
    (if (> dir 0)
        (let skip ((p pos))
          (cond ((>= p len) len)
                ((not (word-char? (string-ref buf p))) (skip (+ p 1)))
                (else (let eat ((p p))
                        (if (and (< p len) (word-char? (string-ref buf p)))
                            (eat (+ p 1)) p)))))
        (let skip ((p pos))
          (cond ((<= p 0) 0)
                ((not (word-char? (string-ref buf (- p 1)))) (skip (- p 1)))
                (else (let eat ((p p))
                        (if (and (> p 0) (word-char? (string-ref buf (- p 1))))
                            (eat (- p 1)) p))))))))

(define (kill-region! start end)
  (let* ((i %current) (buf (in-buffer i)))
    (set-in-buffer! i (string-append (substring buf 0 start) (substring buf end)))
    (set-in-cursor! i start)))

;; ---------------------------------------------------------------------
;; Completion (prefix match, TAB cycles -- StumpWM input-refine-prefix)
;; ---------------------------------------------------------------------

(define (completion-candidates)
  (let ((c (in-completions %current)))
    (if (procedure? c) (c) c)))

(define (complete! dir)
  (let ((i %current))
    (unless (in-comp-matches i)
      (let ((prefix (in-buffer i)))
        (set-in-comp-matches!
         i (filter (lambda (s) (string-prefix? prefix s)) (completion-candidates)))
        (set-in-comp-index! i (if (> dir 0) -1 0))))
    (let ((matches (in-comp-matches i)))
      (when (pair? matches)
        (set-in-comp-index! i (modulo (+ (in-comp-index i) dir) (length matches)))
        (set-in-buffer! i (list-ref matches (in-comp-index i)))
        (set-in-cursor! i (string-length (in-buffer i)))))))

(define (reset-completion!)
  (set-in-comp-matches! %current #f))

;; ---------------------------------------------------------------------
;; History
;; ---------------------------------------------------------------------

(define (history-ref key) (or (hash-ref %histories key) '()))

(define (history-push! key line)
  (unless (string-null? line)
    (let ((h (history-ref key)))
      (unless (and (pair? h) (string=? (car h) line))
        (hash-set! %histories key (cons line h))))))

(define (history-move! dir)
  (let* ((i %current)
         (h (history-ref (in-history-key i)))
         (pos (+ (in-history-pos i) dir)))
    (when (= (in-history-pos i) -1)
      (set-in-saved! i (in-buffer i)))
    (cond
     ((< pos -1) #f)
     ((= pos -1)
      (set-in-history-pos! i -1)
      (set-in-buffer! i (in-saved i))
      (set-in-cursor! i (string-length (in-buffer i))))
     ((< pos (length h))
      (set-in-history-pos! i pos)
      (set-in-buffer! i (list-ref h pos))
      (set-in-cursor! i (string-length (in-buffer i)))))))

;; ---------------------------------------------------------------------
;; Finishing
;; ---------------------------------------------------------------------

(define (submit!)
  (let* ((i %current)
         (line (in-buffer i))
         (cb (in-on-submit i)))
    (history-push! (in-history-key i) line)
    (set! %current #f)
    (clear!)
    (cb line)))

(define (input-abort!)
  (let ((cb (and %current (in-on-abort %current))))
    (set! %current #f)
    (clear!)
    (when cb (cb))))

;; ---------------------------------------------------------------------
;; Key dispatch (StumpWM *input-map* subset)
;; ---------------------------------------------------------------------

;; Modifier bit values mirror init.scm: shift=1 ctrl=4 alt=8 super=64.
(define (ctrl? mods) (logtest mods 4))
(define (alt? mods) (logtest mods 8))
(define (super? mods) (logtest mods 64))

(define (input-handle-key! mods keysym-name utf8)
  "Handles one keypress while a prompt is active. Always consumes (#t)."
  (let* ((i %current)
         (buf (in-buffer i))
         (pos (in-cursor i))
         (key (if (and (ctrl? mods) (= 1 (string-length keysym-name)))
                  (string-append "C-" keysym-name)
                  keysym-name))
         (meta-key (if (and (alt? mods) (= 1 (string-length keysym-name)))
                       (string-append "M-" keysym-name)
                       #f)))
    ;; Any key other than TAB restarts completion matching.
    (unless (member keysym-name '("Tab" "ISO_Left_Tab"))
      (reset-completion!))
    (cond
     ((member keysym-name '("Return" "KP_Enter")) (submit!))
     ((or (string=? keysym-name "Escape") (equal? key "C-g")) (input-abort!))
     ((string=? keysym-name "BackSpace")
      (if (or (ctrl? mods) (alt? mods))
          (kill-region! (word-boundary buf pos -1) pos)
          (delete-backward!))
      (redraw!))
     ((or (string=? keysym-name "Delete") (equal? key "C-d")) (delete-forward!) (redraw!))
     ((equal? meta-key "M-d") (kill-region! pos (word-boundary buf pos 1)) (redraw!))
     ((or (string=? keysym-name "Left") (equal? key "C-b")) (move! -1) (redraw!))
     ((or (string=? keysym-name "Right") (equal? key "C-f")) (move! 1) (redraw!))
     ((equal? meta-key "M-b")
      (set-in-cursor! i (word-boundary buf pos -1)) (redraw!))
     ((equal? meta-key "M-f")
      (set-in-cursor! i (word-boundary buf pos 1)) (redraw!))
     ((or (string=? keysym-name "Home") (equal? key "C-a"))
      (set-in-cursor! i 0) (redraw!))
     ((or (string=? keysym-name "End") (equal? key "C-e"))
      (set-in-cursor! i (string-length buf)) (redraw!))
     ((equal? key "C-k") (kill-region! pos (string-length buf)) (redraw!))
     ((equal? key "C-u") (kill-region! 0 pos) (redraw!))
     ((or (string=? keysym-name "Up") (equal? key "C-p")) (history-move! 1) (redraw!))
     ((or (string=? keysym-name "Down") (equal? key "C-n")) (history-move! -1) (redraw!))
     ((string=? keysym-name "Tab") (complete! 1) (redraw!))
     ((string=? keysym-name "ISO_Left_Tab") (complete! -1) (redraw!))
     ;; Self-insert: whatever text the keymap says this key produces.
     ((and (not (string-null? utf8)) (not (ctrl? mods)) (not (super? mods)))
      (insert! utf8) (redraw!))
     (else #f)) ; unknown chord: swallow silently (modifiers, F-keys, ...)
    #t))
