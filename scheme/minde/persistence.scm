;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Checked persistence with deferred startup diagnostics.

(define-module (minde persistence)
  #:use-module (srfi srfi-1)
  #:use-module (minde foundation serialization)
  #:use-module (minde compositor frames)
  #:use-module (minde compositor rust)
  #:export (load-persistent-state! save-persistent-state!
            show-persistence-errors!))

(define %failed-loads '())
(define %pending-errors '())
(define %persistent-state-version 1)

(define (state-file-exists? path)
  ;; Unlike file-exists?, only absence is quiet. Permission failures and
  ;; dangling symlinks must not turn an existing file into an empty registry.
  (catch 'system-error
    (lambda () (lstat path) #t)
    (lambda args
      (if (= ENOENT (system-error-errno args)) #f
          (apply throw 'system-error args)))))

(define (read-state path kind valid?)
  (let ((saved (read-versioned-datum-file path kind %persistent-state-version)))
    (unless (valid? saved) (error "invalid persistent state payload" path kind))
    saved))

(define (clear-state-error! path)
  (set! %failed-loads (delete path %failed-loads))
  (set! %pending-errors (alist-delete path %pending-errors)))

(define (report-state-error! action path kind key args)
  (let ((message (format #f "could not ~a ~a: ~a; file preserved. Repair and reload it, or move it aside before saving."
                         action kind path)))
    (rust-call-if-bound 'wm-log (format #f "~a (~a ~s)" message key args))
    (if (pair? (rust-call-if-bound 'wm-outputs))
        (echo message)
        (set! %pending-errors
              (acons path message (alist-delete path %pending-errors))))
    #f))

(define (show-persistence-errors!)
  "Shows pending persistence errors after an output becomes available."
  (unless (null? %pending-errors)
    (let ((messages (map cdr (reverse %pending-errors))))
      (set! %pending-errors '())
      (echo (string-join messages "\n")))))

(define (load-persistent-state! path kind valid? publish)
  "Validates the complete version-1 file at PATH before calling PUBLISH.
Missing files leave memory unchanged. Errors preserve the file and block
saving until a successful reload or removal; diagnostics are logged and shown."
  (catch #t
    (lambda ()
      (when (state-file-exists? path)
        (publish (read-state path kind valid?)))
      (clear-state-error! path)
      #t)
    (lambda (key . args)
      (set! %failed-loads (cons path (delete path %failed-loads)))
      (report-state-error! 'load path kind key args))))

(define (save-persistent-state! path kind valid? update)
  "Passes validated saved state (or an empty list) to UPDATE and atomically
writes its validated result. Returns that result, or #f on failure. An
unreadable file is never replaced, including when it changed after loading."
  (catch #t
    (lambda ()
      (let* ((exists? (state-file-exists? path))
             (saved
              (if exists?
                  (begin
                    (when (member path %failed-loads)
                      (error "reload the preserved file before saving" path))
                    ;; An external edit can invalidate a file after startup.
                    (catch #t
                      (lambda () (read-state path kind valid?))
                      (lambda args
                        (set! %failed-loads (cons path (delete path %failed-loads)))
                        (apply throw args))))
                  '()))
             (candidate (update saved)))
        (unless (valid? candidate) (error "invalid state to save" path kind))
        (let ((dir (dirname path)))
          (unless (file-exists? dir) (mkdir dir)))
        (write-versioned-datum-file path kind %persistent-state-version candidate)
        (clear-state-error! path)
        candidate))
    (lambda (key . args)
      (report-state-error! 'save path kind key args))))
