;;; layouts.scm -- named layout presets and save/restore, StumpWM's
;;; dump-desktop/restore-from-file in miniature.
;;;
;;; A layout is a pure-sexp spec (see dump-layout-spec in frames.scm):
;;;   'leaf | (hsplit RATIO SPEC SPEC) | (vsplit RATIO SPEC SPEC)
;;; The registry maps names to specs; it persists as a plain alist
;;; written/read to ~/.config/minde/layouts.scm (override with the
;;; MINDE_LAYOUTS_FILE environment variable, used by the tests).
;;;
;;; Same load-time constraint as frames.scm: nothing here calls a wm-*
;;; Rust subr at module load time.

(define-module (minde layouts)
  #:use-module (srfi srfi-1)
  #:use-module (minde compositor frames)
  #:use-module (minde compositor revision)
  #:use-module (minde persistence)
  #:export (define-layout!
            layout-names
            layout-spec
            apply-layout!
            save-layout!
            load-layouts!))

;; name (string) -> spec, insertion-ordered for stable completion lists.
(define %layouts '())
(define %defined-layout-names '())

(define (register-layout! name spec)
  (set! %layouts
        (append (alist-delete name %layouts) (list (cons name spec))))
  (note-policy-change!))

(define (define-layout! name spec)
  "Registers configuration layout NAME as SPEC, taking precedence over
same-named saved layouts on load. Does not persist; see save-layout!."
  (set! %defined-layout-names (cons name (delete name %defined-layout-names)))
  (register-layout! name spec))

(define (layout-names)
  "Returns registered layout names in stable insertion order."
  (map car %layouts))

(define (layout-spec name)
  "Returns the frame-tree specification registered as NAME, or #f."
  (assoc-ref %layouts name))

(define (apply-layout! name)
  "Rebuilds the current head's frame tree from the layout named NAME,
redistributing its tiled windows (see apply-layout-spec!). Floats and
other heads are unchanged. Echoes an error for an unknown name."
  (let ((spec (layout-spec name)))
    (if spec
        (begin
          (apply-layout-spec! spec)
          (echo (string-append "layout: " name)))
        (echo (string-append "no layout: " name)))))

;; ---------------------------------------------------------------------
;; Persistence
;; ---------------------------------------------------------------------

(define (layouts-file)
  (or (getenv "MINDE_LAYOUTS_FILE")
      (string-append (or (getenv "HOME") ".") "/.config/minde/layouts.scm")))

(define (save-layout! name)
  "Saves the live frame tree as NAME, preserving previously saved layouts.
Other configuration presets are not copied into the file. A name that the
configuration defines is refused: its definition replaces any saved layout
of that name on every load. Returns #f on refusal or failure without
changing the registry or overwriting an unreadable file."
  (if (member name %defined-layout-names)
      (begin
        (echo (string-append "layout " name
                             " is defined by the configuration; choose another name"))
        #f)
      (let* ((spec (dump-layout-spec))
             (saved (save-persistent-state!
                     (layouts-file) 'minde-layouts valid-layouts?
                     (lambda (previous)
                       (append (alist-delete name previous)
                               (list (cons name spec)))))))
        (and saved
             (begin
               (register-layout! name spec)
               (echo (string-append "saved layout: " name))
               #t)))))

(define (valid-layouts? saved)
  (and (list? saved)
       (every (lambda (entry)
                (and (pair? entry) (string? (car entry))
                     (valid-layout-spec? (cdr entry))))
              saved)))

(define (load-layouts!)
  "Loads validated saved layouts; configuration-defined names take precedence.
Missing files are ignored. Invalid files leave the registry unchanged and
block saves until a successful reload or removal. Returns #t on success."
  (load-persistent-state!
   (layouts-file) 'minde-layouts valid-layouts?
   (lambda (saved)
     (for-each
      (lambda (entry)
        (unless (member (car entry) %defined-layout-names)
          (register-layout! (car entry) (cdr entry))))
      saved))))
