;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Event push serialization, sanitization and lock-privacy filtering
;;; (scheme/event-stream.scm).

(use-modules (ice-9 rdelim))

;; ipc-writable-datum lives in ipc-reply.scm; event-stream.scm reuses it for
;; the writable-data guarantee. wm-log is a Rust gsubr at runtime; stub it so
;; the envelope loads headlessly.
(define %log '())
(define (wm-log message) (set! %log (cons message %log)) #t)
(load-from-path "ipc-reply.scm")
(load-from-path "event-stream.scm")

(define failures 0)
(define (check description value)
  (unless value
    (set! failures (+ failures 1))
    (format #t "FAIL - ~a~%" description)))

;; A serialized line must be exactly one datum that reads back cleanly.
(define (parse-line line)
  (call-with-input-string line
    (lambda (port)
      (let ((datum (read port)))
        (check "line is a single datum" (eof-object? (read port)))
        datum))))

;; --- Unlocked serialization matches the documented payload shapes ----------

(let ((datum (parse-line
              (event->line 'new-window '(42 "firefox" "org.mozilla.firefox") #f))))
  (check "new-window is tagged with the event name" (eq? (car datum) 'new-window))
  (check "new-window carries its full payload"
         (equal? datum '(new-window 42 "firefox" "org.mozilla.firefox"))))

(let ((datum (parse-line (event->line 'focus-window '(7) #f))))
  (check "focus-window round-trips" (equal? datum '(focus-window 7))))

(let ((datum (parse-line (event->line 'session-lock '() #f))))
  (check "payload-free event serializes as a bare name list"
         (equal? datum '(session-lock))))

(let ((datum (parse-line (event->line 'focus-frame '(0 0 800 600) #f))))
  (check "geometry event round-trips" (equal? datum '(focus-frame 0 0 800 600))))

;; --- Writable-data guarantee: a #<...> irritant is bounded, never leaked ----

(let* ((line (event->line 'message (list (current-output-port)) #f))
       (datum (parse-line line)))
  (check "unwritable payload value is bounded to a string"
         (and (eq? (car datum) 'message) (string? (cadr datum)))))

;; --- Lock privacy mirrors (minde status)'s redact? policy ---------------

(let ((datum (parse-line
              (event->line 'new-window '(42 "Secret Doc" "org.example") #t))))
  (check "locked new-window keeps its id" (= (cadr datum) 42))
  (check "locked new-window blanks the title" (string=? (caddr datum) ""))
  (check "locked new-window blanks the app-id" (string=? (cadddr datum) "")))

(check "locked message events are suppressed entirely"
       (not (event->line 'message '("balance is 1234") #t)))

(check "locked id-only lifecycle events keep flowing"
       (equal? (parse-line (event->line 'destroy-window '(9) #t))
               '(destroy-window 9)))

(check "locked geometry events keep flowing"
       (equal? (parse-line (event->line 'focus-frame '(1 2 3 4) #t))
               '(focus-frame 1 2 3 4)))

;; --- minde-mirror-event drives wm-publish-event with the finished line --

(let ((published '()))
  ;; Stand in for the Rust gsubrs the running compositor provides.
  (define (wm-session-locked?) #f)
  (define (wm-publish-event line) (set! published (cons line published)) #t)
  (module-define! (current-module) 'wm-session-locked? wm-session-locked?)
  (module-define! (current-module) 'wm-publish-event wm-publish-event)
  (minde-mirror-event 'new-window '(1 "t" "a"))
  (check "mirror published exactly one line" (= (length published) 1))
  (check "mirrored line parses to the event datum"
         (equal? (parse-line (car published)) '(new-window 1 "t" "a"))))

(if (zero? failures)
    (format #t "event-stream-test: all checks passed~%")
    (begin
      (format #t "event-stream-test: ~a failure(s)~%" failures)
      (exit 1)))
