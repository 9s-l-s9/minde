;;; ipc-reply.scm -- self-describing IPC reply envelope.
;;;
;;; Loaded by init.scm (and by tests/ipc-reply-test.scm headlessly). Kept in a
;;; plain file rather than a public module so it can be exercised in isolation
;;; without pulling in the whole compositor policy layer, and without touching
;;; the frozen public (minde ...) module API.
;;;
;;; Reply shape (always a single readable Scheme datum):
;;;   (ok RESULT)
;;;   (error KEY ARGS MESSAGE-STRING BACKTRACE-STRING)
;;; MESSAGE-STRING is a human-readable rendering of the condition and
;;; BACKTRACE-STRING a bounded Guile backtrace, so an LLM or human can
;;; self-correct without a second round trip. RESULT is guaranteed to be
;;; `write`-able and re-`read`-able data: an unreadable value (a record,
;;; procedure or other object printing as #<...>) is reported as an
;;; `unreadable-result` error rather than emitted as an unparseable datum.

(define %ipc-backtrace-max-frames 8)   ; deepest frames to include
(define %ipc-backtrace-max-chars 2000) ; hard cap on the backtrace string

(define (ipc-truncate string limit)
  "Returns STRING bounded to LIMIT characters with an ellipsis marker."
  (if (> (string-length string) limit)
      (string-append (substring string 0 limit) "...[truncated]")
      string))

(define (ipc-format-message key arguments)
  "Best-effort human-readable rendering of an exception. Guarded: error
formatting must never itself throw."
  (catch #t
    (lambda ()
      ;; Standard Guile exceptions (including user `error`, `scm-error`) carry
      ;; (SUBR MESSAGE MESSAGE-ARGS . REST); render MESSAGE with its irritants.
      (if (and (pair? arguments)
               (pair? (cdr arguments))
               (string? (cadr arguments)))
          (let* ((subr (car arguments))
                 (message (cadr arguments))
                 (message-args (if (pair? (cddr arguments)) (caddr arguments) '()))
                 (rendered (apply format #f message
                                  (if (list? message-args) message-args '()))))
            (if subr
                (string-append (format #f "~a" subr) ": " rendered)
                rendered))
          (format #f "~a ~s" key arguments)))
    (lambda _ (format #f "~a ~s" key arguments))))

(define (ipc-format-backtrace stack)
  "Bounded backtrace string for STACK (or empty when unavailable). Guarded so
error formatting cannot itself throw."
  (catch #t
    (lambda ()
      (if stack
          (ipc-truncate
           (call-with-output-string
            (lambda (port)
              (display-backtrace stack port 1 %ipc-backtrace-max-frames)))
           %ipc-backtrace-max-chars)
          ""))
    (lambda _ "")))

(define (ipc-writable-datum value)
  "Returns VALUE when it is `write`-able re-readable data, otherwise its
bounded printed form as a string, so error ARGS (which may carry live
objects such as wrong-type-arg irritants) never make the error reply itself
unreadable."
  (let ((printed (catch #t
                   (lambda ()
                     (call-with-output-string (lambda (p) (write value p))))
                   (lambda _ #f))))
    (cond
     ((not printed) "<unwritable>")
     ((string-contains printed "#<") (ipc-truncate printed 200))
     (else value))))

(define (ipc-ok-reply result)
  "Returns (ok RESULT) when RESULT is `write`-able and re-`read`-able data,
otherwise a well-formed error datum. This is the writable-data guarantee: an
unreadable object (printing as #<...>) never leaks into the reply as an
unparseable datum."
  (let ((printed (catch #t
                   (lambda ()
                     (call-with-output-string (lambda (p) (write result p))))
                   (lambda _ #f))))
    (cond
     ((not printed)
      (list 'error 'unwritable-result '()
            "command result could not be serialized to a Scheme datum" ""))
     ((string-contains printed "#<")
      (list 'error 'unreadable-result '()
            (string-append
             "command result is not re-readable data (contains #<...>): "
             (ipc-truncate printed 200))
            ""))
     (else (list 'ok result)))))

(define (minde-ipc-eval source)
  "Evaluates the single datum in SOURCE and returns a one-datum reply string
following the shape documented above. Called by the Rust IPC source on the
event-loop thread."
  (call-with-output-string
    (lambda (port)
      (let ((stack #f))
        (catch #t
          (lambda ()
            (with-throw-handler #t
              (lambda ()
                (let ((datum
                       (call-with-input-string source
                         (lambda (input)
                           (let ((value (read input)))
                             (unless (eof-object? (read input))
                               (error "IPC accepts exactly one datum"))
                             value)))))
                  (write (ipc-ok-reply (eval datum (interaction-environment)))
                         port)))
              (lambda _ (set! stack (make-stack #t)))))
          (lambda (key . arguments)
            (let ((details (ipc-format-message key arguments)))
              (wm-log (string-append "IPC evaluation failure: " details))
              ;; ARGS may carry live objects (wrong-type-arg irritants and the
              ;; like); sanitize each element so the error datum itself honors
              ;; the readable-reply guarantee.
              (write (list 'error key
                           (if (list? arguments)
                               (map ipc-writable-datum arguments)
                               (ipc-writable-datum arguments))
                           details
                           (ipc-format-backtrace stack))
                     port))))))))
