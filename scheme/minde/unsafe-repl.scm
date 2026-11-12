;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Socket setup for the explicitly enabled, separate-thread development REPL.

(define-module (minde unsafe-repl)
  #:use-module (srfi srfi-13)
  #:export (make-private-repl-socket))

(define (make-private-repl-socket runtime-directory)
  "Bind a mode-0600 REPL socket in a private, current-user runtime directory.
Require an absolute XDG_RUNTIME_DIR; never fall back to shared /tmp or remove
an existing entry. A stale socket must be removed explicitly by its owner."
  (unless (and (string? runtime-directory)
               (string-prefix? "/" runtime-directory))
    (error "unsafe REPL requires an absolute XDG_RUNTIME_DIR"))
  ;; A trailing slash would make lstat follow a directory symlink.
  (let* ((directory (string-trim-right runtime-directory #\/))
         (metadata (lstat directory))
         (path (string-append directory "/minde-repl.sock")))
    (unless (and (eq? (stat:type metadata) 'directory)
                 (= (stat:uid metadata) (geteuid))
                 (zero? (logand (stat:perms metadata) #o077)))
      (error "unsafe REPL runtime directory must be private and owned by the session user"
             directory))
    (let ((sock (socket PF_UNIX SOCK_STREAM 0)))
      (catch #t
        (lambda ()
          ;; bind refuses files, symlinks and sockets already at PATH. The
          ;; private parent prevents cross-user access before chmod finishes.
          (bind sock AF_UNIX path)
          (chmod path #o600)
          sock)
        (lambda (key . args)
          (close-port sock)
          (apply throw key args))))))
