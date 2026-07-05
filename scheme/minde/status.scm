;;; SPDX-License-Identifier: GPL-3.0-or-later
;;; Stable, versioned status API for IPC clients and external bars.

(define-module (minde status)
  #:use-module (ice-9 ftw)
  #:use-module (ice-9 match)
  #:use-module (srfi srfi-1)
  #:use-module (minde frames)
  #:use-module (minde groups)
  #:export (status-schema-version
            current-status-text
            current-state
            current-state-json
            status-file-path
            publish-status!))

(define status-schema-version 1)
(define %sequence 0)
(define %last-fingerprint #f)

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
  (let* ((module (resolve-module '(guile-user) #:ensure #f))
         (variable (and module (module-variable module 'wm-runtime-info))))
    (if variable
        ((variable-ref variable))
        '("unknown" "unknown" -1 0))))

(define (output-state)
  (let* ((module (resolve-module '(guile-user) #:ensure #f))
         (variable (and module (module-variable module 'wm-outputs)))
         (outputs (if variable ((variable-ref variable)) '())))
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

(define* (current-state #:key (redact? #f) (sequence %sequence)
                        (generated-at-ms (current-time-milliseconds))
                        (runtime-info (call-runtime-info)))
  "Returns the schema-versioned compositor state as an alist.
When REDACT? is true, window titles and application identifiers are omitted."
  (let* ((focused (focused-window-id))
         (runtime runtime-info)
         (focused-state
          (and focused
               `((id . ,focused)
                 ,@(if redact?
                       '()
                       `((title . ,(window-title focused))
                         (app_id . ,(or (window-app-id focused) ""))))
                 (urgent . ,(and (member focused (urgent-windows)) #t))))))
    `((schema_version . ,status-schema-version)
      (sequence . ,sequence)
      (generated_at_ms . ,generated-at-ms)
      (runtime . ((backend . ,(list-ref runtime 0))
                  (xwayland . ,(list-ref runtime 1))
                  (xdisplay . ,(let ((display (list-ref runtime 2)))
                                 (if (negative? display) 'null display)))
                  (uptime_ms . ,(list-ref runtime 3))))
      (groups . ,(group-state))
      (focused_group . ,(current-group-name))
      (focused_window . ,(or focused-state 'null))
      (urgent_windows . ,(list->vector (urgent-windows)))
      (outputs . ,(output-state))
      (layout . ,(layout-state)))))

(define (json-string value)
  (call-with-output-string
   (lambda (port)
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
     (display #\" port))))

(define (json-object? value)
  (and (list? value)
       (every (lambda (entry)
                (and (pair? entry)
                     (or (symbol? (car entry)) (string? (car entry)))))
              value)))

(define (json-value value)
  (cond
   ((eq? value 'null) "null")
   ((boolean? value) (if value "true" "false"))
   ((number? value) (number->string value))
   ((string? value) (json-string value))
   ((symbol? value) (json-string (symbol->string value)))
   ((vector? value)
    (string-append "["
                   (string-join (map json-value (vector->list value)) ",")
                   "]"))
   ((json-object? value)
    (string-append
     "{"
     (string-join
      (map (lambda (entry)
             (string-append
              (json-string (if (symbol? (car entry))
                               (symbol->string (car entry))
                               (car entry)))
              ":" (json-value (cdr entry))))
           value)
      ",")
     "}"))
   (else (error "value cannot be represented in status JSON" value))))

(define* (current-state-json #:key (redact? #f))
  "Returns the current compositor state as one compact schema-v1 JSON object."
  (json-value (current-state #:redact? redact?)))

(define (atomic-write-file path contents)
  (let* ((directory (dirname path))
         (temporary (string-append path ".tmp." (number->string (getpid)))))
    (unless (file-exists? directory)
      (mkdir directory #o700))
    (call-with-output-file temporary
      (lambda (port)
        (display contents port)
        (newline port)))
    (chmod temporary #o600)
    (rename-file temporary path)))

(define (current-status-text)
  "Returns a one-line summary suitable for an external bar such as eww."
  (status-line))

(define (publish-status!)
  "Atomically publishes status.json and the compatibility text status file.
Unchanged compositor state is not rewritten. Errors are logged and swallowed
so a missing runtime directory cannot interrupt window management."
  (let* ((runtime (call-runtime-info))
         ;; Uptime is observational and changes continuously; it must not turn
         ;; every policy sync into a false status change.
         (stable-runtime (list (list-ref runtime 0) (list-ref runtime 1)
                               (list-ref runtime 2) 0))
         (fingerprint (current-state #:sequence 0 #:generated-at-ms 0
                                     #:runtime-info stable-runtime)))
    (unless (equal? fingerprint %last-fingerprint)
      (set! %last-fingerprint fingerprint)
      (set! %sequence (+ %sequence 1))
      (catch #t
        (lambda ()
          (atomic-write-file (status-file-path)
                             (json-value (current-state #:sequence %sequence)))
          (atomic-write-file (legacy-status-file-path) (current-status-text)))
        (lambda (key . arguments)
          (let* ((module (resolve-module '(guile-user) #:ensure #f))
                 (variable (and module (module-variable module 'wm-log))))
            (when variable
              ((variable-ref variable)
               (format #f "status publication failed: ~a ~s" key arguments))))))))
  %sequence)
