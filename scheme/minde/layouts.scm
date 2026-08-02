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
  #:use-module (minde foundation serialization)
  #:export (define-layout!
            layout-names
            layout-spec
            apply-layout!
            save-layout!
            load-layouts!))

;; name (string) -> spec, insertion-ordered for stable completion lists.
(define %layouts '())

(define (define-layout! name spec)
  "Registers (or replaces) the layout NAME as SPEC. Does not persist --
see save-layout!."
  (set! %layouts
        (append (alist-delete name %layouts) (list (cons name spec)))))

(define (layout-names)
  "Returns registered layout names in stable insertion order."
  (map car %layouts))

(define (layout-spec name)
  "Returns the frame-tree specification registered as NAME, or #f."
  (assoc-ref %layouts name))

(define (apply-layout! name)
  "Rebuilds the active group's frame tree from the layout named NAME,
redistributing its windows (see apply-layout-spec!). Echoes an error for
an unknown name."
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
  "Snapshots the live frame tree as layout NAME and writes the whole
registry to the layouts file."
  (define-layout! name (dump-layout-spec))
  (let ((path (layouts-file)))
    (catch #t
      (lambda ()
        (let ((dir (dirname path)))
          (unless (file-exists? dir) (mkdir dir)))
        (write-versioned-datum-file path 'minde-layouts 1 %layouts)
        (echo (string-append "saved layout: " name)))
      (lambda (key . args)
        (echo (format #f "could not save layouts: ~a ~s" key args))))))

(define (load-layouts!)
  "Merges layouts from the layouts file into the registry (file entries
win over same-named existing ones). Quietly does nothing if the file is
missing or unreadable."
  (let ((path (layouts-file)))
    (when (file-exists? path)
      (catch #t
        (lambda ()
          (let ((saved (read-versioned-datum-file
                        path 'minde-layouts 1)))
            (when (list? saved)
              (for-each
               (lambda (entry)
                 (when (and (pair? entry) (string? (car entry)))
                   (define-layout! (car entry) (cdr entry))))
               saved))))
        (lambda (key . args)
          (echo (format #f "could not load layouts: ~a ~s" key args)))))))
