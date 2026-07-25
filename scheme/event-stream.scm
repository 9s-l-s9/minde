;;; event-stream.scm -- serialize fired event hooks onto the push socket.
;;;
;;; Loaded by init.scm (and by tests/event-stream-test.scm headlessly). Kept in
;;; a plain file rather than a public module so it can be exercised in isolation
;;; without the compositor, and without touching the frozen public (minde ...)
;;; module API.
;;;
;;; `run-event-hook!' (in (minde hooks)) resolves and calls the top-level
;;; `minde-mirror-event' defined here on every firing, so ALL events reach
;;; subscribers with no user-installed hook. The event is serialized to one
;;; readable s-expression line -- the event name followed by its hook payload,
;;; matching the shapes in %api-hook-metadata, e.g.
;;;   (new-window 42 "firefox" "org.mozilla.firefox")
;;; -- and handed to the wm-publish-event gsubr, which fans it out. Rust appends
;;; the terminating newline.
;;;
;;; Writable-data guarantee: each payload value is passed through
;;; `ipc-writable-datum' (from ipc-reply.scm), so a live #<...> irritant that
;;; slipped into a payload is bounded to a string rather than emitted as an
;;; unparseable line, exactly as the eval reply path guarantees.
;;;
;;; Lock privacy: while the session is locked, title- and content-bearing
;;; events are filtered, mirroring (minde status)'s redact? policy (window
;;; id retained; human-readable title and app-id omitted). new-window keeps its
;;; id but blanks title and app-id; message (arbitrary on-screen text) is
;;; suppressed entirely. Id-only lifecycle and geometry events keep flowing so
;;; an agent can still track focus and window churn while locked.

(define (event-redact-args name args)
  "Returns ARGS filtered for the locked session, or #f to suppress the event.
Keeps window ids and geometry; blanks or drops human-readable strings."
  (case name
    ((new-window)
     ;; (id title app-id) -> keep id, blank the title and app-id strings.
     (if (and (pair? args) (pair? (cdr args)) (pair? (cddr args)))
         (list (car args) "" "")
         args))
    ((message) #f)                      ; arbitrary on-screen text: suppress
    (else args)))

(define (event->line name args locked?)
  "Returns the readable one-line s-expression for event NAME with payload ARGS,
or #f when the event is suppressed under LOCKED?. Each payload value honors the
writable-data guarantee via ipc-writable-datum."
  (let ((payload (if locked? (event-redact-args name args) args)))
    (and payload
         (call-with-output-string
           (lambda (port)
             (write (cons name (map ipc-writable-datum payload)) port))))))

(define (minde-mirror-event name args)
  "Serializes event NAME with payload list ARGS and mirrors it to every
event-socket subscriber via the wm-publish-event gsubr. Called by
run-event-hook! for every firing. A missing gsubr (headless load) or a
lock-suppressed event is a no-op."
  (let* ((locked? (and (defined? 'wm-session-locked?) (wm-session-locked?)))
         (line (event->line name args locked?)))
    (when (and line (defined? 'wm-publish-event))
      (wm-publish-event line))))
