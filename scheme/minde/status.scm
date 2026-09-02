;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Stable, versioned status API for IPC clients and external bars.

(define-module (minde status)
  #:use-module (ice-9 ftw)
  #:use-module (ice-9 match)
  #:use-module (srfi srfi-1)
  #:use-module (minde compositor frames)
  #:use-module (minde compositor rust)
  #:use-module (minde groups)
  #:export (status-schema-version
            current-status-text
            current-state
            current-state-json
            status-file-path
            publish-status!))

(define status-schema-version 1)
(define %api-binding-documentation
  '((status-schema-version . "The integer schema version emitted by the structured status API.")))
(define %sequence 0)
(define %last-fingerprint #f)
;; State awaiting its file write (see publish-status!), or #f.
(define %pending-body #f)
(define %pending-runtime #f)
(define %write-scheduled? #f)

(define (runtime-directory)
  (let ((runtime (getenv "XDG_RUNTIME_DIR")))
    (if runtime
        (string-append runtime "/minde")
        (string-append "/tmp/minde-" (number->string (getuid))))))

(define (status-file-path)
  "Returns the path of the atomically published structured status file."
  (or (getenv "MINDE_STATUS_PATH")
      (string-append (runtime-directory) "/status.json")))

(define (legacy-status-file-path)
  (string-append (or (getenv "XDG_RUNTIME_DIR") "/tmp")
                 "/minde-status"))

(define (current-time-milliseconds)
  (let ((now (gettimeofday)))
    (+ (* (car now) 1000) (quotient (cdr now) 1000))))

(define (call-runtime-info)
  (or (rust-call-if-bound 'wm-runtime-info)
      '("unknown" "unknown" -1 0)))

(define (output-state)
  (let ((outputs (or (rust-call-if-bound 'wm-outputs) '())))
    (list->vector
     (map (lambda (output)
            `((id . ,(list-ref output 0))
              (x . ,(list-ref output 1))
              (y . ,(list-ref output 2))
              (width . ,(list-ref output 3))
              (height . ,(list-ref output 4))
              (name . ,(if (> (length output) 5)
                           (list-ref output 5)
                           "unknown"))))
          outputs))))

(define (group-state)
  (list->vector
   (map (match-lambda
          ((name focused? count floating? dynamic?)
           `((name . ,name)
             (focused . ,focused?)
             (window_count . ,count)
             (floating . ,floating?)
             (dynamic . ,dynamic?))))
        (group-status-summaries))))

(define (layout-state)
  (match (current-layout-status)
    (('dynamic position ratio)
     `((kind . "dynamic")
       (head_mode . ,(symbol->string (head-mode)))
       (head_id . ,(current-head-id))
       (master_position . ,(symbol->string position))
       (master_ratio . ,(format #f "~a" ratio))))
    (('manual spec)
     `((kind . "manual")
       (head_mode . ,(symbol->string (head-mode)))
       (head_id . ,(current-head-id))
       (spec . ,(format #f "~s" spec))))))

(define (state-body redact?)
  ;; The window-management half of the state: everything after the
  ;; header, so a publication can compare it against the previous one and
  ;; reuse it verbatim when it must be written.
  (let* ((focused (focused-window-id))
         (focused-state
          (and focused
               `((id . ,focused)
                 ,@(if redact?
                       '()
                       `((title . ,(window-title focused))
                         (app_id . ,(or (window-app-id focused) ""))))
                 (urgent . ,(and (member focused (urgent-windows)) #t))))))
    `((groups . ,(group-state))
      (focused_group . ,(current-group-name))
      (focused_window . ,(or focused-state 'null))
      (urgent_windows . ,(list->vector (urgent-windows)))
      (outputs . ,(output-state))
      (layout . ,(layout-state)))))

(define (state-with-header body sequence generated-at-ms runtime)
  `((schema_version . ,status-schema-version)
    (sequence . ,sequence)
    (generated_at_ms . ,generated-at-ms)
    (runtime . ((backend . ,(list-ref runtime 0))
                (xwayland . ,(list-ref runtime 1))
                (xdisplay . ,(let ((display (list-ref runtime 2)))
                               (if (negative? display) 'null display)))
                (uptime_ms . ,(list-ref runtime 3))))
    ,@body))

(define* (current-state #:key (redact? #f) (sequence %sequence)
                        (generated-at-ms (current-time-milliseconds))
                        (runtime-info (call-runtime-info)))
  "Returns the schema-versioned compositor state as an alist.
When REDACT? is true, window titles and application identifiers are omitted."
  (state-with-header (state-body redact?) sequence generated-at-ms
                     runtime-info))

(define (write-json-string value port)
  (display #\" port)
  (string-for-each
   (lambda (character)
     (case character
       ((#\") (display "\\\"" port))
       ((#\\) (display "\\\\" port))
       ((#\newline) (display "\\n" port))
       ((#\return) (display "\\r" port))
       ((#\tab) (display "\\t" port))
       (else
        (if (< (char->integer character) 32)
            (let* ((hex "0123456789abcdef")
                   (code (char->integer character)))
              (display "\\u00" port)
              (display (string-ref hex (quotient code 16)) port)
              (display (string-ref hex (modulo code 16)) port))
            (display character port)))))
   value)
  (display #\" port))

(define (json-string value)
  (call-with-output-string
   (lambda (port) (write-json-string value port))))

(define (json-object? value)
  (and (list? value)
       (every (lambda (entry)
                (and (pair? entry)
                     (or (symbol? (car entry)) (string? (car entry)))))
              value)))

(define (write-json value port)
  ;; Compact JSON straight onto PORT: no intermediate strings per node.
  (cond
   ((eq? value 'null) (display "null" port))
   ((boolean? value) (display (if value "true" "false") port))
   ((number? value) (display (number->string value) port))
   ((string? value) (write-json-string value port))
   ((symbol? value) (write-json-string (symbol->string value) port))
   ((vector? value)
    (display #\[ port)
    (let ((count (vector-length value)))
      (do ((index 0 (+ index 1)))
          ((= index count))
        (unless (zero? index) (display #\, port))
        (write-json (vector-ref value index) port)))
    (display #\] port))
   ((json-object? value)
    (display #\{ port)
    (let loop ((entries value) (first? #t))
      (unless (null? entries)
        (let ((entry (car entries)))
          (unless first? (display #\, port))
          (write-json-string (if (symbol? (car entry))
                                 (symbol->string (car entry))
                                 (car entry))
                             port)
          (display #\: port)
          (write-json (cdr entry) port)
          (loop (cdr entries) #f))))
    (display #\} port))
   (else (error "value cannot be represented in status JSON" value))))

(define (json-value value)
  (call-with-output-string (lambda (port) (write-json value port))))

(define* (current-state-json #:key (redact? #f))
  "Returns the current compositor state as one compact schema-v1 JSON object."
  (json-value (current-state #:redact? redact?)))

(define (atomic-write-file path write-contents)
  (let* ((directory (dirname path))
         (temporary (string-append path ".tmp." (number->string (getpid)))))
    (unless (file-exists? directory)
      (mkdir directory #o700))
    (call-with-output-file temporary
      (lambda (port)
        (write-contents port)
        (newline port)))
    (chmod temporary #o600)
    (rename-file temporary path)))

(define (current-status-text)
  "Returns a one-line summary suitable for an external bar such as eww."
  (status-line))

(define (write-status-files!)
  ;; Writes the most recently published state. Runs from a 0 ms timer when
  ;; the compositor offers one, so a burst of publications (a group switch
  ;; syncs several times) costs one pair of file writes.
  (set! %write-scheduled? #f)
  (when %pending-body
    (let ((state (state-with-header %pending-body %sequence
                                    (current-time-milliseconds)
                                    %pending-runtime)))
      (set! %pending-body #f)
      (set! %pending-runtime #f)
      (catch #t
        (lambda ()
          (atomic-write-file (status-file-path)
                             (lambda (port) (write-json state port)))
          (atomic-write-file (legacy-status-file-path)
                             (lambda (port)
                               (display (current-status-text) port))))
        (lambda (key . arguments)
          (rust-call-if-bound
           'wm-log
           (format #f "status publication failed: ~a ~s" key arguments)))))))

(define (schedule-status-write!)
  ;; wm-run-after is init.scm's timer wrapper; without it (unit tests, a
  ;; headless evaluation) the files are written immediately.
  (unless %write-scheduled?
    (set! %write-scheduled? #t)
    (unless (eq? #t (rust-call-if-bound 'wm-run-after 0 write-status-files!))
      (write-status-files!))))

(define (publish-status!)
  "Atomically publishes status.json and the compatibility text status file.
Unchanged compositor state is not rewritten; the sequence number advances
per change even when several changes coalesce into one write. Errors are
logged and swallowed so a missing runtime directory cannot interrupt window
management."
  (let* ((runtime (call-runtime-info))
         (body (state-body #f))
         ;; Uptime is observational and changes continuously; it must not turn
         ;; every policy sync into a false status change.
         (fingerprint (cons (list (list-ref runtime 0) (list-ref runtime 1)
                                  (list-ref runtime 2))
                            body)))
    (unless (equal? fingerprint %last-fingerprint)
      (set! %last-fingerprint fingerprint)
      (set! %sequence (+ %sequence 1))
      (set! %pending-body body)
      (set! %pending-runtime runtime)
      (schedule-status-write!)))
  %sequence)
