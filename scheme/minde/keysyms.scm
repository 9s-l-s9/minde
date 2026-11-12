;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; XKB keysym names shared by standalone checks and compositor policy.

(define-module (minde keysyms)
  #:use-module (system foreign)
  #:use-module (rnrs bytevectors)
  #:export (canonical-keysym-name))

;; Guix replaces this with its input's absolute library path before compiling
;; the installed modules. Other installations use the dynamic loader's path.
(define %xkbcommon-library "libxkbcommon")

(define (bind-xkb library)
  ;; Retain the library handle for as long as its procedures are reachable.
  (vector library
          (pointer->procedure uint32 (dynamic-func "xkb_keysym_from_name" library)
                              (list '* int))
          (pointer->procedure int (dynamic-func "xkb_keysym_get_name" library)
                              (list uint32 '* size_t))))

(define %xkb
  (delay
    (or ;; The compositor already links libxkbcommon.
        (false-if-exception (bind-xkb (dynamic-link)))
        (false-if-exception (bind-xkb (dynamic-link %xkbcommon-library)))
        ;; Development Guix shells expose their inputs through this profile.
        (let ((environment (getenv "GUIX_ENVIRONMENT")))
          (and environment
               (false-if-exception
                (bind-xkb (dynamic-link (string-append environment "/lib/libxkbcommon"))))))
        (error "key validation requires libxkbcommon; enter the project Guix shell"))))

(define (canonical-keysym-name name)
  "Return NAME as emitted by XKB, rejecting unknown or empty keysym names."
  (unless (and (string? name) (not (string-null? name))
               (not (string-index name #\nul)))
    (error "invalid XKB key name" name))
  (let* ((xkb (force %xkb))
         ;; Flags zero preserves case: space and Space are different names.
         (keysym ((vector-ref xkb 1) (string->pointer name "UTF-8") 0)))
    (when (zero? keysym) (error "unknown XKB key name" name))
    ;; Start with XKB's recommended buffer size; grow it if a future library
    ;; adds a longer name. Never read beyond the initialized buffer.
    (let loop ((size 64))
      (let* ((buffer (make-bytevector size 0))
             (pointer (bytevector->pointer buffer))
             (length ((vector-ref xkb 2) keysym pointer size)))
        (cond ((not (positive? length)) (error "cannot obtain canonical XKB key name" name))
              ((>= length size) (loop (+ length 1)))
              (else (pointer->string pointer length "UTF-8")))))))
